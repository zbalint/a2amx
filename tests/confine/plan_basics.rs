use super::*;

#[test]
fn plan_skips_missing_and_unhides_existing_home_secrets_read_only() {
    let fixture = Fixture::new();
    let request = ConfineRequest {
        unhide: vec![
            "~/.aws".into(),
            "~/.ssh/config".into(),
            fixture
                .cwd
                .join("missing-path")
                .to_string_lossy()
                .into_owned(),
        ],
        team_file: Some(
            fixture
                .cwd
                .join("missing-team.toml")
                .to_string_lossy()
                .into_owned(),
        ),
        ..ConfineRequest::default()
    };
    let plan =
        confine::plan(&request, Harness::Generic, &fixture.cwd, &fixture.host()).expect("plan");

    assert!(plan.writable.iter().any(|path| path == &fixture.cwd));
    assert!(
        plan.writable
            .iter()
            .all(|path| !path.starts_with(&fixture.state) && !path.starts_with(&fixture.runtime))
    );
    assert!(
        plan.protected
            .iter()
            .all(|path| !path.starts_with(&fixture.state))
    );
    assert!(
        plan.skipped_paths()
            .iter()
            .any(|path| path.ends_with("missing-path"))
    );
    assert!(
        plan.skipped_paths()
            .iter()
            .any(|path| path.ends_with("missing-team.toml"))
    );
    assert!(
        plan.hidden_paths()
            .iter()
            .any(|path| path == &fixture.home.join(".ssh"))
    );
    let ssh_config = fixture
        .home
        .join(".ssh/config")
        .to_string_lossy()
        .into_owned();
    let wrapped = plan
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("wrap");
    assert!(
        wrapped
            .windows(3)
            .any(|window| window == ["--ro-bind", ssh_config.as_str(), ssh_config.as_str()])
    );
}
#[test]
fn plan_refuses_relative_operator_entries() {
    let fixture = Fixture::new();
    let cases = [
        (
            "write",
            ConfineRequest {
                write: vec!["relative/dir".into()],
                ..ConfineRequest::default()
            },
        ),
        (
            "hide",
            ConfineRequest {
                hide: vec!["relative/dir".into()],
                ..ConfineRequest::default()
            },
        ),
        (
            "unhide",
            ConfineRequest {
                unhide: vec!["relative/dir".into()],
                ..ConfineRequest::default()
            },
        ),
    ];
    for (option, request) in cases {
        let error = match confine::plan(&request, Harness::Generic, &fixture.cwd, &fixture.host()) {
            Ok(_) => panic!("{option} relative entry must be rejected"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("\"relative/dir\" must be an absolute path or start with ~/"),
            "{option}: {error:#}"
        );
    }
    for (entry, expected) in [
        (
            "$HOME/x",
            "write entry \"$HOME/x\": environment variables are not expanded; write the path out, or use ~/ for your home",
        ),
        (
            "~bob/x",
            "write entry \"~bob/x\": only ~ and ~/... are understood, not another user's home",
        ),
    ] {
        let error = match confine::plan(
            &ConfineRequest {
                write: vec![entry.into()],
                ..ConfineRequest::default()
            },
            Harness::Generic,
            &fixture.cwd,
            &fixture.host(),
        ) {
            Ok(_) => panic!("wire syntax entry {entry:?} must be rejected"),
            Err(error) => error,
        };
        assert_eq!(error.to_string(), expected);
    }

    let request = ConfineRequest {
        team_file: Some("relative/team.toml".into()),
        ..ConfineRequest::default()
    };
    let error = match confine::plan(&request, Harness::Generic, &fixture.cwd, &fixture.host()) {
        Ok(_) => panic!("relative team file must be rejected"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("\"relative/team.toml\" must be an absolute path or start with ~/"),
        "{error:#}"
    );
}

#[test]
fn plan_reports_unresolved_operator_paths() {
    if rustix::process::geteuid().is_root() {
        return;
    }
    let fixture = Fixture::new();
    let missing = fixture.cwd.join("missing-write");
    let plan = confine::plan(
        &ConfineRequest {
            write: vec![missing.to_string_lossy().into_owned()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("missing operator write plan");
    assert_eq!(plan.unresolved.len(), 1);
    assert_eq!(plan.unresolved[0].option, "write");
    assert_eq!(plan.unresolved[0].path, missing);
    assert_eq!(plan.unresolved[0].reason, "not found");

    let denied_dir = fixture._root.path().join("denied");
    std::fs::create_dir(&denied_dir).expect("denied directory");
    let mut permissions = std::fs::metadata(&denied_dir)
        .expect("denied metadata")
        .permissions();
    permissions.set_mode(0o000);
    std::fs::set_permissions(&denied_dir, permissions).expect("deny directory");
    let denied = denied_dir.join("hidden");
    let plan = confine::plan(
        &ConfineRequest {
            hide: vec![denied.to_string_lossy().into_owned()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    );
    let mut permissions = std::fs::metadata(&denied_dir)
        .expect("denied metadata after plan")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&denied_dir, permissions).expect("restore directory");
    let plan = plan.expect("permission-denied operator hide plan");
    assert_eq!(plan.unresolved.len(), 1);
    assert_eq!(plan.unresolved[0].option, "hide");
    assert_eq!(plan.unresolved[0].path, denied);
    assert_eq!(plan.unresolved[0].reason, "permission denied");

    let plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("default hide plan");
    assert!(plan.unresolved.is_empty());

    let missing_team = fixture.cwd.join("missing-team.toml");
    let plan = confine::plan(
        &ConfineRequest {
            team_file: Some(missing_team.to_string_lossy().into_owned()),
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("missing team file plan");
    assert!(plan.unresolved.is_empty());

    let missing_cwd = fixture.cwd.join("missing-cwd");
    let error = match confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &missing_cwd,
        &fixture.host(),
    ) {
        Ok(_) => panic!("missing cwd must be rejected"),
        Err(error) => error,
    };
    assert!(!error.to_string().contains("unresolved"));
}
#[test]
fn plan_keeps_missing_harness_data_persistent_by_default() {
    let fixture = Fixture::new();
    let claude_root = fixture.home.join(".claude");
    std::fs::create_dir(&claude_root).expect("Claude root");
    let history = claude_root.join("history.jsonl");
    assert!(!history.exists());

    let plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("Claude harness-data plan");
    assert!(plan.unresolved.is_empty());
    assert!(plan.unsaved_files().is_empty());
    assert!(!plan.skipped_paths().contains(&history));
    assert!(
        plan.writable
            .contains(&std::fs::canonicalize(claude_root).expect("Claude root"))
    );
}

#[test]
fn check_entry_syntax_accepts_and_rejects() {
    for entry in ["/a", "~", "~/x", "rel/x"] {
        confine::check_entry_syntax("write", entry)
            .unwrap_or_else(|error| panic!("accepted entry {entry:?} was rejected: {error:#}"));
    }
    let rejected = [
        (
            "$HOME/x",
            "write entry \"$HOME/x\": environment variables are not expanded; write the path out, or use ~/ for your home",
        ),
        (
            "~bob/x",
            "write entry \"~bob/x\": only ~ and ~/... are understood, not another user's home",
        ),
        (
            "~x",
            "write entry \"~x\": only ~ and ~/... are understood, not another user's home",
        ),
        ("", "write entry is empty"),
    ];
    for (entry, expected) in rejected {
        let error = confine::check_entry_syntax("write", entry).expect_err("invalid entry");
        assert_eq!(error.to_string(), expected);
    }
}
#[test]
fn run_dbus_default_hide_is_not_skipped_when_missing() {
    let fixture = Fixture::new();
    let defaults = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("default hide plan");
    let no_defaults = confine::plan(
        &ConfineRequest {
            no_default_hides: true,
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("no-default-hide plan");
    let dbus = Path::new("/run/dbus");
    if dbus.exists() {
        let dbus = std::fs::canonicalize(dbus).expect("dbus canonical");
        assert!(defaults.hidden_paths().contains(&dbus));
        assert!(defaults.default_hidden.contains(&dbus));
        assert!(!no_defaults.hidden_paths().contains(&dbus));
        assert!(!no_defaults.default_hidden.contains(&dbus));
    } else {
        let dbus = dbus.to_path_buf();
        assert!(!defaults.skipped_paths().contains(&dbus));
        assert!(!no_defaults.skipped_paths().contains(&dbus));
    }
}

#[test]
fn skipped_paths_are_sorted() {
    let fixture = Fixture::new();
    let plan = confine::plan(
        &ConfineRequest {
            hide: vec!["/nonexistent-3az-zz".into(), "/nonexistent-3az-aa".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("sorted skipped-path plan");
    let skipped = plan.skipped_paths();
    let zz = skipped
        .iter()
        .position(|path| path == Path::new("/nonexistent-3az-zz"))
        .expect("zz path skipped");
    let aa = skipped
        .iter()
        .position(|path| path == Path::new("/nonexistent-3az-aa"))
        .expect("aa path skipped");
    assert!(aa < zz);
}
