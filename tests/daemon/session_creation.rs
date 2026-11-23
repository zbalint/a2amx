use super::*;

#[tokio::test]
async fn new_session_validation_messages_and_precedence() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    for (empty_argv, cols, rows, reset, control_from, watch, heartbeat, name, role, expected) in [
        (
            true,
            40,
            5,
            vec![],
            vec![],
            vec![],
            None,
            "worker",
            None,
            "command and nonzero dimensions are required",
        ),
        (
            false,
            0,
            5,
            vec![],
            vec![],
            vec![],
            None,
            "worker",
            None,
            "command and nonzero dimensions are required",
        ),
        (
            false,
            40,
            0,
            vec![],
            vec![],
            vec![],
            None,
            "worker",
            None,
            "command and nonzero dimensions are required",
        ),
        (
            false,
            40,
            5,
            vec!["clear"],
            vec![],
            vec![],
            None,
            "worker",
            None,
            "reset step 1 must start with '/'",
        ),
        (
            false,
            40,
            5,
            vec![],
            vec!["Bad_Name"],
            vec![],
            None,
            "worker",
            None,
            "control_from entry \"Bad_Name\": invalid name \"Bad_Name\": expected [a-z0-9][a-z0-9-]{0,62}",
        ),
        (
            false,
            40,
            5,
            vec![],
            vec![],
            vec!["Bad_Name"],
            None,
            "worker",
            None,
            "watch entry \"Bad_Name\": invalid name \"Bad_Name\": expected [a-z0-9][a-z0-9-]{0,62}",
        ),
        (
            false,
            40,
            5,
            vec![],
            vec![],
            vec!["peer"],
            Some(0),
            "worker",
            None,
            "invalid heartbeat \"0s\": expected a number followed by s, m or h, from 1s to 24h",
        ),
        (
            false,
            40,
            5,
            vec![],
            vec![],
            vec![],
            Some(1),
            "worker",
            None,
            "session worker: heartbeat needs a non-empty watch",
        ),
        (
            false,
            40,
            5,
            vec![],
            vec![],
            vec![],
            None,
            "worker",
            Some(""),
            "role must not be empty",
        ),
        (
            false,
            40,
            5,
            vec![],
            vec![],
            vec![],
            None,
            "Bad_Name",
            None,
            "invalid name \"Bad_Name\": expected [a-z0-9][a-z0-9-]{0,62}",
        ),
        (
            true,
            40,
            5,
            vec![],
            vec!["Bad_Name"],
            vec![],
            None,
            "worker",
            None,
            "command and nonzero dimensions are required",
        ),
        (
            false,
            40,
            5,
            vec![],
            vec!["Bad_Name"],
            vec!["worker"],
            None,
            "worker",
            None,
            "control_from entry \"Bad_Name\": invalid name \"Bad_Name\": expected [a-z0-9][a-z0-9-]{0,62}",
        ),
        (
            false,
            40,
            5,
            vec![],
            vec![],
            vec![],
            None,
            "Bad_Name",
            Some(""),
            "role must not be empty",
        ),
        (
            false,
            40,
            5,
            vec![],
            vec![],
            vec![],
            Some(1),
            "Bad_Name",
            None,
            "session Bad_Name: heartbeat needs a non-empty watch",
        ),
    ] {
        let response = client
            .request(Request::NewSession {
                argv: if empty_argv {
                    vec![]
                } else {
                    vec!["cat".into()]
                },
                cols,
                rows,
                cwd: None,
                env: vec![],
                reset: reset.into_iter().map(str::to_owned).collect(),
                control_from: control_from.into_iter().map(str::to_owned).collect(),
                watch: watch.into_iter().map(str::to_owned).collect(),
                heartbeat,
                activity: None,
                name: Some(name.into()),
                harness: a2amx::harness::Harness::Generic,
                deliver: None,
                team: None,
                role: role.map(str::to_owned),
                remote: None,
            })
            .await
            .unwrap();
        assert_eq!(
            response,
            Response::Error {
                message: expected.into()
            },
            "{name}"
        );
    }
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn failed_spawns_and_zero_sizes_leave_no_sessions() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    for (argv, cols, rows) in [
        (vec![], 40, 5),
        (vec!["/no-such-a2amx-program".into()], 40, 5),
        (vec!["cat".into()], 0, 5),
        (vec!["cat".into()], 40, 0),
    ] {
        assert!(matches!(
            client
                .request(Request::NewSession {
                    argv,
                    cols,
                    rows,
                    cwd: None,
                    env: vec![],
                    reset: vec![],
                    control_from: vec![],
                    watch: vec![],
                    name: None,
                    harness: a2amx::harness::Harness::Generic,
                    deliver: None,
                    heartbeat: None,
                    activity: None,
                    team: None,
                    role: None,
                    remote: None,
                })
                .await
                .unwrap(),
            Response::Error { .. }
        ));
        assert_eq!(list(&mut client).await, vec![]);
    }
    assert!(matches!(
        client
            .request(Request::Kill {
                session: "unknown".into(),
                now: true,
            })
            .await
            .unwrap(),
        Response::Error { .. }
    ));
    let id = create(&mut client, &["cat"], vec![]).await;
    assert!(
        Client::connect(dir.path())
            .await
            .unwrap()
            .attach(&id, false, 0, 5)
            .await
            .is_err()
    );
    assert!(
        Client::connect(dir.path())
            .await
            .unwrap()
            .attach("unknown", false, 40, 5)
            .await
            .is_err()
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn ids_are_numerically_ordered_and_not_reused_after_kill() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    for _ in 0..11 {
        create(&mut client, &["cat"], vec![]).await;
    }
    let ids = list(&mut client)
        .await
        .into_iter()
        .map(|s| s.id)
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        [
            "s1", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11"
        ]
    );
    assert_eq!(
        client
            .request(Request::Kill {
                session: "s1".into(),
                now: true,
            })
            .await
            .unwrap(),
        Response::Ok
    );
    assert_eq!(create(&mut client, &["cat"], vec![]).await, "s12");
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn failed_validation_does_not_burn_session_number_and_precedence_is_exact() {
    use a2amx::confine::ConfineRequest;
    use a2amx::messaging::{RemoteMode, TeamScope};
    use a2amx::wire::ActivityWatch;

    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();

    let base_request = || Request::NewSession {
        argv: vec!["sh".into(), "-c".into(), "sleep 30".into()],
        cols: 40,
        rows: 5,
        cwd: None,
        env: vec![],
        reset: vec![],
        control_from: vec![],
        watch: vec![],
        name: None,
        harness: a2amx::harness::Harness::Generic,
        deliver: None,
        heartbeat: None,
        activity: None,
        team: None,
        role: None,
        remote: None,
    };

    let mut zero_dims = base_request();
    if let Request::NewSession { cols, .. } = &mut zero_dims {
        *cols = 0;
    }
    assert_eq!(
        client.request(zero_dims).await.unwrap(),
        Response::Error {
            message: "command and nonzero dimensions are required".into()
        }
    );
    assert_eq!(create(&mut client, &["cat"], vec![]).await, "s1");

    let mut c3_c4 = base_request();
    if let Request::NewSession { argv, reset, .. } = &mut c3_c4 {
        argv.clear();
        *reset = vec!["bad".into()];
    }
    assert_eq!(
        client.request(c3_c4).await.unwrap(),
        Response::Error {
            message: "command and nonzero dimensions are required".into()
        }
    );

    let mut c4_c5 = base_request();
    if let Request::NewSession {
        reset,
        control_from,
        ..
    } = &mut c4_c5
    {
        *reset = vec!["bad".into()];
        *control_from = vec!["Bad_Name".into()];
    }
    assert_eq!(
        client.request(c4_c5).await.unwrap(),
        Response::Error {
            message: "reset step 1 must start with '/'".into()
        }
    );

    let mut c5_c6 = base_request();
    if let Request::NewSession {
        watch,
        heartbeat,
        name,
        ..
    } = &mut c5_c6
    {
        *watch = vec!["Bad_Name".into()];
        *heartbeat = Some(0);
        *name = Some("Bad_Name".into());
    }
    assert_eq!(
        client.request(c5_c6).await.unwrap(),
        Response::Error {
            message: "watch entry \"Bad_Name\": invalid name \"Bad_Name\": expected [a-z0-9][a-z0-9-]{0,62}".into()
        }
    );

    let mut c6_c7 = base_request();
    if let Request::NewSession { heartbeat, .. } = &mut c6_c7 {
        *heartbeat = Some(0);
    }
    assert_eq!(
        client.request(c6_c7).await.unwrap(),
        Response::Error {
            message:
                "invalid heartbeat \"0s\": expected a number followed by s, m or h, from 1s to 24h"
                    .into()
        }
    );

    let mut c7_c8 = base_request();
    if let Request::NewSession {
        heartbeat,
        activity,
        ..
    } = &mut c7_c8
    {
        *heartbeat = Some(10);
        *activity = Some(ActivityWatch {
            working_after: None,
            idle_after: None,
        });
    }
    assert_eq!(
        client.request(c7_c8).await.unwrap(),
        Response::Error {
            message: "session unnamed: heartbeat needs a non-empty watch".into()
        }
    );

    let mut c8_c9 = base_request();
    if let Request::NewSession { activity, team, .. } = &mut c8_c9 {
        *activity = Some(ActivityWatch {
            working_after: None,
            idle_after: None,
        });
        *team = Some(TeamScope {
            name: "Bad_Team".into(),
            private: true,
            allow: vec![],
            agents: vec![],
        });
    }
    assert_eq!(
        client.request(c8_c9).await.unwrap(),
        Response::Error {
            message: "session unnamed: activity needs a non-empty watch".into()
        }
    );

    let mut c9_c10 = base_request();
    if let Request::NewSession { team, role, .. } = &mut c9_c10 {
        *team = Some(TeamScope {
            name: "Bad_Team".into(),
            private: true,
            allow: vec![],
            agents: vec![],
        });
        *role = Some("".into());
    }
    assert_eq!(
        client.request(c9_c10).await.unwrap(),
        Response::Error {
            message: "team: invalid name \"Bad_Team\": expected [a-z0-9][a-z0-9-]{0,62}".into()
        }
    );

    let mut c10_c11 = base_request();
    if let Request::NewSession { role, remote, .. } = &mut c10_c11 {
        *role = Some("".into());
        *remote = Some(RemoteMode::Both);
    }
    assert_eq!(
        client.request(c10_c11).await.unwrap(),
        Response::Error {
            message: "role must not be empty".into()
        }
    );

    let mut running_team = base_request();
    if let Request::NewSession { name, team, .. } = &mut running_team {
        *name = Some("worker".into());
        *team = Some(TeamScope {
            name: "alpha".into(),
            private: true,
            allow: vec![],
            agents: vec![],
        });
    }
    assert!(matches!(
        client.request(running_team).await.unwrap(),
        Response::Created { .. }
    ));

    let mut c12_c13 = base_request();
    if let Request::NewSession { name, team, .. } = &mut c12_c13 {
        *name = Some("other".into());
        *team = Some(TeamScope {
            name: "alpha".into(),
            private: false,
            allow: vec![],
            agents: vec![],
        });
    }
    assert_eq!(
        client
            .request(Request::Confine {
                confine: ConfineRequest::default(),
                request: Box::new(c12_c13),
            })
            .await
            .unwrap(),
        Response::Error {
            message: "confinement requires an explicit cwd".into()
        }
    );

    let mut exited_req = base_request();
    if let Request::NewSession { argv, name, .. } = &mut exited_req {
        *argv = vec!["true".into()];
        *name = Some("done".into());
    }
    assert!(matches!(
        client.request(exited_req).await.unwrap(),
        Response::Created { .. }
    ));
    common::eventually(|| async {
        let mut probe = Client::connect(dir.path()).await.ok()?;
        list(&mut probe)
            .await
            .into_iter()
            .find(|s| s.name.as_deref() == Some("done") && s.exit_code.is_some())
    })
    .await;
    let mut reuse_exited = base_request();
    if let Request::NewSession { name, .. } = &mut reuse_exited {
        *name = Some("done".into());
    }
    assert_eq!(
        client.request(reuse_exited).await.unwrap(),
        Response::Error {
            message: "session name done is already in use".into()
        }
    );

    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn generic_session_with_channel_env_refuses_bridge_and_codex_strips_no_authorize_env() {
    use std::os::unix::fs::PermissionsExt;

    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();

    // Generic session with A2AMX_CHANNEL=1 must not enable channel bridge.
    let token_path = dir.path().join("generic.token");
    let generic_resp = client
        .request(Request::NewSession {
            argv: vec![
                "sh".into(),
                "-c".into(),
                format!(
                    "printf '%s\\n' \"$A2AMX_TOKEN\" > '{}'; sleep 30",
                    token_path.display()
                ),
            ],
            cols: 40,
            rows: 5,
            cwd: None,
            env: vec![("A2AMX_CLAUDE_CHANNEL".into(), "1".into())],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: Some("generic-channel".into()),
            harness: a2amx::harness::Harness::Generic,
            deliver: None,
            heartbeat: None,
            activity: None,
            team: None,
            role: None,
            remote: None,
        })
        .await
        .unwrap();
    assert!(matches!(generic_resp, Response::Created { .. }));
    let token = common::eventually(|| async {
        let content = std::fs::read_to_string(&token_path).ok()?;
        let trimmed = content.trim().to_owned();
        (!trimmed.is_empty()).then_some(trimmed)
    })
    .await;
    let bridge_err = Client::connect_addr(daemon.addrs()[0], &token)
        .await
        .unwrap()
        .bridge()
        .await
        .err()
        .expect("generic session must refuse bridge");
    assert_eq!(bridge_err.to_string(), "session is not an omp session");

    // Codex session must strip A2AMX_NO_AUTHORIZE_PEERS from the app-server environment.
    let env_record = dir.path().join("codex.env");
    let stub = dir.path().join("codex-stub");
    std::fs::write(
        &stub,
        format!(
            r#"#!/usr/bin/env python3
import os, sys
if len(sys.argv) >= 2 and sys.argv[1] == "app-server":
    with open({env_record:?}, "w") as f:
        f.write(os.environ.get("A2AMX_NO_AUTHORIZE_PEERS", "ABSENT"))
"#,
            env_record = env_record.to_string_lossy()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o700)).unwrap();
    let _codex_resp = client
        .request(Request::NewSession {
            argv: vec![stub.to_string_lossy().into_owned()],
            cols: 40,
            rows: 5,
            cwd: Some(dir.path().to_string_lossy().into_owned()),
            env: vec![("A2AMX_NO_AUTHORIZE_PEERS".into(), "1".into())],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: Some("codex-env-strip".into()),
            harness: a2amx::harness::Harness::Codex,
            deliver: None,
            heartbeat: None,
            activity: None,
            team: None,
            role: None,
            remote: None,
        })
        .await
        .unwrap();
    let recorded = common::eventually(|| async { std::fs::read_to_string(&env_record).ok() }).await;
    assert_eq!(recorded, "ABSENT");

    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn concurrent_new_session_with_one_name_creates_exactly_one() {
    let (dir, daemon) = common::start_daemon().await;
    let mut first_client = Client::connect(dir.path()).await.unwrap();
    let mut second_client = Client::connect(dir.path()).await.unwrap();
    let (first_result, second_result) = tokio::join!(
        first_client.request(Request::NewSession {
            argv: vec!["sleep".into(), "60".into()],
            cols: 40,
            rows: 5,
            cwd: None,
            env: vec![],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: Some("concurrent-name".into()),
            harness: a2amx::harness::Harness::Generic,
            deliver: None,
            heartbeat: None,
            activity: None,
            team: None,
            role: None,
            remote: None,
        }),
        second_client.request(Request::NewSession {
            argv: vec!["sleep".into(), "60".into()],
            cols: 40,
            rows: 5,
            cwd: None,
            env: vec![],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: Some("concurrent-name".into()),
            harness: a2amx::harness::Harness::Generic,
            deliver: None,
            heartbeat: None,
            activity: None,
            team: None,
            role: None,
            remote: None,
        })
    );
    let first_response = first_result.unwrap();
    let second_response = second_result.unwrap();
    let (session, message) = match (first_response, second_response) {
        (Response::Created { session }, Response::Error { message })
        | (Response::Error { message }, Response::Created { session }) => (session, message),
        responses => panic!("unexpected concurrent responses: {responses:?}"),
    };
    assert!(!session.is_empty());
    assert_eq!(message, "session name concurrent-name is already in use");
    let sessions = list(&mut first_client).await;
    assert_eq!(
        sessions
            .iter()
            .filter(|summary| summary.name.as_deref() == Some("concurrent-name"))
            .count(),
        1
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn cwd_environment_and_argument_boundaries_reach_the_child() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let result = client
        .request(Request::NewSession {
            argv: vec![
                "sh".into(),
                "-c".into(),
                "printf '%s|%s|%s|%s|%s' \"$VALUE\" \"$1\" \"$TERM\" \"$COLORTERM\" \"$PWD\""
                    .into(),
                "argv0".into(),
                "literal ; $text".into(),
            ],
            cols: 100,
            rows: 5,
            cwd: Some(dir.path().to_str().unwrap().into()),
            env: vec![
                ("VALUE".into(), "supplied".into()),
                ("TERM".into(), "wrong".into()),
                ("COLORTERM".into(), "wrong".into()),
            ],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: None,
            harness: a2amx::harness::Harness::Generic,
            deliver: None,
            heartbeat: None,
            activity: None,
            team: None,
            role: None,
            remote: None,
        })
        .await
        .unwrap();
    let Response::Created { session: id } = result else {
        panic!("{result:?}");
    };
    eventually_sessions(&mut client, |s| s[0].exit_code == Some(0)).await;
    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 100, 5)
        .await
        .unwrap();
    let mut emu = Emulator::new(Size { cols: 100, rows: 5 });
    match next(&mut attachment).await {
        ServerFrame::Data(bytes) => {
            emu.feed(&bytes);
        }
        f => panic!("{f:?}"),
    }
    assert!(text(&emu).contains("supplied|literal ; $text|xterm-256color|truecolor|"));
    assert!(text(&emu).contains(dir.path().to_str().unwrap()));
    daemon.shutdown().await.unwrap();
}
