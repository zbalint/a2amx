use super::*;

#[test]
fn write_limit_error_precedes_hide_limit_error() {
    let fixture = Fixture::new();
    let request = ConfineRequest {
        write: (0..65).map(|index| format!("/tmp/path-{index}")).collect(),
        hide: (0..65).map(|index| format!("/tmp/hide-{index}")).collect(),
        ..ConfineRequest::default()
    };
    let error = match confine::plan(&request, Harness::Generic, &fixture.cwd, &fixture.host()) {
        Ok(_) => panic!("entry-count plan must fail"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("write has more than 64 entries"),
        "unexpected entry-count error: {error}"
    );
}

#[test]
fn a_list_of_exactly_64_entries_is_accepted() {
    let fixture = Fixture::new();
    let missing: Vec<String> = (0..confine::MAX_LIST_ENTRIES)
        .map(|index| {
            fixture
                .cwd
                .join(format!("missing-{index}"))
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let cases = [
        (
            "write",
            ConfineRequest {
                write: missing.clone(),
                ..ConfineRequest::default()
            },
            "write has more than 64 entries",
        ),
        (
            "hide",
            ConfineRequest {
                hide: missing.clone(),
                ..ConfineRequest::default()
            },
            "hide has more than 64 entries",
        ),
        (
            "unhide",
            ConfineRequest {
                unhide: missing.clone(),
                ..ConfineRequest::default()
            },
            "unhide has more than 64 entries",
        ),
    ];

    for (label, mut request, expected_error) in cases {
        let plan = confine::plan(&request, Harness::Generic, &fixture.cwd, &fixture.host())
            .unwrap_or_else(|error| panic!("{label} 64 entries must be accepted: {error:#}"));
        for path in &missing {
            assert!(
                plan.skipped
                    .iter()
                    .any(|skipped| skipped == Path::new(path)),
                "{label} path was not skipped: {path}"
            );
        }

        let extra = fixture
            .cwd
            .join(format!("missing-{}", confine::MAX_LIST_ENTRIES))
            .to_string_lossy()
            .into_owned();
        match label {
            "write" => request.write.push(extra.clone()),
            "hide" => request.hide.push(extra.clone()),
            "unhide" => request.unhide.push(extra),
            _ => unreachable!("known confinement list label"),
        }
        let error = match confine::plan(&request, Harness::Generic, &fixture.cwd, &fixture.host()) {
            Ok(_) => panic!("{label} 65 entries must be refused"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains(expected_error),
            "unexpected {label} entry-count error: {error}"
        );
    }
}

#[test]
fn hide_limit_error_precedes_unhide_limit_error() {
    let fixture = Fixture::new();
    let missing: Vec<String> = (0..=confine::MAX_LIST_ENTRIES)
        .map(|index| {
            fixture
                .cwd
                .join(format!("missing-{index}"))
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let request = ConfineRequest {
        hide: missing.clone(),
        unhide: missing,
        ..ConfineRequest::default()
    };
    let error = match confine::plan(&request, Harness::Generic, &fixture.cwd, &fixture.host()) {
        Ok(_) => panic!("hide and unhide entry-count plan must fail"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("hide has more than 64 entries"),
        "unexpected entry-count error: {error}"
    );
    assert!(
        !error.to_string().contains("unhide"),
        "hide entry-count error must precede unhide: {error}"
    );
}

#[test]
fn unhide_removes_default_hidden_paths() {
    let fixture = Fixture::new();
    let aws = std::fs::canonicalize(fixture.home.join(".aws")).expect("aws canonical");
    let defaults = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("default hide plan");
    assert!(defaults.hidden_paths().contains(&aws));
    assert!(defaults.default_hidden.contains(&aws));

    let unhidden = confine::plan(
        &ConfineRequest {
            unhide: vec!["~/.aws".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("unhide plan");
    assert!(!unhidden.hidden_paths().contains(&aws));
    assert!(!unhidden.default_hidden.contains(&aws));
}

#[test]
fn other_harness_credentials_are_default_hidden() {
    let fixture = Fixture::new();
    let claude_credentials = fixture.home.join(".claude/.credentials.json");
    let codex_credentials = fixture.home.join(".codex/auth.json");
    std::fs::create_dir_all(claude_credentials.parent().expect("Claude parent"))
        .expect("Claude directory");
    std::fs::create_dir_all(codex_credentials.parent().expect("Codex parent"))
        .expect("Codex directory");
    std::fs::write(&claude_credentials, "fixture").expect("Claude credentials");
    std::fs::write(&codex_credentials, "fixture").expect("Codex credentials");

    let plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("credential hide plan");
    for path in [claude_credentials, codex_credentials] {
        let path = std::fs::canonicalize(path).expect("credential canonical");
        assert!(plan.hidden_paths().contains(&path));
        assert!(plan.default_hidden.contains(&path));
    }
}

#[test]
fn default_hide_names_and_sources_match_the_literal_sets() {
    let generic = vec![
        "~/.ssh",
        "~/.gnupg",
        "~/.aws",
        "~/.config/gcloud",
        "~/.kube",
        "~/.docker",
        "~/.netrc",
        "~/.local/share/keyrings",
        "~/.config/chromium",
        "~/.config/google-chrome",
        "~/.mozilla/firefox",
        "~/.config/gh",
        "~/.git-credentials",
        "~/.config/git/credentials",
        "~/.npmrc",
        "~/.pypirc",
        "~/.cargo/credentials",
        "~/.cargo/credentials.toml",
        "~/.azure",
        "~/.terraform.d",
        "~/.vault-token",
        "~/.pgpass",
        "~/.password-store",
        "~/.config/op",
        "~/.cache/huggingface/token",
        "~/.config/rclone",
        "~/.s3cfg",
        "~/.claude/ide",
        "~/.claude/.credentials.json",
        "~/.codex/auth.json",
        "~/.gemini/antigravity-cli/antigravity-oauth-token",
        "~/.config/antigravity",
        "/run/dbus",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    assert_eq!(generic.len(), 33);
    assert_eq!(confine::default_hide_names(Harness::Generic), generic);

    let claude = vec![
        "~/.ssh",
        "~/.gnupg",
        "~/.aws",
        "~/.config/gcloud",
        "~/.kube",
        "~/.docker",
        "~/.netrc",
        "~/.local/share/keyrings",
        "~/.config/chromium",
        "~/.config/google-chrome",
        "~/.mozilla/firefox",
        "~/.config/gh",
        "~/.git-credentials",
        "~/.config/git/credentials",
        "~/.npmrc",
        "~/.pypirc",
        "~/.cargo/credentials",
        "~/.cargo/credentials.toml",
        "~/.azure",
        "~/.terraform.d",
        "~/.vault-token",
        "~/.pgpass",
        "~/.password-store",
        "~/.config/op",
        "~/.cache/huggingface/token",
        "~/.config/rclone",
        "~/.s3cfg",
        "~/.claude/ide",
        "~/.codex/auth.json",
        "~/.gemini/antigravity-cli/antigravity-oauth-token",
        "~/.config/antigravity",
        "/run/dbus",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    assert_eq!(claude.len(), 32);
    assert_eq!(confine::default_hide_names(Harness::Claude), claude);

    let codex = vec![
        "~/.ssh",
        "~/.gnupg",
        "~/.aws",
        "~/.config/gcloud",
        "~/.kube",
        "~/.docker",
        "~/.netrc",
        "~/.local/share/keyrings",
        "~/.config/chromium",
        "~/.config/google-chrome",
        "~/.mozilla/firefox",
        "~/.config/gh",
        "~/.git-credentials",
        "~/.config/git/credentials",
        "~/.npmrc",
        "~/.pypirc",
        "~/.cargo/credentials",
        "~/.cargo/credentials.toml",
        "~/.azure",
        "~/.terraform.d",
        "~/.vault-token",
        "~/.pgpass",
        "~/.password-store",
        "~/.config/op",
        "~/.cache/huggingface/token",
        "~/.config/rclone",
        "~/.s3cfg",
        "~/.claude/ide",
        "~/.claude/.credentials.json",
        "~/.gemini/antigravity-cli/antigravity-oauth-token",
        "~/.config/antigravity",
        "/run/dbus",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    assert_eq!(codex.len(), 32);
    assert_eq!(confine::default_hide_names(Harness::Codex), codex);

    let agy = generic
        .iter()
        .filter(|name| {
            !matches!(
                name.as_str(),
                "~/.gemini/antigravity-cli/antigravity-oauth-token" | "~/.config/antigravity"
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(agy.len(), 31);
    assert_eq!(confine::default_hide_names(Harness::Agy), agy);

    let fixture = Fixture::new();
    let claude_credentials = fixture.home.join(".claude/.credentials.json");
    let codex_credentials = fixture.home.join(".codex/auth.json");
    let agy_token = fixture
        .home
        .join(".gemini/antigravity-cli/antigravity-oauth-token");
    let agy_scope = fixture.home.join(".config/antigravity");
    std::fs::create_dir_all(claude_credentials.parent().expect("Claude parent"))
        .expect("Claude directory");
    std::fs::create_dir_all(codex_credentials.parent().expect("Codex parent"))
        .expect("Codex directory");
    std::fs::create_dir_all(agy_token.parent().expect("agy token parent")).expect("agy directory");
    std::fs::create_dir_all(&agy_scope).expect("agy scope directory");
    std::fs::write(&claude_credentials, "fixture").expect("Claude credentials");
    std::fs::write(&codex_credentials, "fixture").expect("Codex credentials");
    std::fs::write(&agy_token, "fixture").expect("agy token");

    let dbus = std::fs::canonicalize("/run/dbus").ok();
    for (harness, expected_credentials) in [
        (
            Harness::Claude,
            vec![&codex_credentials, &agy_token, &agy_scope],
        ),
        (
            Harness::Generic,
            vec![
                &claude_credentials,
                &codex_credentials,
                &agy_token,
                &agy_scope,
            ],
        ),
        (
            Harness::Codex,
            vec![&claude_credentials, &agy_token, &agy_scope],
        ),
    ] {
        let plan = confine::plan(
            &ConfineRequest::default(),
            harness,
            &fixture.cwd,
            &fixture.host(),
        )
        .expect("default hide plan");
        let mut expected = vec![
            std::fs::canonicalize(fixture.home.join(".ssh")).expect("ssh canonical"),
            std::fs::canonicalize(fixture.home.join(".aws")).expect("aws canonical"),
        ];
        expected.extend(
            expected_credentials
                .into_iter()
                .map(|path| std::fs::canonicalize(path).expect("credential canonical")),
        );
        if let Some(dbus) = &dbus {
            expected.push(dbus.clone());
        }
        expected.sort();
        assert_eq!(plan.default_hidden, expected);
    }
}

#[test]
fn git_config_worktree_is_protected_inside_writable_common_dir() {
    let fixture = Fixture::new();
    let repo = fixture.cwd.join("repo");
    let common = repo.join(".git");
    let config_worktree = common.join("config.worktree");
    std::fs::create_dir_all(&common).expect("git directory");
    std::fs::write(&config_worktree, "[core]\n").expect("worktree config");

    let plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &repo,
        &fixture.host(),
    )
    .expect("config.worktree plan");
    let common = std::fs::canonicalize(common).expect("common canonical");
    let config_worktree =
        std::fs::canonicalize(config_worktree).expect("worktree config canonical");
    assert!(plan.writable.contains(&common));
    assert!(plan.readonly.contains(&config_worktree));
    assert!(plan.protected.contains(&config_worktree));
}

#[test]
fn write_error_precedes_bad_unhide_error() {
    let fixture = Fixture::new();
    let socket = fixture.home.join(".docker/run/docker.sock");
    std::fs::create_dir_all(socket.parent().expect("socket parent")).expect("socket directory");
    std::fs::write(&socket, "socket placeholder").expect("socket fixture");
    let error = match confine::plan(
        &ConfineRequest {
            write: vec!["~/.docker/run/docker.sock".into()],
            unhide: vec![fixture.state.to_string_lossy().into_owned()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    ) {
        Ok(_) => panic!("container socket write must be refused first"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("write path"));
    assert!(!error.to_string().contains("unhide"));
}
#[test]
fn mandatory_hides_remain_when_default_hides_are_disabled() {
    let fixture = Fixture::new();
    let request = ConfineRequest {
        no_default_hides: true,
        ..ConfineRequest::default()
    };
    let plan =
        confine::plan(&request, Harness::Generic, &fixture.cwd, &fixture.host()).expect("plan");
    assert!(
        plan.hidden_paths()
            .contains(&fixture.host().layout.state_dir())
    );
    assert!(
        plan.hidden_paths()
            .contains(&fixture.host().layout.run_dir())
    );
    assert!(
        plan.hidden_paths()
            .contains(&fixture.host().layout.secrets_dir())
    );
    for mandatory in ["/run/WSL", "/mnt/wslg"] {
        if let Ok(path) = std::fs::canonicalize(mandatory) {
            assert!(
                plan.hidden_paths().contains(&path),
                "missing mandatory hide: {path:?}"
            );
        }
    }
    assert!(
        plan.hidden_paths()
            .iter()
            .all(|path| path != &fixture.home.join(".aws"))
    );
}

#[test]
fn omp_exposes_only_state_omp_on_top_of_mandatory_state_hide() {
    let fixture = Fixture::new();
    std::fs::create_dir_all(fixture.host().layout.omp_dir()).expect("omp state");
    let plan = confine::plan(
        &ConfineRequest {
            no_default_hides: true,
            ..ConfineRequest::default()
        },
        Harness::Omp,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("plan");
    let omp = fixture
        .host()
        .layout
        .omp_dir()
        .to_string_lossy()
        .into_owned();
    let wrapped = plan
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("wrap");

    let state_hide = wrapped.windows(2).position(|window| {
        window
            == [
                "--tmpfs",
                fixture.host().layout.state_dir().to_string_lossy().as_ref(),
            ]
    });
    assert!(
        state_hide.is_some(),
        "state hide must precede the OMP exposure"
    );
    assert!(
        wrapped
            .windows(4)
            .any(|window| { window == ["--ro-bind", omp.as_str(), omp.as_str(), "--chdir"] })
            || wrapped
                .windows(3)
                .any(|window| window == ["--ro-bind", omp.as_str(), omp.as_str()])
    );
}

#[test]
fn path_lists_and_argv_sizes_are_capped() {
    let fixture = Fixture::new();
    let too_many = ConfineRequest {
        write: (0..=confine::MAX_LIST_ENTRIES)
            .map(|index| format!("/tmp/path-{index}"))
            .collect(),
        ..ConfineRequest::default()
    };
    assert!(confine::plan(&too_many, Harness::Generic, &fixture.cwd, &fixture.host()).is_err());
    let too_many_unhide = ConfineRequest {
        unhide: (0..=confine::MAX_LIST_ENTRIES)
            .map(|index| format!("/tmp/unhide-{index}"))
            .collect(),
        ..ConfineRequest::default()
    };
    assert!(
        confine::plan(
            &too_many_unhide,
            Harness::Generic,
            &fixture.cwd,
            &fixture.host()
        )
        .is_err()
    );
    let too_long = ConfineRequest {
        hide: vec!["x".repeat(4097)],
        ..ConfineRequest::default()
    };
    assert!(confine::plan(&too_long, Harness::Generic, &fixture.cwd, &fixture.host()).is_err());
    let too_long_unhide = ConfineRequest {
        unhide: vec!["x".repeat(4097)],
        ..ConfineRequest::default()
    };
    assert!(
        confine::plan(
            &too_long_unhide,
            Harness::Generic,
            &fixture.cwd,
            &fixture.host()
        )
        .is_err()
    );
    let too_long_team_file = ConfineRequest {
        team_file: Some("x".repeat(4097)),
        ..ConfineRequest::default()
    };
    assert!(
        confine::plan(
            &too_long_team_file,
            Harness::Generic,
            &fixture.cwd,
            &fixture.host()
        )
        .is_err()
    );
    let plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("plan");
    assert!(
        plan.with_bwrap(PathBuf::from("bwrap"))
            .wrap(vec!["x".repeat(128 * 1024 + 1)])
            .is_err()
    );
}
#[test]
fn total_argv_size_checks_sum_arguments_below_per_argument_cap() {
    let fixture = Fixture::new();
    let plan = || {
        confine::plan(
            &ConfineRequest::default(),
            Harness::Generic,
            &fixture.cwd,
            &fixture.host(),
        )
        .expect("plan")
        .with_bwrap(PathBuf::from("bwrap"))
    };
    let over_total_cap = vec!["x".repeat(131_000), "y".repeat(131_000)];
    assert!(plan().wrap(over_total_cap).is_err());
    let below_total_cap = vec!["x".repeat(120_000), "y".repeat(120_000)];
    assert!(plan().wrap(below_total_cap).is_ok());
}
