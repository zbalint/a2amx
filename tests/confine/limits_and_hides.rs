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

#[test]
fn limit_and_validation_precedence_is_exact() {
    let fixture = Fixture::new();
    let over: Vec<String> = (0..=confine::MAX_LIST_ENTRIES)
        .map(|i| format!("/tmp/entry-{i}"))
        .collect();
    let err = |req: ConfineRequest, cwd: &Path, host: &Host| match confine::plan(
        &req,
        Harness::Generic,
        cwd,
        host,
    ) {
        Ok(_) => panic!("expected plan failure for {req:?}"),
        Err(e) => e.to_string(),
    };

    assert_eq!(
        err(
            ConfineRequest {
                write: over.clone(),
                hide: over.clone(),
                ..ConfineRequest::default()
            },
            &fixture.cwd,
            &fixture.host(),
        ),
        "write has more than 64 entries"
    );
    assert_eq!(
        err(
            ConfineRequest {
                hide: over.clone(),
                unhide: over.clone(),
                ..ConfineRequest::default()
            },
            &fixture.cwd,
            &fixture.host(),
        ),
        "hide has more than 64 entries"
    );
    assert_eq!(
        err(
            ConfineRequest {
                unhide: over.clone(),
                grants: over.clone(),
                ..ConfineRequest::default()
            },
            &fixture.cwd,
            &fixture.host(),
        ),
        "unhide has more than 64 entries"
    );
    assert_eq!(
        err(
            ConfineRequest {
                grants: over.clone(),
                keep_env: vec!["GITHUB_TOKEN".into(); 65],
                ..ConfineRequest::default()
            },
            &fixture.cwd,
            &fixture.host(),
        ),
        "grants has more than 64 entries"
    );
    assert_eq!(
        err(
            ConfineRequest {
                keep_env: {
                    let mut list = vec!["NOT_A_CRED".into()];
                    list.extend(std::iter::repeat_n("GITHUB_TOKEN".to_owned(), 64));
                    list
                },
                ..ConfineRequest::default()
            },
            &fixture.cwd,
            &fixture.host(),
        ),
        "keep_env has more than 64 entries"
    );
    assert_eq!(
        err(
            ConfineRequest {
                keep_env: vec!["NOT_A_CRED".into()],
                write: vec!["relative".into()],
                ..ConfineRequest::default()
            },
            &fixture.cwd,
            &fixture.host(),
        ),
        "keep_env \"NOT_A_CRED\" is not unset by default, so there is nothing to keep"
    );
    assert_eq!(
        err(
            ConfineRequest {
                keep_env: vec![
                    "GITHUB_TOKEN".into(),
                    "GITHUB_TOKEN".into(),
                    "NOT_A_CRED".into(),
                ],
                ..ConfineRequest::default()
            },
            &fixture.cwd,
            &fixture.host(),
        ),
        "keep_env \"GITHUB_TOKEN\" is repeated"
    );
    assert_eq!(
        err(
            ConfineRequest {
                keep_env: vec![
                    "NOT_A_CRED".into(),
                    "GITHUB_TOKEN".into(),
                    "GITHUB_TOKEN".into(),
                ],
                ..ConfineRequest::default()
            },
            &fixture.cwd,
            &fixture.host(),
        ),
        "keep_env \"NOT_A_CRED\" is not unset by default, so there is nothing to keep"
    );
    assert_eq!(
        err(
            ConfineRequest {
                write: vec!["relative/first".into(), "$HOME/second".into()],
                hide: vec!["relative/hide".into()],
                ..ConfineRequest::default()
            },
            &fixture.cwd,
            &fixture.host(),
        ),
        "write entry \"relative/first\" must be an absolute path or start with ~/"
    );
    assert_eq!(
        err(
            ConfineRequest {
                hide: vec!["$HOME/bad-hide".into()],
                unhide: vec!["$HOME/bad-unhide".into()],
                ..ConfineRequest::default()
            },
            &fixture.cwd,
            &fixture.host(),
        ),
        "hide entry \"$HOME/bad-hide\": environment variables are not expanded; write the path out, or use ~/ for your home"
    );
    assert_eq!(
        err(
            ConfineRequest {
                unhide: vec!["$HOME/bad-unhide".into()],
                team_file: Some("x".repeat(confine::MAX_PATH_BYTES + 1)),
                ..ConfineRequest::default()
            },
            &fixture.cwd,
            &fixture.host(),
        ),
        "unhide entry \"$HOME/bad-unhide\": environment variables are not expanded; write the path out, or use ~/ for your home"
    );
    assert_eq!(
        err(
            ConfineRequest {
                team_file: Some(format!("rel/{}", "x".repeat(confine::MAX_PATH_BYTES))),
                ..ConfineRequest::default()
            },
            &fixture.cwd,
            &fixture.host(),
        ),
        format!("team_file path exceeds {} bytes", confine::MAX_PATH_BYTES)
    );
    assert_eq!(
        err(
            ConfineRequest {
                team_file: Some("relative/team.toml".into()),
                grants: vec!["no-such-grant".into()],
                ..ConfineRequest::default()
            },
            &fixture.cwd,
            &fixture.host(),
        ),
        "team_file entry \"relative/team.toml\" must be an absolute path or start with ~/"
    );

    let mut bad_home_host = fixture.host();
    bad_home_host.home = fixture._root.path().join("missing-home");
    assert!(
        err(
            ConfineRequest {
                grants: vec!["no-such-grant".into()],
                ..ConfineRequest::default()
            },
            &fixture.cwd,
            &bad_home_host,
        )
        .starts_with("unknown confinement grant \"no-such-grant\"; valid grants:"),
        "grant::validate must run before builder errors"
    );
    assert!(
        err(ConfineRequest::default(), &fixture.cwd, &bad_home_host).contains("home "),
        "builder error must check home"
    );

    let mut bad_root_host = fixture.host();
    bad_root_host.layout = a2amx::layout::Layout::new(fixture._root.path().join("missing-root"));
    assert!(
        err(ConfineRequest::default(), &fixture.cwd, &bad_root_host).contains("a2amx root "),
        "builder error must check root"
    );

    let mut bad_runtime_host = fixture.host();
    bad_runtime_host.runtime_dir = fixture._root.path().join("missing-runtime");
    assert!(
        err(ConfineRequest::default(), &fixture.cwd, &bad_runtime_host)
            .contains("runtime directory "),
        "builder error must check runtime directory"
    );

    std::fs::write(&fixture.empty, b"not-empty").expect("non-empty confine-empty");
    assert_eq!(
        err(ConfineRequest::default(), &fixture.cwd, &fixture.host()),
        "confinement empty file must be an empty run file"
    );
    let missing_cwd = fixture._root.path().join("missing-cwd");
    assert!(
        err(ConfineRequest::default(), &missing_cwd, &fixture.host()).contains("cwd "),
        "missing cwd must fail before empty file check"
    );
    let file_cwd = fixture._root.path().join("file-cwd");
    std::fs::write(&file_cwd, "not a dir").expect("file cwd");
    assert_eq!(
        err(ConfineRequest::default(), &file_cwd, &fixture.host()),
        format!(
            "cwd is not a directory: {}",
            std::fs::canonicalize(&file_cwd).expect("canon").display()
        )
    );
}

#[test]
fn implicit_writes_hidden_finish_order_and_sorted_outputs() {
    let fixture = Fixture::new();
    let aws = std::fs::canonicalize(fixture.home.join(".aws")).expect("aws");
    let sub_cwd = aws.join("repo");
    std::fs::create_dir_all(&sub_cwd).expect("sub_cwd");
    let hidden_git = fixture.home.join(".ssh/git-common");
    std::fs::create_dir_all(hidden_git.join("objects")).expect("objects");
    std::fs::write(hidden_git.join("HEAD"), "ref: refs/heads/main\n").expect("HEAD");
    std::fs::write(
        sub_cwd.join(".git"),
        format!("gitdir: {}\n", hidden_git.display()),
    )
    .expect("gitfile");
    let ssh = std::fs::canonicalize(fixture.home.join(".ssh")).expect("ssh");
    let err = match confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &sub_cwd,
        &fixture.host(),
    ) {
        Ok(_) => panic!("hidden cwd and git dir must fail"),
        Err(e) => e.to_string(),
    };
    let sub_cwd_canon = std::fs::canonicalize(&sub_cwd).expect("sub_cwd canon");
    let hidden_git_canon = std::fs::canonicalize(&hidden_git).expect("hidden_git canon");
    assert_eq!(
        err,
        format!(
            "working directory {0} resolves to the hidden path {1}; open it with write = [\"{0}\"] if that is intended; git directory {2} resolves to the hidden path {3}; open it with write = [\"{2}\"] if that is intended",
            sub_cwd_canon.display(),
            aws.display(),
            hidden_git_canon.display(),
            ssh.display()
        )
    );

    let suppressed = confine::plan(
        &ConfineRequest {
            write: vec![
                sub_cwd_canon.to_string_lossy().into_owned(),
                hidden_git_canon.to_string_lossy().into_owned(),
            ],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &sub_cwd,
        &fixture.host(),
    )
    .expect("explicit writes suppress implicit-write-hidden error");
    assert!(suppressed.writable.contains(&sub_cwd_canon));

    // finish error order: harness-config hide vs unhide vs grant unhide vs implicit write
    let claude_root = fixture.home.join(".claude");
    std::fs::create_dir_all(&claude_root).expect("claude root");
    let claude_canon = std::fs::canonicalize(&claude_root).expect("claude canon");
    let err_hc_hide = match confine::plan(
        &ConfineRequest {
            grants: vec!["harness-config".into(), "aws".into()],
            hide: vec!["~/.claude".into(), "~/.aws".into()],
            ..ConfineRequest::default()
        },
        Harness::Claude,
        &sub_cwd,
        &fixture.host(),
    ) {
        Ok(_) => panic!("must fail"),
        Err(e) => e.to_string(),
    };
    assert_eq!(
        err_hc_hide,
        format!(
            "confinement grant \"harness-config\" conflicts with the hide of {}",
            claude_canon.display()
        )
    );

    let err_hc_unhide = match confine::plan(
        &ConfineRequest {
            grants: vec!["harness-config".into(), "aws".into()],
            hide: vec!["~/.aws".into()],
            unhide: vec!["~/.claude".into()],
            ..ConfineRequest::default()
        },
        Harness::Claude,
        &sub_cwd,
        &fixture.host(),
    ) {
        Ok(_) => panic!("must fail"),
        Err(e) => e.to_string(),
    };
    assert_eq!(
        err_hc_unhide,
        format!(
            "confinement grant \"harness-config\" conflicts with the unhide of {}",
            claude_canon.display()
        )
    );

    let err_grant_unhide = match confine::plan(
        &ConfineRequest {
            grants: vec!["aws".into()],
            hide: vec!["~/.aws".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &sub_cwd,
        &fixture.host(),
    ) {
        Ok(_) => panic!("must fail"),
        Err(e) => e.to_string(),
    };
    assert_eq!(
        err_grant_unhide,
        format!(
            "confinement grant \"aws\" conflicts with the hide of {}",
            aws.display()
        )
    );

    // Sorted outputs + unresolved sort by (option, path) + effective_grants dedup
    let plan = confine::plan(
        &ConfineRequest {
            grants: vec!["ssh".into(), "cache".into()],
            write: vec![
                fixture.cwd.join("z-missing").to_string_lossy().into_owned(),
                fixture.cwd.join("a-missing").to_string_lossy().into_owned(),
            ],
            hide: vec![fixture.cwd.join("m-missing").to_string_lossy().into_owned()],
            keep_env: vec!["NPM_TOKEN".into(), "GH_TOKEN".into()],
            ..ConfineRequest::default()
        },
        Harness::Claude,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("sorted plan");
    assert_eq!(plan.grants, vec!["cache", "ssh"]);
    assert_eq!(
        plan.effective_grants,
        vec!["cache", "harness-config", "ssh"]
    );
    assert_eq!(plan.keep_env, vec!["GH_TOKEN", "NPM_TOKEN"]);
    let assert_sorted = |list: &[PathBuf], name: &str| {
        let mut sorted = list.to_vec();
        sorted.sort();
        assert_eq!(list, sorted.as_slice(), "{name} not sorted");
    };
    assert_sorted(&plan.readonly, "readonly");
    assert_sorted(&plan.writable, "writable");
    assert_sorted(&plan.hidden, "hidden");
    assert_sorted(&plan.mandatory_hidden, "mandatory_hidden");
    assert_sorted(&plan.default_hidden, "default_hidden");
    assert_sorted(&plan.container_hidden, "container_hidden");
    assert_sorted(&plan.skipped, "skipped");
    let unresolved_keys: Vec<_> = plan
        .unresolved
        .iter()
        .map(|u| (u.option, u.path.clone()))
        .collect();
    let mut sorted_unresolved = unresolved_keys.clone();
    sorted_unresolved.sort();
    assert_eq!(unresolved_keys, sorted_unresolved);
}

#[test]
fn readonly_retain_precedes_grant_conflict_check() {
    let fixture = Fixture::new();
    let claude_root = fixture.home.join(".claude");
    std::fs::create_dir_all(&claude_root).expect("claude root");
    let claude_canon = std::fs::canonicalize(&claude_root).expect("claude canon");
    let plan = confine::plan(
        &ConfineRequest {
            grants: vec!["harness-config".into()],
            write: vec!["~/.claude".into()],
            unhide: vec!["~/.claude".into()],
            ..ConfineRequest::default()
        },
        Harness::Claude,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("explicit write removes harness-config root from readonly before conflict check");
    assert!(plan.writable.contains(&claude_canon));
    assert!(!plan.readonly.contains(&claude_canon));
    assert!(plan.unresolved.is_empty());
}

#[test]
fn readonly_sort_orders_multiple_entries_and_absent_team_file_leaves_skipped_empty() {
    let fixture = Fixture::new();
    let plan = confine::plan(
        &ConfineRequest {
            unhide: vec!["~/.ssh/config".into(), "~/.aws".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("plan with multiple readonly entries");
    assert!(
        plan.readonly.len() >= 2,
        "readonly must have at least two entries: {:?}",
        plan.readonly
    );
    let mut sorted_readonly = plan.readonly.clone();
    sorted_readonly.sort();
    assert_eq!(plan.readonly, sorted_readonly, "readonly must be sorted");

    let no_team_file = confine::plan(
        &ConfineRequest {
            no_default_hides: true,
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("plan without team_file");
    let mut expected_skipped: Vec<PathBuf> = [
        "/run/buildkit/buildkitd.sock",
        "/run/crio/crio.sock",
        "/run/k3s/containerd/containerd.sock",
        "/run/libvirt/libvirt-sock",
        "/run/podman/podman.sock",
        "/var/lib/incus/unix.socket",
        "/var/lib/lxd/unix.socket",
        "/var/snap/lxd/common/lxd/unix.socket",
    ]
    .into_iter()
    .map(PathBuf::from)
    .chain([
        fixture.home.join(".docker/desktop/docker.sock"),
        fixture.home.join(".docker/run/docker.sock"),
    ])
    .filter(|path| !path.exists())
    .collect();
    expected_skipped.sort();
    assert_eq!(no_team_file.skipped, expected_skipped);
}

#[test]
fn implicit_write_inside_container_socket_names_both_default_and_container_hide() {
    let fixture = Fixture::new();
    let docker_dir = fixture.home.join(".docker");
    let socket_dir = docker_dir.join("run/docker.sock");
    let sub_cwd = socket_dir.join("repo");
    std::fs::create_dir_all(&sub_cwd).expect("sub_cwd inside docker socket dir");
    let docker_canon = std::fs::canonicalize(&docker_dir).expect("docker canon");
    let socket_canon = std::fs::canonicalize(&socket_dir).expect("socket canon");
    let sub_cwd_canon = std::fs::canonicalize(&sub_cwd).expect("sub_cwd canon");

    let err = match confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &sub_cwd,
        &fixture.host(),
    ) {
        Ok(_) => panic!("cwd inside docker socket dir must fail"),
        Err(e) => e.to_string(),
    };
    assert_eq!(
        err,
        format!(
            "working directory {0} resolves to the hidden path {1}; open it with write = [\"{0}\"] if that is intended; working directory {0} resolves to the hidden path {2}; open it with write = [\"{0}\"] if that is intended",
            sub_cwd_canon.display(),
            docker_canon.display(),
            socket_canon.display(),
        )
    );
}
