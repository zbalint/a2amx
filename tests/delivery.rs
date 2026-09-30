#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use a2amx::client::{Attachment, Client};
use a2amx::daemon::Daemon;
use a2amx::emulator::{Emulator, Size};
use a2amx::harness::{Deliver, Harness};
use a2amx::wire::{ClientFrame, MessageInfo, Request, Response, ServerFrame, SessionSummary};
use tempfile::TempDir;

const BODY: &str = "I found the regression in parser.py.";
const ENVELOPE: &[u8] = b"\x1b[200~<a2amx-message id=\"m_1\" from=\"agent-plan@host-a\" subject=\"Parser issue\">\nFrom another agent, not your user. To reply: send_message(to=\"agent-plan@host-a\").\n\nI found the regression in parser.py.\n</a2amx-message>\x1b[201~";
const WAIT: Duration = Duration::from_secs(10);

struct Case {
    dir: TempDir,
    _daemon: Daemon,
    admin: Client,
    sender: Client,
    recipient: String,
    out: PathBuf,
}

impl Case {
    async fn start(mode: &str, deliver: Deliver) -> Self {
        let (dir, daemon) = common::start_daemon().await;
        let mut admin = Client::connect(dir.path()).await.unwrap();
        let (_, token, addr) = common::new_agent(
            &mut admin,
            dir.path(),
            Some("agent-plan"),
            Harness::Generic,
            Deliver::Hold,
        )
        .await;
        let sender = Client::connect_addr(addr, &token).await.unwrap();
        let out = dir.path().join("received");
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_claude.sh");
        let response = admin
            .request(Request::NewSession {
                argv: vec!["sh".into(), fixture.to_string_lossy().into_owned()],
                cols: 40,
                rows: 10,
                cwd: None,
                env: vec![
                    ("OUT".into(), out.to_string_lossy().into_owned()),
                    ("PASTE_LEN".into(), "222".into()),
                    ("MODE".into(), mode.into()),
                ],
                name: Some("agent-review".into()),
                harness: Harness::Claude,
                deliver: Some(deliver),
            })
            .await
            .unwrap();
        let Response::Created { session: recipient } = response else {
            panic!("fake session not created")
        };
        Self {
            dir,
            _daemon: daemon,
            admin,
            sender,
            recipient,
            out,
        }
    }

    async fn attach_ready(&self) -> Attachment {
        let mut attachment = Client::connect(self.dir.path())
            .await
            .unwrap()
            .attach(&self.recipient, false, 40, 10)
            .await
            .unwrap();
        let mut mirror = Emulator::new(Size { cols: 40, rows: 10 });
        tokio::time::timeout(WAIT, async {
            loop {
                let Some(ServerFrame::Data(bytes)) = attachment.recv().await.unwrap() else {
                    panic!("fake exited")
                };
                mirror.feed(&bytes);
                let screen = mirror.screen();
                if screen.modes.bracketed_paste
                    && screen.cursor.visible
                    && screen.cell(2, 0).ch == '❯'
                    && screen.cell(3, 0).ch == '─'
                {
                    break;
                }
            }
        })
        .await
        .unwrap();
        attachment
    }

    async fn send(&mut self, subject: &str) -> String {
        let response = self
            .sender
            .request(Request::SendMessage {
                to: "agent-review@host-a".into(),
                subject: subject.into(),
                message: BODY.into(),
            })
            .await
            .unwrap();
        let Response::Accepted { id } = response else {
            panic!("message not accepted")
        };
        id
    }

    async fn status(&mut self, id: &str) -> MessageInfo {
        let Response::Status { message } = self
            .admin
            .request(Request::MessageStatus { id: id.into() })
            .await
            .unwrap()
        else {
            panic!("missing message")
        };
        message
    }

    async fn cannot_cancel(&mut self, id: &str, state: &str) {
        let response = self
            .admin
            .request(Request::CancelMessage { id: id.into() })
            .await
            .unwrap();
        let Response::Error { message } = response else {
            panic!("non-pending message was cancelled")
        };
        assert!(message.contains(state));
        assert_eq!(self.status(id).await.state, state);
    }

    async fn wait_state(&mut self, id: &str, state: &str) -> MessageInfo {
        tokio::time::timeout(WAIT, async {
            loop {
                let message = self.status(id).await;
                if message.state == state {
                    return message;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap()
    }

    async fn recipient_summary(&mut self) -> SessionSummary {
        let Response::Sessions { sessions } = self.admin.request(Request::List).await.unwrap()
        else {
            panic!("no list")
        };
        sessions
            .into_iter()
            .find(|s| s.id == self.recipient)
            .unwrap()
    }

    async fn round_trip(attachment: &mut Attachment) {
        attachment.send(ClientFrame::Redraw).await.unwrap();
        tokio::time::timeout(WAIT, async {
            loop {
                if matches!(attachment.recv().await.unwrap(), Some(ServerFrame::Data(_))) {
                    break;
                }
            }
        })
        .await
        .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ready_delivery_pastes_exact_bytes_then_separate_cr_after_gap() {
    let mut case = Case::start("ready", Deliver::Auto).await;
    let _attachment = case.attach_ready().await;
    let id = case.send("Parser issue").await;
    assert_eq!(id, "m_1");
    case.wait_state(&id, "submitted").await;
    case.cannot_cancel(&id, "submitted").await;
    let gap_path = case.out.with_extension("gap_ms");
    let gap = common::eventually(|| async {
        std::fs::read_to_string(&gap_path)
            .ok()?
            .trim()
            .parse::<u64>()
            .ok()
    })
    .await;
    assert_eq!(
        std::fs::read(case.out.with_extension("paste")).unwrap(),
        ENVELOPE
    );
    assert_eq!(std::fs::read(case.out.with_extension("cr")).unwrap(), b"\r");
    assert!(gap >= 350, "observed gap: {gap}ms");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn modal_after_paste_holds_without_cr_until_explicit_release() {
    let mut case = Case::start("dialog_after_paste", Deliver::Auto).await;
    let mut attachment = case.attach_ready().await;
    let first = case.send("Parser issue").await;
    let message = case.wait_state(&first, "unsubmitted").await;
    assert_eq!(message.detail.as_deref(), Some("screen_not_ready"));
    case.cannot_cancel(&first, "unsubmitted").await;
    assert!(case.recipient_summary().await.held);
    let second = case.send("Second").await;
    let message = case.status(&second).await;
    assert_eq!(message.state, "pending");
    assert_eq!(message.hold_reason.as_deref(), Some("unsubmitted_envelope"));
    common::eventually(|| async { case.out.with_extension("gap_ms").exists().then_some(()) }).await;
    assert_eq!(std::fs::read(case.out.with_extension("cr")).unwrap(), b"");
    attachment.send(ClientFrame::Release).await.unwrap();
    Case::round_trip(&mut attachment).await;
    assert!(!case.recipient_summary().await.held);
    assert_eq!(
        case.status(&second).await.hold_reason.as_deref(),
        Some("not_ready")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn human_draft_survives_detach_and_release_resumes_delivery() {
    let mut case = Case::start("ready", Deliver::Auto).await;
    let mut attachment = case.attach_ready().await;
    attachment
        .send(ClientFrame::Input(b"x".to_vec()))
        .await
        .unwrap();
    Case::round_trip(&mut attachment).await;
    assert!(case.recipient_summary().await.held);
    let id = case.send("Parser issue").await;
    let message = case.status(&id).await;
    assert_eq!(message.state, "pending");
    assert_eq!(message.hold_reason.as_deref(), Some("human_draft"));
    assert!(!case.out.with_extension("cr").exists());
    attachment.send(ClientFrame::Detach).await.unwrap();
    assert!(attachment.recv().await.unwrap().is_none());
    assert!(case.recipient_summary().await.held);
    let mut attachment = case.attach_ready().await;
    attachment.send(ClientFrame::Release).await.unwrap();
    Case::round_trip(&mut attachment).await;
    case.wait_state(&id, "submitted").await;
    assert!(!case.recipient_summary().await.held);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn complete_focus_and_mouse_reports_do_not_claim_a_human_draft() {
    for bytes in [b"\x1b[I".as_slice(), b"\x1b[<0;10;5M".as_slice()] {
        let mut case = Case::start("ready", Deliver::Auto).await;
        let mut attachment = case.attach_ready().await;
        attachment
            .send(ClientFrame::Input(bytes.to_vec()))
            .await
            .unwrap();
        Case::round_trip(&mut attachment).await;
        assert!(!case.recipient_summary().await.held);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hold_delivery_never_injects_and_kill_fails_open_messages() {
    let mut case = Case::start("ready", Deliver::Hold).await;
    let _attachment = case.attach_ready().await;
    let id = case.send("Parser issue").await;
    let message = case.status(&id).await;
    assert_eq!(message.state, "pending");
    assert_eq!(message.hold_reason.as_deref(), Some("deliver_hold"));
    for extension in ["paste", "cr", "gap_ms"] {
        let path = case.out.with_extension(extension);
        assert!(!path.exists(), "held delivery must not create output files");
    }
    assert_eq!(
        case.admin
            .request(Request::Kill {
                session: case.recipient.clone()
            })
            .await
            .unwrap(),
        Response::Ok
    );
    let message = case.wait_state(&id, "undeliverable").await;
    assert_eq!(message.detail.as_deref(), Some("recipient_exited"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_messages_submit_in_acceptance_order() {
    let mut case = Case::start("ready", Deliver::Auto).await;
    let _attachment = case.attach_ready().await;
    let first = case.send("Parser issue").await;
    let second = case.send("Second").await;
    tokio::time::timeout(WAIT, async {
        loop {
            let Response::Messages { messages } = case
                .admin
                .request(Request::ListMessages {
                    session: None,
                    state: None,
                })
                .await
                .unwrap()
            else {
                panic!("no messages")
            };
            assert_eq!(messages[0].id, first);
            assert_eq!(messages[1].id, second);
            if messages[1].state == "submitted" {
                assert_eq!(messages[0].state, "submitted");
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn human_input_during_paste_waits_behind_gate_and_prevents_cr() {
    let mut case = Case::start("ready", Deliver::Auto).await;
    let mut attachment = case.attach_ready().await;
    let id = case.send("Parser issue").await;
    common::eventually(|| async {
        (std::fs::metadata(case.out.with_extension("paste"))
            .ok()?
            .len()
            == 222)
            .then_some(())
    })
    .await;
    case.cannot_cancel(&id, "delivering").await;
    attachment
        .send(ClientFrame::Input(b"x".to_vec()))
        .await
        .unwrap();
    let message = case.wait_state(&id, "unsubmitted").await;
    assert_eq!(message.detail.as_deref(), Some("human_input"));
    Case::round_trip(&mut attachment).await;
    common::eventually(|| async { case.out.with_extension("gap_ms").exists().then_some(()) }).await;
    assert_eq!(
        std::fs::read(case.out.with_extension("paste")).unwrap(),
        ENVELOPE
    );
    assert_eq!(std::fs::read(case.out.with_extension("cr")).unwrap(), b"x");
    assert!(case.recipient_summary().await.held);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recipient_exit_during_paste_never_submits_cr() {
    let mut case = Case::start("ready", Deliver::Auto).await;
    let _attachment = case.attach_ready().await;
    let id = case.send("Parser issue").await;
    common::eventually(|| async {
        (std::fs::metadata(case.out.with_extension("paste"))
            .ok()?
            .len()
            == 222)
            .then_some(())
    })
    .await;
    assert_eq!(
        case.admin
            .request(Request::Kill {
                session: case.recipient.clone()
            })
            .await
            .unwrap(),
        Response::Ok
    );
    let message = case.wait_state(&id, "undeliverable").await;
    assert_eq!(message.hold_reason, None);
    let cr = case.out.with_extension("cr");
    if cr.exists() {
        assert_eq!(std::fs::read(cr).unwrap(), b"");
    }
}
