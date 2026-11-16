use super::*;

#[test]
fn codex_is_accepted_and_write_guards_are_reciprocal() {
    let fixture = Fixture::new();
    std::fs::create_dir_all(fixture.home.join(".codex")).expect("Codex home");
    let plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Codex,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("Codex confinement");
    assert!(
        plan.writable
            .iter()
            .any(|path| path == &fixture.home.join(".codex"))
    );
    assert!(
        confine::check_write_allowed(&fixture.state, &fixture.home, &fixture.state, false).is_err()
    );
    assert!(
        confine::check_write_allowed(
            fixture.state.parent().expect("parent"),
            &fixture.home,
            &fixture.state,
            false,
        )
        .is_err()
    );
    assert!(
        confine::check_write_allowed(&fixture.runtime, &fixture.home, &fixture.state, false)
            .is_ok()
    );
    assert!(
        confine::check_write_allowed(
            fixture.state.parent().expect("state ancestor"),
            &fixture.home,
            &fixture.state,
            true,
        )
        .is_ok()
    );
    let state_child = fixture.state.join("nested");
    std::fs::create_dir_all(&state_child).expect("state child");
    assert!(
        confine::check_write_allowed(&state_child, &fixture.home, &fixture.state, true).is_err()
    );

    assert!(
        confine::check_unhide_allowed(
            &fixture.home,
            &fixture.state,
            &fixture.runtime,
            &fixture.home
        )
        .is_err()
    );
    assert!(
        confine::check_unhide_allowed(
            fixture.home.parent().expect("home parent"),
            &fixture.state,
            &fixture.runtime,
            &fixture.home
        )
        .is_err()
    );
    assert!(
        confine::plan(
            &ConfineRequest::default(),
            Harness::Generic,
            &fixture.home,
            &fixture.host()
        )
        .is_err()
    );
    assert!(
        confine::plan(
            &ConfineRequest {
                write: vec!["/".into()],
                ..ConfineRequest::default()
            },
            Harness::Generic,
            &fixture.cwd,
            &fixture.host()
        )
        .is_err()
    );
    assert!(
        confine::plan(
            &ConfineRequest {
                write: vec![fixture.state.to_string_lossy().into_owned()],
                ..ConfineRequest::default()
            },
            Harness::Generic,
            &fixture.cwd,
            &fixture.host()
        )
        .is_err()
    );

    assert!(
        confine::plan(
            &ConfineRequest {
                write: vec![fixture.runtime.to_string_lossy().into_owned()],
                ..ConfineRequest::default()
            },
            Harness::Generic,
            &fixture.cwd,
            &fixture.host()
        )
        .is_err()
    );
    std::fs::write(fixture.state.join("run/admin.token"), "admin").expect("admin token");
    for path in [
        fixture.state.join("run/admin.token"),
        fixture.state.clone(),
        fixture.runtime.clone(),
    ] {
        assert!(
            confine::check_unhide_allowed(&path, &fixture.state, &fixture.runtime, &fixture.home)
                .is_err()
        );
    }
}
#[test]
fn codex_plan_wraps_own_socket_dir_with_read_only_and_read_write_variants() {
    let fixture = Fixture::new();
    std::fs::create_dir_all(fixture.home.join(".codex")).expect("Codex home");
    std::fs::write(fixture.home.join(".codex/auth.json"), "{}").expect("Codex auth");
    let plan = confine::plan(
        &ConfineRequest {
            write: vec!["/tmp".into()],
            ..ConfineRequest::default()
        },
        Harness::Codex,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("Codex plan")
    .with_bwrap(PathBuf::from("bwrap"))
    .with_codex_dir(fixture.host().layout.codex_session_dir("s1"));
    let without_codex_dir = confine::plan(
        &ConfineRequest {
            write: vec!["/tmp".into()],
            ..ConfineRequest::default()
        },
        Harness::Codex,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("Codex plan without socket dir")
    .with_bwrap(PathBuf::from("bwrap"));
    let legacy_wrapped = without_codex_dir
        .wrap(vec!["codex".into()])
        .expect("legacy TUI argv");
    let root = fixture.host().layout.root().to_string_lossy().into_owned();
    let state = fixture
        .host()
        .layout
        .state_dir()
        .to_string_lossy()
        .into_owned();
    let runtime = fixture.runtime.to_string_lossy().into_owned();
    let run = fixture
        .host()
        .layout
        .run_dir()
        .to_string_lossy()
        .into_owned();
    let secrets = fixture
        .host()
        .layout
        .secrets_dir()
        .to_string_lossy()
        .into_owned();
    let home = fixture.home.to_string_lossy().into_owned();
    let cwd = fixture.cwd.to_string_lossy().into_owned();
    let empty = fixture
        .host()
        .layout
        .confine_empty()
        .to_string_lossy()
        .into_owned();
    let mut expected = vec![
        "bwrap".into(),
        "--die-with-parent".into(),
        "--unshare-user".into(),
        "--unshare-ipc".into(),
        "--unshare-pid".into(),
        "--ro-bind".into(),
        "/".into(),
        "/".into(),
        "--dev".into(),
        "/dev".into(),
        "--proc".into(),
        "/proc".into(),
        "--size".into(),
        "1073741824".into(),
        "--tmpfs".into(),
        "/tmp".into(),
    ];
    for name in [
        "DBUS_SESSION_BUS_ADDRESS",
        "SSH_AUTH_SOCK",
        "GPG_AGENT_INFO",
        "WSL_INTEROP",
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "DOCKER_HOST",
        "DOCKER_CONTEXT",
        "CONTAINER_HOST",
        "CONTAINER_CONNECTION",
        "CONTAINERD_ADDRESS",
        "BUILDKIT_HOST",
        "LIBVIRT_DEFAULT_URI",
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "GH_ENTERPRISE_TOKEN",
        "GITLAB_TOKEN",
        "GITLAB_PRIVATE_TOKEN",
        "CI_JOB_TOKEN",
        "BITBUCKET_TOKEN",
        "NPM_TOKEN",
        "NODE_AUTH_TOKEN",
        "PYPI_TOKEN",
        "TWINE_PASSWORD",
        "CARGO_REGISTRY_TOKEN",
        "CARGO_REGISTRIES_CRATES_IO_TOKEN",
        "AZURE_CLIENT_SECRET",
        "ARM_CLIENT_SECRET",
        "VAULT_TOKEN",
        "HF_TOKEN",
        "HUGGING_FACE_HUB_TOKEN",
        "SONAR_TOKEN",
        "SENTRY_AUTH_TOKEN",
        "DIGITALOCEAN_ACCESS_TOKEN",
        "HCLOUD_TOKEN",
        "CLOUDFLARE_API_TOKEN",
        "NETLIFY_AUTH_TOKEN",
        "VERCEL_TOKEN",
        "FLY_API_TOKEN",
        "HEROKU_API_KEY",
        "PGPASSWORD",
        "MYSQL_PWD",
    ] {
        expected.extend(["--unsetenv".into(), name.into()]);
    }
    expected.extend(["--bind".into(), "/tmp".into(), "/tmp".into()]);
    // These hides exist only on hosts that have the path (WSL, a desktop bus), like the plan.
    let host_paths: Vec<String> = ["/mnt/wslg", "/run/WSL", "/run/dbus"]
        .into_iter()
        .filter_map(|path| std::fs::canonicalize(path).ok())
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    for path in &host_paths {
        expected.extend(["--tmpfs".into(), path.clone()]);
    }
    expected.extend([
        "--ro-bind".into(),
        empty.clone(),
        "/run/docker.sock".into(),
        "--ro-bind".into(),
        empty.clone(),
        "/run/snapd-snap.socket".into(),
        "--ro-bind".into(),
        empty.clone(),
        "/run/snapd.socket".into(),
        "--ro-bind".into(),
        empty.clone(),
        "/run/containerd/containerd.sock".into(),
        "--ro-bind".into(),
        empty.clone(),
        "/run/containerd/containerd.sock.ttrpc".into(),
        "--tmpfs".into(),
        runtime.clone(),
        "--ro-bind".into(),
        root.clone(),
        root.clone(),
        "--bind".into(),
        cwd.clone(),
        cwd.clone(),
        "--tmpfs".into(),
        format!("{home}/.aws"),
        "--tmpfs".into(),
        format!("{home}/.ssh"),
        "--tmpfs".into(),
        run.clone(),
        "--tmpfs".into(),
        state.clone(),
        "--bind".into(),
        format!("{home}/.codex"),
        format!("{home}/.codex"),
        "--tmpfs".into(),
        secrets.clone(),
    ]);
    for path in &host_paths {
        expected.extend(["--remount-ro".into(), path.clone()]);
    }
    expected.extend([
        "--remount-ro".into(),
        runtime,
        "--remount-ro".into(),
        format!("{home}/.aws"),
        "--remount-ro".into(),
        format!("{home}/.ssh"),
        "--remount-ro".into(),
        run,
        "--remount-ro".into(),
        state,
        "--remount-ro".into(),
        secrets,
        "--chdir".into(),
        cwd,
        "--".into(),
        "codex".into(),
    ]);
    assert_eq!(legacy_wrapped, expected);
    let wrapped = plan.wrap(vec!["codex".into()]).expect("TUI argv");
    let server = plan
        .wrap_app_server(vec!["codex".into()])
        .expect("app-server argv");
    let session = fixture
        .host()
        .layout
        .codex_session_dir("s1")
        .to_string_lossy()
        .into_owned();
    let socket = fixture
        .host()
        .layout
        .codex_session_dir("s1")
        .join("sock")
        .to_string_lossy()
        .into_owned();
    let target = format!("/tmp/codex-daemon-{}", rustix::process::geteuid().as_raw());
    let tui = ["--ro-bind", socket.as_str(), target.as_str()];
    let rw = ["--bind", socket.as_str(), target.as_str()];
    assert!(wrapped.windows(3).any(|window| window == tui));
    assert!(server.windows(3).any(|window| window == rw));
    let chdir = wrapped
        .iter()
        .position(|value| value == "--chdir")
        .expect("--chdir");
    assert_eq!(&wrapped[chdir - 3..chdir], tui);
    let server_chdir = server
        .iter()
        .position(|value| value == "--chdir")
        .expect("server --chdir");
    assert_eq!(&server[server_chdir - 3..server_chdir], rw);
    let identity = ["--ro-bind", session.as_str(), session.as_str()];
    let identity_start = server
        .windows(3)
        .position(|window| window == identity)
        .expect("server identity bind");
    assert!(!wrapped.windows(3).any(|window| window == identity));
    let first_remount = server
        .iter()
        .position(|value| value == "--remount-ro")
        .expect("first remount");
    assert!(identity_start < first_remount);
    let codex_home_path = fixture.home.join(".codex").to_string_lossy().into_owned();
    let home_bind = server
        .windows(3)
        .position(|window| {
            window[0] == "--bind" && window[1] == codex_home_path && window[2] == codex_home_path
        })
        .expect("Codex home bind");
    assert!(identity_start > home_bind);
    let codex_home = fixture.home.join(".codex");
    assert!(plan.writable.iter().any(|path| path == &codex_home));
    assert!(!plan.hidden_paths().contains(&codex_home.join("auth.json")));
}

#[test]
fn codex_launch_argv_matches_plan_wrap() {
    let fixture = Fixture::new();
    let plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Codex,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("Codex plan")
    .with_bwrap(PathBuf::from("bwrap"));
    let argv = confine::launch_argv(
        &plan,
        "confinement notice",
        vec!["codex".into(), "--remote".into()],
        Harness::Codex,
    )
    .expect("Codex launch argv");
    let expected = plan
        .wrap(vec!["codex".into(), "--remote".into()])
        .expect("expected wrapped Codex argv");
    assert_eq!(argv, expected);
}

#[test]
fn agy_plan_binds_only_antigravity_data_by_default() {
    let fixture = Fixture::new();
    let gemini = fixture.home.join(".gemini");
    let antigravity = gemini.join("antigravity-cli");
    let config = gemini.join("config");
    let token = antigravity.join("antigravity-oauth-token");
    let scope = fixture.home.join(".config/antigravity");
    std::fs::create_dir_all(&config).expect("Gemini config");
    std::fs::create_dir_all(&antigravity).expect("Antigravity data");
    std::fs::create_dir_all(&scope).expect("credential scope");
    std::fs::write(&token, "fixture").expect("OAuth token");

    let plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Agy,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("Agy confinement");
    let antigravity = std::fs::canonicalize(antigravity).expect("data canonical");
    let gemini = std::fs::canonicalize(gemini).expect("root canonical");
    let config = std::fs::canonicalize(config).expect("config canonical");
    let token = std::fs::canonicalize(token).expect("token canonical");
    let scope = std::fs::canonicalize(scope).expect("scope canonical");
    assert!(plan.writable.contains(&antigravity));
    assert!(
        !plan
            .writable
            .iter()
            .any(|path| path == &gemini || config.starts_with(path))
    );
    assert!(!plan.hidden_paths().contains(&token));
    assert!(!plan.hidden_paths().contains(&scope));
    assert!(!plan.skipped_paths().contains(&token));
    assert!(!plan.skipped_paths().contains(&scope));

    let granted = confine::plan(
        &ConfineRequest {
            grants: vec!["harness-config".into()],
            ..ConfineRequest::default()
        },
        Harness::Agy,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("Agy harness-config grant");
    assert!(granted.writable.contains(&gemini));
    assert!(!granted.writable.contains(&antigravity));
}

#[test]
fn ensure_data_dirs_creates_agy_data_only_when_root_exists() {
    let fixture = Fixture::new();
    let gemini = fixture.home.join(".gemini");
    std::fs::create_dir_all(&gemini).expect("Gemini root");
    confine::ensure_data_dirs(Harness::Agy, &fixture.home).expect("Agy data dirs");
    let antigravity = gemini.join("antigravity-cli");
    assert!(antigravity.is_dir());
    assert_eq!(
        std::fs::metadata(&antigravity)
            .expect("data metadata")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );

    let absent = Fixture::new();
    confine::ensure_data_dirs(Harness::Agy, &absent.home).expect("absent Gemini root");
    assert!(!absent.home.join(".gemini/antigravity-cli").exists());
}

#[test]
fn agy_launch_argv_matches_plan_wrap() {
    let fixture = Fixture::new();
    let plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Agy,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("Agy plan")
    .with_bwrap(PathBuf::from("bwrap"));
    let argv = confine::launch_argv(
        &plan,
        "confinement notice",
        vec!["agy".into(), "--remote".into()],
        Harness::Agy,
    )
    .expect("Agy launch argv");
    let expected = plan
        .wrap(vec!["agy".into(), "--remote".into()])
        .expect("expected wrapped Agy argv");
    assert_eq!(argv, expected);
}

#[test]
fn symlinked_harness_data_paths_are_refused_before_setup() {
    let claude = Fixture::new();
    let claude_root = claude.home.join(".claude");
    std::fs::create_dir_all(&claude_root).expect("claude root");
    symlink(&claude.state, claude_root.join("projects")).expect("claude projects symlink");
    assert!(
        confine::plan(
            &ConfineRequest::default(),
            Harness::Claude,
            &claude.cwd,
            &claude.host()
        )
        .is_err()
    );

    let omp = Fixture::new();
    let omp_root = omp.home.join(".omp");
    std::fs::create_dir_all(&omp_root).expect("omp root");
    let omp_agent = omp_root.join("agent");
    symlink(&omp.state, &omp_agent).expect("omp agent symlink");
    assert!(
        confine::plan(
            &ConfineRequest::default(),
            Harness::Omp,
            &omp.cwd,
            &omp.host()
        )
        .is_err()
    );
}

#[test]
fn omp_sidecars_cover_existing_agent_and_stats_databases_only() {
    let root = tempfile::tempdir().expect("sidecar tempdir");
    let home = root.path().join("home");
    let omp = home.join(".omp");
    let agent = omp.join("agent");
    std::fs::create_dir_all(&agent).expect("agent directory");
    let agent_db = agent.join("a.db");
    let stats_db = omp.join("stats.db");
    let missing_db = agent.join("missing.db");
    std::fs::write(&agent_db, "agent").expect("agent database");
    std::fs::write(&stats_db, "stats").expect("stats database");
    std::fs::write(agent.join("sidecar.db-wal"), "wal").expect("existing sidecar");

    confine::ensure_data_files(Harness::Omp, &home).expect("omp sidecars");
    for database in [&agent_db, &stats_db] {
        for suffix in ["-wal", "-shm"] {
            let sidecar = PathBuf::from(format!("{}{}", database.display(), suffix));
            assert!(sidecar.is_file(), "missing sidecar: {sidecar:?}");
            assert_eq!(
                std::fs::metadata(sidecar)
                    .expect("sidecar metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
    assert!(!missing_db.with_extension("db-wal").exists());
    assert!(!agent.join("sidecar.db-wal-wal").exists());

    let claude_home = root.path().join("claude-home");
    std::fs::create_dir_all(claude_home.join(".claude")).expect("claude home");
    confine::ensure_data_files(Harness::Claude, &claude_home).expect("claude files");
    assert!(!claude_home.join(".claude/stats.db-wal").exists());
    let generic_home = root.path().join("generic-home");
    confine::ensure_data_files(Harness::Generic, &generic_home).expect("generic files");
    assert!(!generic_home.join(".omp/stats.db-wal").exists());
}
#[test]
fn harness_config_default_opens_only_the_harness_root() {
    let claude = Fixture::new();
    populate_claude_overlay(&claude, true);
    let claude_root = std::fs::canonicalize(claude.home.join(".claude")).expect("Claude root");
    let claude_plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &claude.cwd,
        &claude.host(),
    )
    .expect("Claude plan");
    assert!(claude_plan.writable.contains(&claude_root));
    assert!(claude_plan.overlay_parents().is_empty());
    assert!(claude_plan.grants.is_empty());

    let omp = Fixture::new();
    populate_omp_overlay(&omp);
    let omp_root = std::fs::canonicalize(omp.home.join(".omp")).expect("OMP root");
    let omp_plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Omp,
        &omp.cwd,
        &omp.host(),
    )
    .expect("OMP plan");
    assert!(omp_plan.writable.contains(&omp_root));
    assert!(omp_plan.overlay_parents().is_empty());

    let generic = Fixture::new();
    let generic_plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &generic.cwd,
        &generic.host(),
    )
    .expect("Generic plan");
    assert!(!generic_plan.writable.contains(&generic.home.join(".cargo")));
    assert!(!generic_plan.writable.contains(&generic.home.join(".cache")));
}

#[test]
fn confine_info_grants_are_sorted_deduplicated_and_include_implicit_harness_config() {
    let claude = Fixture::new();
    populate_claude_overlay(&claude, false);
    let plan = confine::plan(
        &ConfineRequest {
            grants: vec!["ssh".into(), "cache".into()],
            ..ConfineRequest::default()
        },
        Harness::Claude,
        &claude.cwd,
        &claude.host(),
    )
    .expect("Claude plan");
    assert_eq!(plan.grants, vec!["cache", "ssh"]);
    let info = plan.confine_info();
    assert_eq!(info.grants, vec!["cache", "harness-config", "ssh"]);
    assert_eq!(info.write, Vec::<String>::new());

    // An impersonal request that already names harness-config must not duplicate it.
    let granted = confine::plan(
        &ConfineRequest {
            grants: vec!["harness-config".into()],
            ..ConfineRequest::default()
        },
        Harness::Claude,
        &claude.cwd,
        &claude.host(),
    )
    .expect("explicit harness-config plan");
    assert_eq!(granted.confine_info().grants, vec!["harness-config"]);
}
