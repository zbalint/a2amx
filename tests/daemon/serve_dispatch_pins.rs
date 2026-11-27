use std::time::Duration;

use a2amx::harness::{Deliver, Harness};
use a2amx::wire::{BridgeDown, BridgeUp};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::*;

#[tokio::test]
async fn shutdown_replies_ok_once_then_closes() {
    let (dir, daemon) = common::start_daemon().await;
    let token = std::fs::read_to_string(dir.path().join("run/admin.token")).unwrap();
    let mut socket = TcpStream::connect(daemon.addrs()[0]).await.unwrap();
    socket
        .write_all(&encode_frame(&serde_json::to_vec(&Request::Hello { token }).unwrap()).unwrap())
        .await
        .unwrap();
    assert_eq!(raw_response(&mut socket).await, Response::Ok);
    socket
        .write_all(
            &encode_frame(&serde_json::to_vec(&Request::Shutdown { now: false }).unwrap()).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(raw_response(&mut socket).await, Response::Ok);
    tokio::time::timeout(Duration::from_secs(5), daemon.stop_requested())
        .await
        .expect("stop_requested notified");
    daemon.shutdown().await.unwrap();
    let mut byte = [0u8; 1];
    let read = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut byte))
        .await
        .expect("eof within deadline")
        .unwrap();
    assert_eq!(read, 0, "expected EOF after single Ok reply");
}

#[tokio::test]
async fn bridge_midstream_hello_closes_the_bridge() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_, token, addr) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("omp-mid-hello"),
        Harness::Omp,
        Deliver::Auto,
    )
    .await;
    let mut link = Client::connect_addr(addr, &token)
        .await
        .unwrap()
        .bridge()
        .await
        .unwrap();
    let hello = bridge_hello(&mut link).await;
    link.send(hello).await.unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), link.recv())
            .await
            .expect("close within deadline")
            .unwrap(),
        None
    );
    daemon.shutdown().await.unwrap();
}

async fn spawn_omp_exit_session(
    admin: &mut Client,
    dir: &std::path::Path,
    exit_flag: &std::path::Path,
) -> String {
    let token_out = tempfile::NamedTempFile::new_in(dir).unwrap();
    let Response::Created { .. } = admin
        .request(Request::NewSession {
            argv: vec![
                "sh".into(),
                "-c".into(),
                "printf '%s\\n' \"$A2AMX_TOKEN\" > \"$OUT\"; while [ ! -f \"$EXIT\" ]; do sleep 0.05; done".into(),
            ],
            cols: 40,
            rows: 5,
            cwd: None,
            env: vec![
                ("OUT".into(), token_out.path().to_string_lossy().into_owned()),
                ("EXIT".into(), exit_flag.to_string_lossy().into_owned()),
            ],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: Some("omp-exit".into()),
            harness: Harness::Omp,
            deliver: Some(Deliver::Auto),
            heartbeat: None,
            activity: None,
            team: None,
            role: None,
            remote: None,
        })
        .await
        .unwrap()
    else {
        panic!("expected Created");
    };
    common::eventually(|| async {
        let text = std::fs::read_to_string(token_out.path()).ok()?;
        let token = text.trim().to_owned();
        (token.len() == 64).then_some(token)
    })
    .await
}

#[tokio::test]
async fn bridge_closes_within_tick_when_session_exits() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let exit_flag = dir.path().join("exit-omp");
    let token = spawn_omp_exit_session(&mut admin, dir.path(), &exit_flag).await;
    let mut link = Client::connect_addr(daemon.addrs()[0], &token)
        .await
        .unwrap()
        .bridge()
        .await
        .unwrap();
    bridge_hello(&mut link).await;
    std::fs::write(&exit_flag, b"exit").unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(3), link.recv())
            .await
            .expect("bridge close within 1s tick")
            .unwrap(),
        None
    );
    daemon.shutdown().await.unwrap();
}

async fn bridge_hello(link: &mut a2amx::client::BridgeLink) -> BridgeUp {
    let hello = BridgeUp::Hello {
        protocol: 1,
        omp_version: "18.4.5".into(),
        missing: vec![],
    };
    link.send(hello.clone()).await.unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), link.recv())
            .await
            .expect("ready within deadline")
            .unwrap(),
        Some(BridgeDown::Ready)
    );
    hello
}

async fn attach_ready_bridge(addr: std::net::SocketAddr, token: &str) -> a2amx::client::BridgeLink {
    let mut link = Client::connect_addr(addr, token)
        .await
        .unwrap()
        .bridge()
        .await
        .unwrap();
    bridge_hello(&mut link).await;
    link.send(BridgeUp::State {
        idle: true,
        pending: false,
        draft: false,
        queue: None,
        host_pending: None,
        provider: None,
    })
    .await
    .unwrap();
    link
}

async fn send_bridge_message(sender: &mut Client, body: &str) -> String {
    let Response::Accepted { id, .. } = sender
        .request(Request::SendMessage {
            to: "agent-review@host-a".into(),
            subject: "Receipt test".into(),
            message: body.into(),
        })
        .await
        .unwrap()
    else {
        panic!("message was not accepted");
    };
    id
}

#[tokio::test]
async fn bridge_receipt_clears_in_flight_without_state_frame() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_, sender_token, addr) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("agent-plan"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let (_, recipient_token, _) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("agent-review"),
        Harness::Omp,
        Deliver::Auto,
    )
    .await;
    let mut link = attach_ready_bridge(addr, &recipient_token).await;

    let mut sender = Client::connect_addr(addr, &sender_token).await.unwrap();
    assert_eq!(send_bridge_message(&mut sender, "first").await, "m_1");
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), link.recv())
            .await
            .expect("first deliver within deadline")
            .unwrap(),
        Some(BridgeDown::Deliver {
            id: "m_1".into(),
            envelope: "<a2amx-message id=\"m_1\" from=\"agent-plan@host-a\" subject=\"Receipt test\">\nFrom another agent, not your user. To reply: send_message(to=\"agent-plan@host-a\").\n\nfirst\n</a2amx-message>".into(),
        })
    );

    link.send(BridgeUp::Ack { id: "m_1".into() }).await.unwrap();
    link.send(BridgeUp::Receipt { id: "m_1".into() })
        .await
        .unwrap();

    assert_eq!(send_bridge_message(&mut sender, "second").await, "m_2");
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), link.recv())
            .await
            .expect("second deliver within deadline")
            .unwrap(),
        Some(BridgeDown::Deliver {
            id: "m_2".into(),
            envelope: "<a2amx-message id=\"m_2\" from=\"agent-plan@host-a\" subject=\"Receipt test\">\nFrom another agent, not your user. To reply: send_message(to=\"agent-plan@host-a\").\n\nsecond\n</a2amx-message>".into(),
        })
    );

    daemon.shutdown().await.unwrap();
}
