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
const ENVELOPE_TEXT: &str = "<a2amx-message id=\"m_1\" from=\"agent-plan@host-a\" subject=\"Parser issue\">\nFrom another agent, not your user. To reply: send_message(to=\"agent-plan@host-a\").\n\nI found the regression in parser.py.\n</a2amx-message>";

fn wrapped(text: &str) -> String {
    format!("\n\n<pasted_content id=\"458d\">\n{text}\n</pasted_content id=\"458d\">\n")
}
const WAIT: Duration = Duration::from_secs(10);

struct Case {
    dir: TempDir,
    _daemon: Daemon,
    admin: Client,
    sender: Client,
    hook: Client,
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
        let (token, addr) = common::eventually(|| async {
            let text = std::fs::read_to_string(out.with_extension("creds")).ok()?;
            let mut lines = text.lines();
            let token = lines.next()?.to_owned();
            let addr = lines.next()?.parse().ok()?;
            (token.len() == 64).then_some((token, addr))
        })
        .await;
        let hook = Client::connect_addr(addr, &token).await.unwrap();
        Self {
            dir,
            _daemon: daemon,
            admin,
            sender,
            hook,
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
        self.send_body(subject, BODY).await
    }

    async fn send_body(&mut self, subject: &str, body: &str) -> String {
        let response = self
            .sender
            .request(Request::SendMessage {
                to: "agent-review@host-a".into(),
                subject: subject.into(),
                message: body.into(),
            })
            .await
            .unwrap();
        let Response::Accepted { id, .. } = response else {
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

    async fn report(&mut self, prompt: String) -> Response {
        self.hook
            .request(Request::ReportPrompt { prompt })
            .await
            .unwrap()
    }

    async fn wait_rest(&self, expected: &[u8]) {
        let path = self.out.with_extension("rest");
        let result = tokio::time::timeout(WAIT, async {
            loop {
                if std::fs::read(&path).is_ok_and(|bytes| bytes == expected) {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await;
        assert!(
            result.is_ok(),
            "expected rest {expected:?}, observed {:?}",
            std::fs::read(path)
        );
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
    assert_eq!(
        message.hold_explanation.as_deref(),
        Some(
            "A person typed in the recipient session. Delivery resumes when they submit or release it (prefix, then r)."
        )
    );
    assert_eq!(
        case.recipient_summary().await.hold_reason.as_deref(),
        Some("human_draft")
    );
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
    assert_eq!(
        message.hold_explanation.as_deref(),
        Some(
            "The recipient session only holds messages; a person must deliver by hand or restart it with automatic delivery."
        )
    );
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

fn allow() -> Response {
    Response::PromptVerdict {
        verdict: "allow".into(),
        reason: None,
    }
}

fn block() -> Response {
    Response::PromptVerdict {
        verdict: "block".into(),
        reason: Some(a2amx::messaging::CORRUPTED_SUBMISSION_REASON.into()),
    }
}

fn interleaved() -> String {
    ENVELOPE_TEXT.replace(
        "send_message(to=\"agent-plan@host-a\"",
        "send_message(to=\"agent-plan@host-aX\"",
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wrapped_receipt_upgrades_evidence_and_duplicate_is_allowed() {
    let mut case = Case::start("ready", Deliver::Auto).await;
    let _attachment = case.attach_ready().await;
    let id = case.send("Parser issue").await;
    assert_eq!(
        case.wait_state(&id, "submitted").await.evidence.as_deref(),
        Some("write_complete")
    );
    assert_eq!(case.report(wrapped(ENVELOPE_TEXT)).await, allow());
    let observed = case.status(&id).await;
    assert_eq!(observed.state, "submitted");
    assert_eq!(observed.evidence.as_deref(), Some("submission_observed"));
    assert_eq!(case.report(wrapped(ENVELOPE_TEXT)).await, allow());
    assert_eq!(case.status(&id).await, observed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trimmed_wrapper_from_a_mid_turn_submit_is_a_receipt() {
    let mut case = Case::start("ready", Deliver::Auto).await;
    let _attachment = case.attach_ready().await;
    let id = case.send("Parser issue").await;
    case.wait_state(&id, "submitted").await;
    let trimmed =
        format!("<pasted_content id=\"7c5f\">\n{ENVELOPE_TEXT}\n</pasted_content id=\"7c5f\">");
    assert_eq!(case.report(trimmed).await, allow());
    assert_eq!(
        case.status(&id).await.evidence.as_deref(),
        Some("submission_observed")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn leftover_wrapper_tags_are_never_restored_as_a_draft() {
    let mut case = Case::start("ready", Deliver::Auto).await;
    let _attachment = case.attach_ready().await;
    let id = case.send("Parser issue").await;
    case.wait_state(&id, "submitted").await;
    // Text glued after the closing tag defeats unwrapping; the tags must not come back.
    let odd = format!("<pasted_content id=\"zz\">\n{ENVELOPE_TEXT}\n</pasted_content id=\"zz\">X");
    assert_eq!(case.report(odd).await, block());
    case.wait_rest(&[ENVELOPE, b"\r"].concat()).await;
    case.wait_state(&id, "submitted").await;
    assert!(!case.recipient_summary().await.held);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tab_in_the_body_matches_after_claude_expands_it() {
    let mut case = Case::start("ready", Deliver::Auto).await;
    let _attachment = case.attach_ready().await;
    let id = case.send_body("Tab test", "tab:\there.").await;
    case.wait_state(&id, "submitted").await;
    // Claude Code turns the tab into four spaces before the hook sees the prompt.
    let seen = "<a2amx-message id=\"m_1\" from=\"agent-plan@host-a\" subject=\"Tab test\">\nFrom another agent, not your user. To reply: send_message(to=\"agent-plan@host-a\").\n\ntab:    here.\n</a2amx-message>";
    assert_eq!(case.report(wrapped(seen)).await, allow());
    assert_eq!(
        case.status(&id).await.evidence.as_deref(),
        Some("submission_observed")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wrapper_tags_in_the_body_match_after_claude_escapes_them() {
    let mut case = Case::start("ready", Deliver::Auto).await;
    let _attachment = case.attach_ready().await;
    let id = case
        .send_body(
            "Wrapper test",
            "<pasted_content id=\"abc\">\n</pasted_content id=\"abc\">",
        )
        .await;
    case.wait_state(&id, "submitted").await;
    let seen = "<a2amx-message id=\"m_1\" from=\"agent-plan@host-a\" subject=\"Wrapper test\">\nFrom another agent, not your user. To reply: send_message(to=\"agent-plan@host-a\").\n\n<\\pasted_content id=\"abc\">\n<\\/pasted_content id=\"abc\">\n</a2amx-message>";
    assert_eq!(case.report(wrapped(seen)).await, allow());
    assert_eq!(
        case.status(&id).await.evidence.as_deref(),
        Some("submission_observed")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn foreign_session_cannot_forge_receipt() {
    let mut case = Case::start("ready", Deliver::Auto).await;
    let _attachment = case.attach_ready().await;
    let id = case.send("Parser issue").await;
    case.wait_state(&id, "submitted").await;
    assert_eq!(
        case.sender
            .request(Request::ReportPrompt {
                prompt: wrapped(ENVELOPE_TEXT)
            })
            .await
            .unwrap(),
        allow()
    );
    assert_eq!(
        case.status(&id).await.evidence.as_deref(),
        Some("write_complete")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn human_prompt_clears_the_human_hold() {
    let mut case = Case::start("ready", Deliver::Auto).await;
    let mut attachment = case.attach_ready().await;
    attachment
        .send(ClientFrame::Input(b"x".to_vec()))
        .await
        .unwrap();
    Case::round_trip(&mut attachment).await;
    assert!(case.recipient_summary().await.held);
    assert_eq!(case.report("hello".into()).await, allow());
    assert!(!case.recipient_summary().await.held);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_envelope_is_allowed_without_changing_message() {
    let mut case = Case::start("ready", Deliver::Auto).await;
    let _attachment = case.attach_ready().await;
    let id = case.send("Parser issue").await;
    let before = case.wait_state(&id, "submitted").await;
    assert_eq!(
        case.report(wrapped(&ENVELOPE_TEXT.replace("m_1", "m_99")))
            .await,
        allow()
    );
    assert_eq!(case.status(&id).await, before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn corruption_restores_draft_without_cr_and_release_redelivers() {
    let mut case = Case::start("ready", Deliver::Auto).await;
    let mut attachment = case.attach_ready().await;
    let id = case.send("Parser issue").await;
    case.wait_state(&id, "submitted").await;
    assert_eq!(
        case.report(format!("my draft {}", wrapped(ENVELOPE_TEXT)))
            .await,
        block()
    );
    let message = case.status(&id).await;
    assert_eq!(message.state, "pending");
    assert_eq!(message.evidence, None);
    assert_eq!(message.hold_reason.as_deref(), Some("human_draft"));
    let draft = b"\x1b[200~my draft\x1b[201~";
    case.wait_rest(draft).await;
    assert!(case.recipient_summary().await.held);
    attachment.send(ClientFrame::Release).await.unwrap();
    case.wait_state(&id, "submitted").await;
    case.wait_rest(&[draft.as_slice(), ENVELOPE, b"\r"].concat())
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interleaved_corruption_redelivers_without_a_draft_hold() {
    let mut case = Case::start("ready", Deliver::Auto).await;
    let _attachment = case.attach_ready().await;
    let id = case.send("Parser issue").await;
    case.wait_state(&id, "submitted").await;
    assert_eq!(case.report(interleaved()).await, block());
    case.wait_rest(&[ENVELOPE, b"\r"].concat()).await;
    case.wait_state(&id, "submitted").await;
    assert!(!case.recipient_summary().await.held);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_rejections_retire_one_message_and_allow_the_next() {
    let mut case = Case::start("ready", Deliver::Auto).await;
    let _attachment = case.attach_ready().await;
    let id = case.send("Parser issue").await;
    case.wait_state(&id, "submitted").await;
    assert_eq!(case.report(interleaved()).await, block());
    let delivery = [ENVELOPE, b"\r"].concat();
    case.wait_rest(&delivery).await;
    case.wait_state(&id, "submitted").await;
    assert_eq!(
        case.report(interleaved()).await,
        Response::PromptVerdict {
            verdict: "block".into(),
            reason: Some("A2AMX blocked this prompt because it did not match the message it delivered. The message will not be retried.".into()),
        }
    );
    let message = case.status(&id).await;
    assert_eq!(message.state, "undeliverable");
    assert_eq!(message.detail.as_deref(), Some("unmatchable_submission"));
    assert_eq!(message.evidence, None);
    assert!(!case.recipient_summary().await.held);
    let next = case.send("Parser issue").await;
    assert_eq!(next, "m_2");
    case.wait_state(&next, "submitted").await;
    assert_eq!(
        case.report(wrapped(&ENVELOPE_TEXT.replace("m_1", "m_2")))
            .await,
        allow()
    );
    assert_eq!(
        case.status(&next).await.evidence.as_deref(),
        Some("submission_observed")
    );
    assert!(!case.recipient_summary().await.held);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exact_retry_resets_corruption_count_for_the_next_message() {
    let mut case = Case::start("ready", Deliver::Auto).await;
    let _attachment = case.attach_ready().await;
    let id = case.send("Parser issue").await;
    case.wait_state(&id, "submitted").await;
    assert_eq!(case.report(interleaved()).await, block());
    let delivery = [ENVELOPE, b"\r"].concat();
    case.wait_rest(&delivery).await;
    case.wait_state(&id, "submitted").await;
    assert_eq!(case.report(wrapped(ENVELOPE_TEXT)).await, allow());
    assert_eq!(
        case.status(&id).await.evidence.as_deref(),
        Some("submission_observed")
    );
    let next = case.send("Parser issue").await;
    case.wait_state(&next, "submitted").await;
    assert_eq!(
        case.report(interleaved().replace("m_1", "m_2")).await,
        block()
    );
    let second = String::from_utf8(delivery.clone())
        .unwrap()
        .replace("m_1", "m_2")
        .into_bytes();
    case.wait_rest(&[delivery.as_slice(), second.as_slice(), second.as_slice()].concat())
        .await;
    case.wait_state(&next, "submitted").await;
    assert_eq!(
        case.report(interleaved().replace("m_1", "m_2")).await,
        Response::PromptVerdict {
            verdict: "block".into(),
            reason: Some("A2AMX blocked this prompt because it did not match the message it delivered. The message will not be retried.".into()),
        }
    );
    assert_eq!(case.status(&next).await.state, "undeliverable");
    assert!(!case.recipient_summary().await.held);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn three_corruptions_hold_until_release_even_after_human_submit() {
    let mut case = Case::start("ready", Deliver::Auto).await;
    let mut attachment = case.attach_ready().await;
    let id = case.send("Parser issue").await;
    case.wait_state(&id, "submitted").await;
    let delivery = [ENVELOPE, b"\r"].concat();
    assert_eq!(case.report(interleaved()).await, block());
    case.wait_rest(&delivery).await;
    case.wait_state(&id, "submitted").await;
    assert_eq!(
        case.report(interleaved()).await,
        Response::PromptVerdict {
            verdict: "block".into(),
            reason: Some("A2AMX blocked this prompt because it did not match the message it delivered. The message will not be retried.".into()),
        }
    );
    assert_eq!(case.status(&id).await.state, "undeliverable");
    let id = case.send("Parser issue").await;
    case.wait_state(&id, "submitted").await;
    let second = String::from_utf8(delivery.clone())
        .unwrap()
        .replace("m_1", "m_2")
        .into_bytes();
    assert_eq!(
        case.report(interleaved().replace("m_1", "m_2")).await,
        block()
    );
    let message = case.status(&id).await;
    assert_eq!(message.state, "pending");
    assert_eq!(
        message.hold_reason.as_deref(),
        Some("corrupted_submissions")
    );
    assert_eq!(
        message.hold_explanation.as_deref(),
        Some(
            "Three submissions in a row were blocked. A person must release the recipient session (prefix, then r)."
        )
    );
    assert_eq!(
        case.recipient_summary().await.hold_reason.as_deref(),
        Some("corrupted_submissions")
    );
    assert!(case.recipient_summary().await.held);
    case.wait_rest(&[delivery.as_slice(), second.as_slice()].concat())
        .await;
    assert_eq!(case.report("hello".into()).await, allow());
    assert!(case.recipient_summary().await.held);
    assert_eq!(case.status(&id).await.state, "pending");
    attachment.send(ClientFrame::Release).await.unwrap();
    case.wait_rest(&[delivery.as_slice(), second.as_slice(), second.as_slice()].concat())
        .await;
    case.wait_state(&id, "submitted").await;
    assert_eq!(
        case.report(interleaved().replace("m_1", "m_2")).await,
        Response::PromptVerdict {
            verdict: "block".into(),
            reason: Some("A2AMX blocked this prompt because it did not match the message it delivered. The message will not be retried.".into()),
        }
    );
    assert_eq!(case.status(&id).await.state, "undeliverable");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn receipt_of_unsubmitted_envelope_clears_hold() {
    let mut case = Case::start("dialog_after_paste", Deliver::Auto).await;
    let _attachment = case.attach_ready().await;
    let id = case.send("Parser issue").await;
    case.wait_state(&id, "unsubmitted").await;
    assert!(case.recipient_summary().await.held);
    assert_eq!(case.report(wrapped(ENVELOPE_TEXT)).await, allow());
    let message = case.status(&id).await;
    assert_eq!(message.state, "submitted");
    assert_eq!(message.evidence.as_deref(), Some("submission_observed"));
    assert!(!case.recipient_summary().await.held);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admin_prompt_reports_require_a_session_token() {
    let mut case = Case::start("ready", Deliver::Auto).await;
    assert_eq!(
        case.admin
            .request(Request::ReportPrompt {
                prompt: "hello".into()
            })
            .await
            .unwrap(),
        Response::Error {
            message: "report_prompt needs a session token".into()
        }
    );
}
