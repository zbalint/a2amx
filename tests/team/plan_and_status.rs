use super::*;

fn summary(name: &str, id: &str, exit_code: Option<i32>) -> SessionSummary {
    SessionSummary {
        id: id.into(),
        name: Some(name.into()),
        argv: vec!["cat".into()],
        cols: 80,
        rows: 24,
        attached: false,
        viewer: false,
        exit_code,
        pending: 0,
        address: String::new(),
        held: false,
        hold_reason: None,
        quota: None,
        harness: Default::default(),
        confine: None,
        cwd: None,
        activity: None,
        team: None,
        uptime_secs: None,
        activity_secs: None,
        starting: false,
    }
}
#[test]
fn reset_targets_preserve_order_and_protect_attached_sessions() {
    use team::{ResetState, ResetTarget};

    let names = ["a", "b", "c", "d", "e"].map(String::from);
    let mut attached = summary("c", "c-id", None);
    attached.attached = true;
    let existing = [
        attached,
        summary("b", "b-id", Some(0)),
        summary("a", "a-id", None),
        summary("d", "d-id", None),
    ];
    let except = ["d", "unselected"].map(String::from);
    assert_eq!(
        team::reset_targets(
            &names,
            team::ResetOptions {
                explicit: false,
                except: &except,
                include_attached: false,
            },
            &existing,
        ),
        vec![
            ResetTarget {
                name: "a".into(),
                state: ResetState::Reset { id: "a-id".into() },
            },
            ResetTarget {
                name: "b".into(),
                state: ResetState::Exited { id: "b-id".into() },
            },
            ResetTarget {
                name: "c".into(),
                state: ResetState::SkippedAttached { id: "c-id".into() },
            },
            ResetTarget {
                name: "e".into(),
                state: ResetState::Missing,
            },
        ]
    );
    for (explicit, include_attached) in [(false, true), (true, false), (true, true)] {
        assert_eq!(
            team::reset_targets(
                &names,
                team::ResetOptions {
                    explicit,
                    except: &except,
                    include_attached,
                },
                &existing,
            ),
            vec![
                ResetTarget {
                    name: "a".into(),
                    state: ResetState::Reset { id: "a-id".into() },
                },
                ResetTarget {
                    name: "b".into(),
                    state: ResetState::Exited { id: "b-id".into() },
                },
                ResetTarget {
                    name: "c".into(),
                    state: ResetState::Reset { id: "c-id".into() },
                },
                ResetTarget {
                    name: "e".into(),
                    state: ResetState::Missing,
                },
            ]
        );
    }
    let mut attached_exited = summary("a", "a-id", Some(1));
    attached_exited.attached = true;
    assert_eq!(
        team::reset_targets(
            &["a".into()],
            team::ResetOptions {
                explicit: false,
                except: &[],
                include_attached: false,
            },
            &[attached_exited],
        ),
        vec![ResetTarget {
            name: "a".into(),
            state: ResetState::Exited { id: "a-id".into() },
        }]
    );
}
#[test]
fn plan_preserves_wanted_order_and_preflights_every_exited_conflict() {
    let wanted = team::flag_sessions(&["architect=cat".into(), "developer=cat".into()]).unwrap();
    assert_eq!(
        team::plan(&wanted, &[summary("developer", "s2", None)]).unwrap(),
        [
            team::Action::Start(Box::new(wanted[0].clone())),
            team::Action::AlreadyRunning {
                name: "developer".into(),
                id: "s2".into()
            },
        ],
    );
    assert_eq!(
        team::plan(&wanted, &[summary("developer", "s2", Some(0))]).unwrap_err(),
        [team::Conflict {
            name: "developer".into(),
            id: "s2".into(),
            reason: team::ConflictReason::Exited,
        }],
    );
    assert_eq!(
        team::plan(
            &wanted,
            &[
                summary("developer", "s2", Some(1)),
                summary("architect", "s1", Some(0))
            ]
        )
        .unwrap_err(),
        [
            team::Conflict {
                name: "architect".into(),
                id: "s1".into(),
                reason: team::ConflictReason::Exited,
            },
            team::Conflict {
                name: "developer".into(),
                id: "s2".into(),
                reason: team::ConflictReason::Exited,
            },
        ],
    );
}

#[test]
fn plan_reports_running_team_setting_mismatch() {
    let wanted = team::parse(
        "team = \"demo\"\n[[session]]\nname = \"worker\"\ncommand = [\"cat\"]",
        std::path::Path::new("/work/demo"),
        false,
    )
    .unwrap();
    let mut existing = summary("demo-worker", "s1", None);
    existing.team = Some(a2amx::messaging::TeamScope {
        name: "demo".into(),
        private: false,
        allow: Vec::new(),
        agents: Vec::new(),
    });
    let conflict = team::plan(&wanted, &[existing]).unwrap_err();
    assert_eq!(
        conflict,
        [team::Conflict {
            name: "demo-worker".into(),
            id: "s1".into(),
            reason: team::ConflictReason::TeamMismatch,
        }]
    );
}
#[test]
fn plan_notices_agent_scope_mismatch_and_accepts_matching_scope() {
    let wanted = team::parse("team = \"a2amx\"\n[[session]]\nname = \"architect\"\ncommand = [\"cat\"]\nallow_agents = [\"saltmdb/architect\"]", std::path::Path::new("/work/demo"), false)
    .unwrap();
    let mut existing = summary("a2amx-architect", "s1", None);
    existing.team = Some(a2amx::messaging::TeamScope {
        name: "a2amx".into(),
        private: true,
        allow: Vec::new(),
        agents: Vec::new(),
    });
    assert_eq!(
        team::plan(&wanted, &[existing.clone()]),
        Err(vec![team::Conflict {
            name: "a2amx-architect".into(),
            id: "s1".into(),
            reason: team::ConflictReason::TeamMismatch,
        }])
    );
    existing.team.as_mut().unwrap().agents = vec!["saltmdb-architect".into()];
    assert_eq!(
        team::plan(&wanted, &[existing]),
        Ok(vec![team::Action::AlreadyRunning {
            name: "a2amx-architect".into(),
            id: "s1".into(),
        }])
    );
}

#[test]
fn inspect_preserves_order_and_plan_reports_the_same_conflicts() {
    let wanted = team::flag_sessions(&[
        "a=cat".into(),
        "b=cat".into(),
        "c=cat".into(),
        "d=cat".into(),
    ])
    .unwrap();
    let mut mismatch = summary("c", "c-id", None);
    mismatch.team = Some(a2amx::messaging::TeamScope {
        name: "different".into(),
        private: true,
        allow: Vec::new(),
        agents: Vec::new(),
    });
    let existing = [
        mismatch,
        summary("b", "b-id", Some(0)),
        summary("a", "a-id", None),
    ];
    assert_eq!(
        team::inspect(&wanted, &existing),
        [
            team::Entry {
                name: "a".into(),
                state: team::EntryState::Running { id: "a-id".into() },
            },
            team::Entry {
                name: "b".into(),
                state: team::EntryState::Conflict {
                    id: "b-id".into(),
                    reason: team::ConflictReason::Exited,
                },
            },
            team::Entry {
                name: "c".into(),
                state: team::EntryState::Conflict {
                    id: "c-id".into(),
                    reason: team::ConflictReason::TeamMismatch,
                },
            },
            team::Entry {
                name: "d".into(),
                state: team::EntryState::Missing,
            },
        ]
    );
    assert_eq!(
        team::plan(&wanted, &existing).unwrap_err(),
        [
            team::Conflict {
                name: "b".into(),
                id: "b-id".into(),
                reason: team::ConflictReason::Exited,
            },
            team::Conflict {
                name: "c".into(),
                id: "c-id".into(),
                reason: team::ConflictReason::TeamMismatch,
            },
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_reports_running_and_missing_without_changing_sessions() {
    let (dir, _daemon) = common::start_daemon().await;
    std::fs::write(
        dir.path().join("up.toml"),
        r#"
team = "config"
[[session]]
name = "running"
command = ["sh", "-c", "sleep 30"]
"#,
    )
    .unwrap();
    let up = run_binary(dir.path(), &["team", "up", "--detach", "--file", "up.toml"]).unwrap();
    assert!(
        up.status.success(),
        "{}",
        String::from_utf8_lossy(&up.stderr)
    );
    std::fs::write(
        dir.path().join("config/team.toml"),
        r#"
team = "config"
[[session]]
name = "running"
command = ["sh", "-c", "sleep 30"]
[[session]]
name = "missing"
command = ["cat"]
"#,
    )
    .unwrap();
    let before = run_binary(dir.path(), &["list"]).unwrap().stdout;

    let status = run_binary(
        dir.path(),
        &["team", "status", "--file", "config/team.toml"],
    )
    .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert!(status.stderr.is_empty());
    let stdout = String::from_utf8(status.stdout).unwrap();
    assert!(stdout.contains("NAME TEAM STATE CONFINE NOTE\n"));
    assert!(stdout.contains("config-running config (private) running off s1\n"));
    assert!(stdout.contains("config-missing config (private) missing  -\n"));

    let after = run_binary(dir.path(), &["list"]).unwrap().stdout;
    assert_eq!(after, before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_keeps_generic_running_note_as_the_session_id() {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    common::new_team_agent(
        &mut admin,
        dir.path(),
        Some("config-plain"),
        Harness::Generic,
        Deliver::Hold,
        Some(a2amx::messaging::TeamScope {
            name: "config".into(),
            private: true,
            allow: Vec::new(),
            agents: Vec::new(),
        }),
        &[],
        &[],
    )
    .await;
    std::fs::write(
        dir.path().join("config/team.toml"),
        r#"
team = "config"
[[session]]
name = "plain"
command = ["cat"]
"#,
    )
    .unwrap();

    let status = run_binary(
        dir.path(),
        &["team", "status", "--file", "config/team.toml"],
    )
    .unwrap();
    assert_eq!(status.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(status.stdout).unwrap(),
        "NAME TEAM STATE CONFINE NOTE\nconfig-plain config (private) running off s1\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_marks_a_running_claude_session_that_is_not_ready() {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    common::new_team_agent(
        &mut admin,
        dir.path(),
        Some("config-worker"),
        Harness::Claude,
        Deliver::Hold,
        Some(a2amx::messaging::TeamScope {
            name: "config".into(),
            private: true,
            allow: Vec::new(),
            agents: Vec::new(),
        }),
        &[],
        &[],
    )
    .await;
    std::fs::write(
        dir.path().join("config/team.toml"),
        r#"
team = "config"
[[session]]
name = "worker"
command = ["claude"]
"#,
    )
    .unwrap();

    let status = run_binary(
        dir.path(),
        &["team", "status", "--file", "config/team.toml"],
    )
    .unwrap();
    assert_eq!(status.status.code(), Some(0));
    let stdout = String::from_utf8(status.stdout).unwrap();
    assert!(
        stdout.contains("config-worker config (private) running off "),
        "{stdout}"
    );
    assert!(stdout.contains("(not ready: starting)"), "{stdout}");
}

async fn start_fake_claude(dir: &Path) -> anyhow::Result<()> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_claude.sh");
    let mut client = Client::connect(dir).await?;
    let response = client
        .request(Request::NewSession {
            argv: vec!["sh".into(), fixture.to_string_lossy().into_owned()],
            cols: 80,
            rows: 24,
            cwd: None,
            env: vec![
                ("OUT".into(), dir.join("out").to_string_lossy().into_owned()),
                ("PASTE_LEN".into(), "1".into()),
                ("MODE".into(), "ready".into()),
            ],
            reset: Vec::new(),
            control_from: Vec::new(),
            watch: Vec::new(),
            name: Some("config-ready".into()),
            harness: Harness::Claude,
            deliver: Some(Deliver::Hold),
            team: Some(a2amx::messaging::TeamScope {
                name: "config".into(),
                private: true,
                allow: Vec::new(),
                agents: Vec::new(),
            }),
            role: None,
            remote: None,
            heartbeat: None,
            activity: None,
        })
        .await?;
    match response {
        Response::Created { .. } => Ok(()),
        other => anyhow::bail!("fake Claude start failed: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_leaves_a_ready_claude_session_unmarked() {
    let (dir, _daemon) = common::start_daemon().await;
    start_fake_claude(dir.path())
        .await
        .expect("start team-scoped fake Claude");
    std::fs::write(
        dir.path().join("config/team.toml"),
        "team = \"config\"\n[[session]]\nname = \"ready\"\ncommand = [\"claude\"]\n",
    )
    .unwrap();

    // The fixture draws Claude's ready screen after it starts; each status request samples
    // the screen, so the first rows may still say "starting".
    let rows = common::eventually(|| async {
        let status = run_binary(
            dir.path(),
            &["team", "status", "--file", "config/team.toml"],
        )
        .unwrap();
        let rows = String::from_utf8(status.stdout).unwrap();
        (!rows.contains("not ready")).then_some(rows)
    })
    .await;
    assert!(
        rows.contains("config-ready config (private) running off "),
        "{rows}"
    );
    assert!(!rows.contains("(not ready:"), "{rows}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_reports_conflicts_without_a_generic_error_line() {
    let (dir, _daemon) = common::start_daemon().await;
    assert!(
        run_binary(
            dir.path(),
            &[
                "new",
                "--detach",
                "--name",
                "config-exited",
                "--",
                "sh",
                "-c",
                "exit 0"
            ],
        )
        .unwrap()
        .status
        .success()
    );
    common::eventually(|| async {
        let output = run_binary(dir.path(), &["list"]).unwrap();
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("exited(0)")
            .then_some(())
    })
    .await;
    std::fs::write(
        dir.path().join("exited.toml"),
        "team = \"config\"\n[[session]]\nname = 'exited'\ncommand = ['cat']",
    )
    .unwrap();
    let exited = run_binary(dir.path(), &["team", "status", "--file", "exited.toml"]).unwrap();
    assert_eq!(exited.status.code(), Some(1));
    let exited_stdout = String::from_utf8(exited.stdout).unwrap();
    assert!(
        exited_stdout.contains("config-exited config (private) exited"),
        "{exited_stdout}"
    );
    assert!(
        exited_stdout.contains("run a2amx kill config-exited first"),
        "{exited_stdout}"
    );
    assert!(
        !String::from_utf8_lossy(&exited.stderr).starts_with("a2amx:"),
        "{:?}",
        exited.stderr
    );

    let mismatch_team = r#"team = "x"
[[session]]
name = "same"
command = ["sleep", "30"]
"#;
    std::fs::write(dir.path().join("mismatch-up.toml"), mismatch_team).unwrap();
    let up = run_binary(
        dir.path(),
        &["team", "up", "--detach", "--file", "mismatch-up.toml"],
    )
    .unwrap();
    assert!(
        up.status.success(),
        "{}",
        String::from_utf8_lossy(&up.stderr)
    );
    std::fs::write(
        dir.path().join("mismatch-status.toml"),
        "team = 'x'\nprivate = false\n[[session]]\nname = 'same'\ncommand = ['cat']",
    )
    .unwrap();
    let mismatch = run_binary(
        dir.path(),
        &["team", "status", "--file", "mismatch-status.toml"],
    )
    .unwrap();
    assert_eq!(mismatch.status.code(), Some(1));
    let mismatch_stdout = String::from_utf8(mismatch.stdout).unwrap();
    assert!(mismatch_stdout.contains("same x team-mismatch"));
    assert!(mismatch_stdout.contains("run a2amx team down first"));
    assert!(!String::from_utf8_lossy(&mismatch.stderr).starts_with("a2amx:"));
}
