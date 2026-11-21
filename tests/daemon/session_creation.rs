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
