#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use a2amx::client::{Client, IssuedLeaseCode, NewSessionRequest};
use a2amx::confine::ConfineRequest;
use a2amx::harness::{Deliver, Harness};
use a2amx::messaging::{RemoteMode, TeamScope};
use a2amx::wire::{ActivityWatch, ExplainInfo, MessageInfo, Request, ResetVerdict, Response};
use tokio::net::TcpListener;

async fn spawn_fake(
    replies: Vec<Response>,
) -> anyhow::Result<(
    SocketAddr,
    Arc<Mutex<Vec<Request>>>,
    tokio::task::JoinHandle<()>,
)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let recorded_task = Arc::clone(&recorded);
    let handle = tokio::spawn(async move {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        let Some(hello_bytes) = common::fake_recv_frame(&mut stream).await.ok().flatten() else {
            return;
        };
        let Ok(Request::Hello { .. }) = serde_json::from_slice(&hello_bytes) else {
            return;
        };
        let Ok(ok_bytes) = serde_json::to_vec(&Response::Ok) else {
            return;
        };
        if common::fake_send_frame(&mut stream, &ok_bytes)
            .await
            .is_err()
        {
            return;
        }
        for reply in replies {
            let Some(req_bytes) = common::fake_recv_frame(&mut stream).await.ok().flatten() else {
                break;
            };
            let Ok(req) = serde_json::from_slice::<Request>(&req_bytes) else {
                break;
            };
            if let Ok(mut guard) = recorded_task.lock() {
                guard.push(req);
            }
            let Ok(resp_bytes) = serde_json::to_vec(&reply) else {
                break;
            };
            if common::fake_send_frame(&mut stream, &resp_bytes)
                .await
                .is_err()
            {
                break;
            }
        }
    });
    Ok((addr, recorded, handle))
}

/// Run one typed call against a fake daemon that answers with `reply`; return the call's result and the requests the daemon recorded.
async fn exchange<T>(
    reply: Response,
    call: impl AsyncFnOnce(&mut Client) -> T,
) -> (T, Vec<Request>) {
    let (addr, recorded, handle) = spawn_fake(vec![reply]).await.unwrap();
    let mut client = Client::connect_addr(addr, "token").await.unwrap();
    let result = call(&mut client).await;
    drop(client);
    handle.await.unwrap();
    let requests = recorded.lock().unwrap().clone();
    (result, requests)
}

fn sample_explain() -> ExplainInfo {
    ExplainInfo {
        id: "s1".into(),
        address: "plain@host-a".into(),
        harness: Harness::Generic,
        exit_code: None,
        ready: Some(true),
        starting: false,
        activity: None,
        activity_cause: None,
        needs_input: None,
        hold: None,
        pending: 0,
        reset: ResetVerdict {
            allowed: true,
            code: None,
            reason: None,
        },
    }
}

fn sample_message() -> MessageInfo {
    MessageInfo {
        id: "m1".into(),
        from: "a@host-a".into(),
        to: "b@host-a".into(),
        subject: "hi".into(),
        state: "undeliverable".into(),
        detail: None,
        hold_reason: None,
        evidence: None,
        hold_explanation: None,
        accepted_at: None,
        updated_at: None,
    }
}

#[tokio::test]
async fn sessions_success_error_and_unexpected() {
    let (res, req) = exchange(Response::Sessions { sessions: vec![] }, async |c| {
        c.sessions().await
    })
    .await;
    assert_eq!(res.unwrap(), vec![]);
    assert_eq!(req, vec![Request::List]);

    let (res, _) = exchange(
        Response::Error {
            message: "boom".into(),
        },
        async |c| c.sessions().await,
    )
    .await;
    assert_eq!(res.unwrap_err().to_string(), "boom");

    let (res, _) = exchange(
        Response::Created {
            session: "x".into(),
        },
        async |c| c.sessions().await,
    )
    .await;
    assert_eq!(
        res.unwrap_err().to_string(),
        "unexpected daemon response: Created { session: \"x\" }"
    );
}

#[tokio::test]
async fn kill_success_error_and_unexpected() {
    let (res, req) = exchange(Response::Ok, async |c| c.kill("s1".into(), true).await).await;
    res.unwrap();
    assert_eq!(
        req,
        vec![Request::Kill {
            session: "s1".into(),
            now: true,
        }]
    );

    let (res, _) = exchange(
        Response::Error {
            message: "no such session".into(),
        },
        async |c| c.kill("s1".into(), false).await,
    )
    .await;
    assert_eq!(res.unwrap_err().to_string(), "no such session");

    let (res, _) = exchange(
        Response::Created {
            session: "x".into(),
        },
        async |c| c.kill("s1".into(), false).await,
    )
    .await;
    assert_eq!(
        res.unwrap_err().to_string(),
        "unexpected daemon response: Created { session: \"x\" }"
    );
}

#[tokio::test]
async fn reset_success_failed_error_and_unexpected() {
    let (res, req) = exchange(Response::Reset { steps: 2 }, async |c| c.reset("s1").await).await;
    assert_eq!(res.unwrap(), 2);
    assert_eq!(
        req,
        vec![Request::Reset {
            session: "s1".into(),
        }]
    );

    let (res, _) = exchange(
        Response::Failed {
            code: "draft_present".into(),
            message: "draft".into(),
        },
        async |c| c.reset("s1").await,
    )
    .await;
    assert_eq!(res.unwrap_err().to_string(), "draft_present: draft");

    let (res, _) = exchange(
        Response::Error {
            message: "boom".into(),
        },
        async |c| c.reset("s1").await,
    )
    .await;
    assert_eq!(res.unwrap_err().to_string(), "boom");

    let (res, _) = exchange(
        Response::Created {
            session: "x".into(),
        },
        async |c| c.reset("s1").await,
    )
    .await;
    assert_eq!(
        res.unwrap_err().to_string(),
        "unexpected daemon response: Created { session: \"x\" }"
    );
}

#[tokio::test]
async fn shutdown_success_error_and_unexpected() {
    let (res, req) = exchange(Response::Ok, async |c| c.shutdown(false).await).await;
    res.unwrap();
    assert_eq!(req, vec![Request::Shutdown { now: false }]);

    let (res, _) = exchange(
        Response::Error {
            message: "denied".into(),
        },
        async |c| c.shutdown(true).await,
    )
    .await;
    assert_eq!(res.unwrap_err().to_string(), "denied");

    let (res, _) = exchange(
        Response::Created {
            session: "x".into(),
        },
        async |c| c.shutdown(false).await,
    )
    .await;
    assert_eq!(
        res.unwrap_err().to_string(),
        "unexpected daemon response: Created { session: \"x\" }"
    );
}

#[tokio::test]
async fn version_success_old_daemon_and_unexpected() {
    let (res, req) = exchange(
        Response::Version {
            version: "1.2".into(),
        },
        async |c| c.version().await,
    )
    .await;
    assert_eq!(res.unwrap(), Some("1.2".into()));
    assert_eq!(req, vec![Request::Version]);

    let (res, _) = exchange(
        Response::Error {
            message: "old".into(),
        },
        async |c| c.version().await,
    )
    .await;
    assert_eq!(res.unwrap(), None);

    let (res, _) = exchange(Response::Ok, async |c| c.version().await).await;
    assert_eq!(
        res.unwrap_err().to_string(),
        "unexpected daemon response: Ok"
    );
}

#[tokio::test]
async fn explain_success_error_and_unexpected() {
    let info = sample_explain();
    let (res, req) = exchange(Response::Explain { info: info.clone() }, async |c| {
        c.explain("s1".into()).await
    })
    .await;
    assert_eq!(res.unwrap(), info);
    assert_eq!(
        req,
        vec![Request::Explain {
            session: "s1".into(),
        }]
    );

    let (res, _) = exchange(
        Response::Error {
            message: "missing".into(),
        },
        async |c| c.explain("s1".into()).await,
    )
    .await;
    assert_eq!(res.unwrap_err().to_string(), "missing");

    let (res, _) = exchange(
        Response::Created {
            session: "x".into(),
        },
        async |c| c.explain("s1".into()).await,
    )
    .await;
    assert_eq!(
        res.unwrap_err().to_string(),
        "unexpected daemon response: Created { session: \"x\" }"
    );
}

#[tokio::test]
async fn screen_success_error_and_unexpected() {
    let (res, req) = exchange(
        Response::Screen {
            lines: vec!["line1".into(), "line2".into()],
        },
        async |c| c.screen("s1".into()).await,
    )
    .await;
    assert_eq!(res.unwrap(), vec!["line1".to_owned(), "line2".to_owned()]);
    assert_eq!(
        req,
        vec![Request::Screen {
            session: "s1".into(),
        }]
    );

    let (res, _) = exchange(
        Response::Error {
            message: "missing".into(),
        },
        async |c| c.screen("s1".into()).await,
    )
    .await;
    assert_eq!(res.unwrap_err().to_string(), "missing");

    let (res, _) = exchange(
        Response::Created {
            session: "x".into(),
        },
        async |c| c.screen("s1".into()).await,
    )
    .await;
    assert_eq!(
        res.unwrap_err().to_string(),
        "unexpected daemon response: Created { session: \"x\" }"
    );
}

#[tokio::test]
async fn messages_success_failed_error_and_unexpected() {
    let msg = sample_message();
    let (res, req) = exchange(
        Response::Messages {
            messages: vec![msg.clone()],
        },
        async |c| c.messages(None, Some("undeliverable".into())).await,
    )
    .await;
    assert_eq!(res.unwrap(), vec![msg]);
    assert_eq!(
        req,
        vec![Request::ListMessages {
            session: None,
            state: Some("undeliverable".into()),
        }]
    );

    let (res, _) = exchange(
        Response::Failed {
            code: "c".into(),
            message: "m".into(),
        },
        async |c| c.messages(None, Some("undeliverable".into())).await,
    )
    .await;
    assert_eq!(res.unwrap_err().to_string(), "m");

    let (res, _) = exchange(
        Response::Error {
            message: "e".into(),
        },
        async |c| c.messages(None, None).await,
    )
    .await;
    assert_eq!(res.unwrap_err().to_string(), "e");

    let (res, _) = exchange(
        Response::Created {
            session: "x".into(),
        },
        async |c| c.messages(None, None).await,
    )
    .await;
    assert_eq!(
        res.unwrap_err().to_string(),
        "unexpected daemon response: Created { session: \"x\" }"
    );
}

#[tokio::test]
async fn cancel_message_success_failed_error_and_unexpected() {
    let (res, req) = exchange(Response::Ok, async |c| c.cancel_message("m1".into()).await).await;
    res.unwrap();
    assert_eq!(req, vec![Request::CancelMessage { id: "m1".into() }]);

    let (res, _) = exchange(
        Response::Failed {
            code: "c".into(),
            message: "gone".into(),
        },
        async |c| c.cancel_message("m1".into()).await,
    )
    .await;
    assert_eq!(res.unwrap_err().to_string(), "gone");

    let (res, _) = exchange(
        Response::Error {
            message: "e".into(),
        },
        async |c| c.cancel_message("m1".into()).await,
    )
    .await;
    assert_eq!(res.unwrap_err().to_string(), "e");

    let (res, _) = exchange(
        Response::Created {
            session: "x".into(),
        },
        async |c| c.cancel_message("m1".into()).await,
    )
    .await;
    assert_eq!(
        res.unwrap_err().to_string(),
        "unexpected daemon response: Created { session: \"x\" }"
    );
}

#[tokio::test]
async fn issue_lease_code_success_error_and_unexpected() {
    let (res, req) = exchange(
        Response::LeaseCode {
            code: "AB12".into(),
            agents: vec!["a".into()],
            valid_secs: 60,
        },
        async |c| c.issue_lease_code("t1".into()).await,
    )
    .await;
    assert_eq!(
        res.unwrap(),
        IssuedLeaseCode {
            code: "AB12".into(),
            agents: vec!["a".into()],
            valid_secs: 60,
        }
    );
    assert_eq!(req, vec![Request::IssueLeaseCode { team: "t1".into() }]);

    let (res, _) = exchange(
        Response::Error {
            message: "no remote".into(),
        },
        async |c| c.issue_lease_code("t1".into()).await,
    )
    .await;
    assert_eq!(res.unwrap_err().to_string(), "no remote");

    let (res, _) = exchange(
        Response::Created {
            session: "x".into(),
        },
        async |c| c.issue_lease_code("t1".into()).await,
    )
    .await;
    assert_eq!(
        res.unwrap_err().to_string(),
        "unexpected daemon response: Created { session: \"x\" }"
    );
}

fn new_session() -> NewSessionRequest {
    NewSessionRequest {
        argv: vec!["sleep".into(), "60".into()],
        cols: 100,
        rows: 30,
        cwd: Some("/tmp/work".into()),
        env: vec![("K".into(), "V".into())],
        reset: vec!["a".into()],
        control_from: vec!["c".into()],
        watch: vec!["w".into()],
        activity: Some(ActivityWatch {
            working_after: Some(120),
            idle_after: Some(600),
        }),
        heartbeat: Some(30),
        name: Some("n1".into()),
        harness: Harness::Claude,
        deliver: Some(Deliver::Hold),
        team: Some(TeamScope {
            name: "team-a".into(),
            private: true,
            allow: vec!["agent-b".into()],
            agents: vec!["agent-c".into()],
        }),
        role: Some("r".into()),
        remote: Some(RemoteMode::Both),
    }
}

fn recorded_json(recorded: &[Request]) -> Vec<serde_json::Value> {
    recorded
        .iter()
        .map(|request| serde_json::to_value(request).unwrap())
        .collect()
}

fn expected_new_session() -> serde_json::Value {
    serde_json::json!({
        "type": "new_session",
        "argv": ["sleep", "60"],
        "cols": 100,
        "rows": 30,
        "cwd": "/tmp/work",
        "env": [["K", "V"]],
        "reset": ["a"],
        "control_from": ["c"],
        "watch": ["w"],
        "activity": {"working_after": 120, "idle_after": 600},
        "heartbeat": 30,
        "name": "n1",
        "harness": "claude",
        "deliver": "hold",
        "team": {"name": "team-a", "private": true, "allow": ["agent-b"], "agents": ["agent-c"]},
        "role": "r",
        "remote": "both",
    })
}

#[tokio::test]
async fn create_session_unconfined_success_records_all_fields() {
    let session = new_session();
    let expected = expected_new_session();
    let (res, req) = exchange(
        Response::Created {
            session: "s9".into(),
        },
        async |c| c.create_session(session, None).await,
    )
    .await;
    assert_eq!(res.unwrap(), "s9");
    assert_eq!(recorded_json(&req), vec![expected]);
}

#[tokio::test]
async fn create_session_confined_success_wraps_all_fields() {
    let session = new_session();
    let confine = ConfineRequest {
        grants: vec!["git".into()],
        write: vec!["/tmp/w".into()],
        ..Default::default()
    };
    let expected = serde_json::json!({
        "type": "confine",
        "confine": serde_json::to_value(&confine).unwrap(),
        "request": expected_new_session(),
    });
    let (res, req) = exchange(
        Response::Created {
            session: "s9".into(),
        },
        async |c| c.create_session(session, Some(confine)).await,
    )
    .await;
    assert_eq!(res.unwrap(), "s9");
    assert_eq!(recorded_json(&req), vec![expected]);
}

#[tokio::test]
async fn create_session_confined_version_skew_maps_error() {
    let (res, _) = exchange(
        Response::Error {
            message: "invalid control request: unknown variant".into(),
        },
        async |c| {
            c.create_session(new_session(), Some(ConfineRequest::default()))
                .await
        },
    )
    .await;
    assert_eq!(
        res.unwrap_err().to_string(),
        "the running daemon does not support --confine; restart it"
    );
}

#[tokio::test]
async fn create_session_unconfined_version_skew_keeps_daemon_error() {
    let (res, _) = exchange(
        Response::Error {
            message: "invalid control request: unknown variant".into(),
        },
        async |c| c.create_session(new_session(), None).await,
    )
    .await;
    assert_eq!(
        res.unwrap_err().to_string(),
        "invalid control request: unknown variant"
    );
}

#[tokio::test]
async fn create_session_other_error_and_unexpected_reply() {
    let (res, _) = exchange(
        Response::Error {
            message: "other".into(),
        },
        async |c| {
            c.create_session(new_session(), Some(ConfineRequest::default()))
                .await
        },
    )
    .await;
    assert_eq!(res.unwrap_err().to_string(), "other");

    let (res, _) = exchange(Response::Ok, async |c| {
        c.create_session(new_session(), None).await
    })
    .await;
    assert_eq!(
        res.unwrap_err().to_string(),
        "unexpected daemon response: Ok"
    );
}

#[tokio::test]
async fn transport_failure_after_hello_returns_err() {
    let (addr, _, handle) = spawn_fake(vec![]).await.unwrap();
    let mut client = Client::connect_addr(addr, "token").await.unwrap();
    assert!(client.kill("s1".into(), false).await.is_err());
    handle.await.unwrap();
}
