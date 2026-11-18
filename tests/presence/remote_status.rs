use super::*;

async fn remote_status_until(
    attachment: &mut a2amx::client::Attachment,
    predicate: impl Fn(&RemoteStatus) -> bool,
) -> RemoteStatus {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match attachment.recv().await.unwrap() {
                Some(ServerFrame::Status(info)) => {
                    if let Some(remote) = info.remote
                        && predicate(&remote)
                    {
                        return remote;
                    }
                }
                Some(ServerFrame::Data(_)) => {}
                Some(frame) => panic!("unexpected attachment frame: {frame:?}"),
                None => panic!("attachment closed while waiting for remote status"),
            }
        }
    })
    .await
    .expect("remote status arrives within poll window")
}

async fn status_once(attachment: &mut a2amx::client::Attachment) -> a2amx::wire::StatusInfo {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match attachment.recv().await.unwrap() {
                Some(ServerFrame::Status(info)) => return info,
                Some(ServerFrame::Data(_)) => {}
                Some(frame) => panic!("unexpected attachment frame: {frame:?}"),
                None => panic!("attachment closed before initial status"),
            }
        }
    })
    .await
    .expect("initial status arrives within poll window")
}

#[tokio::test]
async fn attach_status_reports_remote_lease_and_gateway_lifecycle() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let team = team_scope("status-remote");
    let (session, _) = create_confined_with_token(
        &mut admin,
        fixture.path(),
        "status-remote",
        Some(RemoteMode::Both),
        Some(team.clone()),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach_status(&session, false, 40, 10, 0)
        .await
        .unwrap();
    assert_eq!(
        remote_status_until(&mut attachment, |remote| remote.lease == RemoteLease::None).await,
        RemoteStatus {
            lease: RemoteLease::None,
            gateway: false,
        }
    );

    let _issued_only = common::issue_code(&mut admin, &team.name).await;
    drop(attachment);
    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach_status(&session, false, 40, 10, 0)
        .await
        .unwrap();
    assert_eq!(
        status_once(&mut attachment).await.remote.unwrap().lease,
        RemoteLease::None
    );

    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, &team.name).await;
    assert_eq!(
        remote_status_until(&mut attachment, |remote| {
            remote.lease == RemoteLease::Active && remote.gateway
        })
        .await,
        RemoteStatus {
            lease: RemoteLease::Active,
            gateway: true,
        }
    );
    drop(gateway);
    assert_eq!(
        remote_status_until(&mut attachment, |remote| {
            remote.lease == RemoteLease::Active && !remote.gateway
        })
        .await,
        RemoteStatus {
            lease: RemoteLease::Active,
            gateway: false,
        }
    );
    let gateway = common::gateway(dir.path(), daemon.addrs()[0]).await;
    assert_eq!(
        remote_status_until(&mut attachment, |remote| {
            remote.lease == RemoteLease::Active && remote.gateway
        })
        .await,
        RemoteStatus {
            lease: RemoteLease::Active,
            gateway: true,
        }
    );
    drop(gateway);
    drop(attachment);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn attach_status_reports_expiry_and_nonmember_and_nonremote_states() {
    let Some((dir, daemon)) = start_confined_daemon_with_limits(common::limits_with(1, 60)).await
    else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let team = team_scope("status-expiry");
    let (session, _) = create_confined_with_token(
        &mut admin,
        fixture.path(),
        "status-expiry",
        Some(RemoteMode::Send),
        Some(team.clone()),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, &team.name).await;
    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach_status(&session, false, 40, 10, 0)
        .await
        .unwrap();
    let _ = remote_status_until(&mut attachment, |remote| {
        remote.lease == RemoteLease::Active
    })
    .await;
    let ended =
        remote_status_until(&mut attachment, |remote| remote.lease == RemoteLease::Ended).await;
    assert_eq!(ended.lease, RemoteLease::Ended);
    drop(gateway);
    drop(attachment);

    let fixture_late = tempfile::tempdir().unwrap();
    let (late, _) = create_confined_with_token(
        &mut admin,
        fixture_late.path(),
        "status-late",
        Some(RemoteMode::Send),
        Some(team),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut late_attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach_status(&late, false, 40, 10, 0)
        .await
        .unwrap();
    let late_status = status_once(&mut late_attachment).await;
    assert_eq!(late_status.remote.unwrap().lease, RemoteLease::None);

    let fixture_local = tempfile::tempdir().unwrap();
    let (local, _) = create_confined_with_token(
        &mut admin,
        fixture_local.path(),
        "status-local",
        None,
        None,
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut local_attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach_status(&local, false, 40, 10, 0)
        .await
        .unwrap();
    assert!(status_once(&mut local_attachment).await.remote.is_none());
    daemon.shutdown().await.unwrap();
}
