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
    let claude_projects = claude_root.join("projects");
    symlink(&claude.state, &claude_projects).expect("claude projects symlink");
    let claude_error = confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &claude.cwd,
        &claude.host(),
    )
    .err()
    .expect("claude projects symlink outside root");
    let claude_root_canon = std::fs::canonicalize(&claude_root).expect("claude root canon");
    let claude_state_canon = std::fs::canonicalize(&claude.state).expect("claude state canon");
    // Validation's inside-root check fires before mount_data_dir.
    assert_eq!(
        claude_error.to_string(),
        format!(
            "harness data path {} resolves to {}, outside {}; remove the link, keep the data inside {}, or open it with write = [\"{}\"] if that is intended",
            claude_projects.display(),
            claude_state_canon.display(),
            claude_root_canon.display(),
            claude_root_canon.display(),
            claude_state_canon.display(),
        )
    );

    let omp = Fixture::new();
    let omp_root = omp.home.join(".omp");
    std::fs::create_dir_all(&omp_root).expect("omp root");
    let omp_agent = omp_root.join("agent");
    symlink(&omp.state, &omp_agent).expect("omp agent symlink");
    let omp_error = confine::plan(
        &ConfineRequest::default(),
        Harness::Omp,
        &omp.cwd,
        &omp.host(),
    )
    .err()
    .expect("omp agent symlink overlapping state");
    let omp_state_canon = std::fs::canonicalize(&omp.state).expect("omp state canon");
    // `.omp/agent` is its own expected_source, so its inside check passes and check_write_allowed fires.
    assert_eq!(
        omp_error.to_string(),
        format!(
            "writable path {} overlaps the a2amx directory",
            omp_state_canon.display()
        )
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
    assert!(!agent.join("sidecar.db-wal-wal").exists());

    let claude_home = root.path().join("claude-home");
    std::fs::create_dir_all(claude_home.join(".claude")).expect("claude home");
    std::fs::create_dir_all(claude_home.join(".omp/agent")).expect("claude omp agent");
    std::fs::write(claude_home.join(".omp/agent/a.db"), "agent").expect("claude omp db");
    confine::ensure_data_files(Harness::Claude, &claude_home).expect("claude files");
    assert!(!claude_home.join(".omp/agent/a.db-wal").exists());
    assert!(!claude_home.join(".omp/agent/a.db-shm").exists());
    let generic_home = root.path().join("generic-home");
    std::fs::create_dir_all(generic_home.join(".omp/agent")).expect("generic omp agent");
    std::fs::write(generic_home.join(".omp/agent/a.db"), "agent").expect("generic omp db");
    confine::ensure_data_files(Harness::Generic, &generic_home).expect("generic files");
    assert!(!generic_home.join(".omp/agent/a.db-wal").exists());
    assert!(!generic_home.join(".omp/agent/a.db-shm").exists());
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

#[test]
fn harness_data_writable_and_skipped_lists_are_exact_per_harness() {
    // Claude: unhide ~/.claude so effective_grants does not collapse data mounts into harness-config.
    let claude = Fixture::new();
    populate_claude_overlay(&claude, true);
    let claude_home_file = claude.home.join(".claude.json");
    std::fs::write(&claude_home_file, "{}").expect("claude.json");
    let claude_plan = confine::plan(
        &ConfineRequest {
            unhide: vec!["~/.claude".into()],
            no_default_hides: true,
            ..ConfineRequest::default()
        },
        Harness::Claude,
        &claude.cwd,
        &claude.host(),
    )
    .expect("Claude data plan");
    let claude_root = claude.home.join(".claude");
    let mut expected_claude_writable = vec![std::fs::canonicalize(&claude.cwd).expect("cwd")];
    for name in [
        "projects",
        "todos",
        "shell-snapshots",
        "session-env",
        "sessions",
        "statsig",
        "file-history",
        "paste-cache",
        "debug",
        "cache",
        "telemetry",
        "plans",
        "backups",
    ] {
        expected_claude_writable.push(std::fs::canonicalize(claude_root.join(name)).expect("dir"));
    }
    for name in ["history.jsonl", "stats-cache.json", ".credentials.json"] {
        expected_claude_writable.push(std::fs::canonicalize(claude_root.join(name)).expect("file"));
    }
    expected_claude_writable.push(std::fs::canonicalize(&claude_home_file).expect("claude.json"));
    expected_claude_writable.sort();
    assert_eq!(claude_plan.writable, expected_claude_writable);
    let claude_home_skipped: Vec<PathBuf> = claude_plan
        .skipped_paths()
        .iter()
        .filter(|path| path.starts_with(&claude_root) || *path == &claude_home_file)
        .cloned()
        .collect();
    assert_eq!(claude_home_skipped, Vec::<PathBuf>::new());

    // OMP: 7 dirs, BTreeSet-ordered *.db + -wal/-shm, stats.db only when a file, non-.db ignored, .db dir still listed.
    let omp = Fixture::new();
    let omp_root = omp.home.join(".omp");
    let omp_agent = omp_root.join("agent");
    std::fs::create_dir_all(&omp_agent).expect("omp agent");
    std::fs::write(omp_agent.join("b.db"), "b").expect("b.db");
    std::fs::write(omp_agent.join("a.db"), "a").expect("a.db");
    std::fs::write(omp_agent.join("a.db-wal"), "wal").expect("a.db-wal");
    std::fs::write(omp_agent.join("notes.txt"), "txt").expect("non-db");
    std::fs::create_dir_all(omp_agent.join("dir.db")).expect("dir.db");
    std::fs::write(omp_root.join("stats.db"), "stats").expect("stats.db");
    let omp_plan = confine::plan(
        &ConfineRequest {
            unhide: vec!["~/.omp".into()],
            no_default_hides: true,
            ..ConfineRequest::default()
        },
        Harness::Omp,
        &omp.cwd,
        &omp.host(),
    )
    .expect("OMP data plan");
    let omp_root_canon = std::fs::canonicalize(&omp_root).expect("omp root canon");
    let omp_agent_canon = std::fs::canonicalize(&omp_agent).expect("omp agent canon");
    let mut expected_omp_writable = vec![
        std::fs::canonicalize(&omp.cwd).expect("cwd"),
        omp_agent_canon.join("a.db"),
        omp_agent_canon.join("a.db-wal"),
        omp_root.join("agent/a.db-shm"),
        omp_agent_canon.join("b.db"),
        omp_root.join("agent/b.db-wal"),
        omp_root.join("agent/b.db-shm"),
        omp_agent_canon.join("dir.db"),
        omp_root_canon.join("stats.db"),
        omp_root.join("stats.db-wal"),
        omp_root.join("stats.db-shm"),
    ];
    for name in [
        "cache",
        "logs",
        "run",
        "agent/sessions",
        "agent/projects",
        "agent/cache",
        "agent/blobs",
    ] {
        expected_omp_writable.push(omp_root.join(name));
    }
    expected_omp_writable.sort();
    assert_eq!(omp_plan.writable, expected_omp_writable);
    let mut expected_omp_skipped = vec![
        omp_root.join("agent/a.db-shm"),
        omp_root.join("agent/b.db-wal"),
        omp_root.join("agent/b.db-shm"),
        omp_root.join("agent/dir.db-wal"),
        omp_root.join("agent/dir.db-shm"),
        omp_root.join("stats.db-wal"),
        omp_root.join("stats.db-shm"),
    ];
    expected_omp_skipped.sort();
    let omp_home_skipped: Vec<PathBuf> = omp_plan
        .skipped_paths()
        .iter()
        .filter(|path| path.starts_with(&omp_root))
        .cloned()
        .collect();
    assert_eq!(omp_home_skipped, expected_omp_skipped);

    // OMP without stats.db (or stats.db as a directory): stats.db and its sidecars are not listed.
    let omp_no_stats = Fixture::new();
    std::fs::create_dir_all(omp_no_stats.home.join(".omp/stats.db")).expect("stats.db dir");
    let omp_no_stats_plan = confine::plan(
        &ConfineRequest {
            unhide: vec!["~/.omp".into()],
            no_default_hides: true,
            ..ConfineRequest::default()
        },
        Harness::Omp,
        &omp_no_stats.cwd,
        &omp_no_stats.host(),
    )
    .expect("OMP no stats plan");
    assert!(
        !omp_no_stats_plan
            .writable
            .iter()
            .any(|p| p.ends_with("stats.db"))
    );
    let omp_no_stats_skipped: Vec<PathBuf> = omp_no_stats_plan
        .skipped_paths()
        .iter()
        .filter(|path| path.starts_with(omp_no_stats.home.join(".omp")))
        .cloned()
        .collect();
    assert_eq!(omp_no_stats_skipped, Vec::<PathBuf>::new());

    // Agy: only antigravity-cli is added.
    let agy = Fixture::new();
    std::fs::create_dir_all(agy.home.join(".gemini/antigravity-cli")).expect("agy data");
    let agy_plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Agy,
        &agy.cwd,
        &agy.host(),
    )
    .expect("Agy plan");
    let mut expected_agy = vec![
        std::fs::canonicalize(&agy.cwd).expect("cwd"),
        std::fs::canonicalize(agy.home.join(".gemini/antigravity-cli")).expect("agy data canon"),
    ];
    expected_agy.sort();
    assert_eq!(agy_plan.writable, expected_agy);

    // Generic and Codex open nothing as harness data.
    let generic = Fixture::new();
    let generic_plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &generic.cwd,
        &generic.host(),
    )
    .expect("Generic plan");
    assert_eq!(
        generic_plan.writable,
        vec![std::fs::canonicalize(&generic.cwd).expect("cwd")]
    );

    let codex = Fixture::new();
    std::fs::create_dir_all(codex.home.join(".codex")).expect("codex root");
    let codex_plan = confine::plan(
        &ConfineRequest {
            hide: vec!["~/.codex".into()],
            ..ConfineRequest::default()
        },
        Harness::Codex,
        &codex.cwd,
        &codex.host(),
    )
    .expect("Codex plan");
    assert_eq!(
        codex_plan.writable,
        vec![std::fs::canonicalize(&codex.cwd).expect("cwd")]
    );
}

#[test]
fn harness_data_mounts_skip_missing_or_non_dir_root_and_refuse_dangling_dir_link() {
    // Missing root and non-directory root do nothing for data mounts.
    let missing = Fixture::new();
    let missing_plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &missing.cwd,
        &missing.host(),
    )
    .expect("missing root plan");
    assert_eq!(
        missing_plan.writable,
        vec![std::fs::canonicalize(&missing.cwd).expect("cwd")]
    );

    let file_root = Fixture::new();
    std::fs::write(file_root.home.join(".claude"), "not a dir").expect("file root");
    let file_root_plan = confine::plan(
        &ConfineRequest {
            unhide: vec!["~/.claude".into()],
            ..ConfineRequest::default()
        },
        Harness::Claude,
        &file_root.cwd,
        &file_root.host(),
    )
    .expect("file root plan");
    assert_eq!(
        file_root_plan.writable,
        vec![std::fs::canonicalize(&file_root.cwd).expect("cwd")]
    );

    // Dangling symlink in a data directory path is refused with exact text.
    let dangling = Fixture::new();
    let claude_root = dangling.home.join(".claude");
    std::fs::create_dir_all(&claude_root).expect("claude root");
    let projects = claude_root.join("projects");
    symlink(dangling.home.join("does-not-exist"), &projects).expect("dangling symlink");
    let error = confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &dangling.cwd,
        &dangling.host(),
    )
    .err()
    .expect("dangling data dir symlink");
    assert_eq!(
        error.to_string(),
        format!(
            "harness data path cannot be canonicalized: {}",
            projects.display()
        )
    );

    // Missing data file is recorded in skipped (not writable).
    let missing_file = Fixture::new();
    std::fs::create_dir_all(missing_file.home.join(".claude")).expect("claude root");
    let missing_file_plan = confine::plan(
        &ConfineRequest {
            unhide: vec!["~/.claude".into()],
            ..ConfineRequest::default()
        },
        Harness::Claude,
        &missing_file.cwd,
        &missing_file.host(),
    )
    .expect("missing data file plan");
    assert!(
        missing_file_plan
            .skipped_paths()
            .contains(&missing_file.home.join(".claude/history.jsonl"))
    );
    assert!(
        !missing_file_plan
            .writable
            .contains(&missing_file.home.join(".claude/history.jsonl"))
    );
}

#[test]
fn harness_data_validation_checks_claude_home_file_and_check_order() {
    // ~/.claude.json symlink to a directory is refused with home as root.
    let dir_link = Fixture::new();
    let sub = dir_link.home.join("sub");
    std::fs::create_dir_all(&sub).expect("sub dir");
    let claude_json = dir_link.home.join(".claude.json");
    symlink(&sub, &claude_json).expect("claude.json dir symlink");
    let error = confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &dir_link.cwd,
        &dir_link.host(),
    )
    .err()
    .expect("claude.json dir symlink");
    let home_canon = std::fs::canonicalize(&dir_link.home).expect("home canon");
    let sub_canon = std::fs::canonicalize(&sub).expect("sub canon");
    assert_eq!(
        error.to_string(),
        format!(
            "harness data path {} resolves to {}, outside {}; remove the link, keep the data inside {}, or open it with write = [\"{}\"] if that is intended",
            claude_json.display(),
            sub_canon.display(),
            home_canon.display(),
            home_canon.display(),
            sub_canon.display(),
        )
    );

    // ~/.claude.json symlink to a file in a subdirectory is refused with home as root.
    let nested_file = Fixture::new();
    let nested_dir = nested_file.home.join("sub");
    std::fs::create_dir_all(&nested_dir).expect("sub dir");
    let nested_target = nested_dir.join("other.json");
    std::fs::write(&nested_target, "{}").expect("nested target");
    let claude_json = nested_file.home.join(".claude.json");
    symlink(&nested_target, &claude_json).expect("nested claude.json symlink");
    let error = confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &nested_file.cwd,
        &nested_file.host(),
    )
    .err()
    .expect("claude.json nested file symlink");
    let home_canon = std::fs::canonicalize(&nested_file.home).expect("home canon");
    let nested_canon = std::fs::canonicalize(&nested_target).expect("nested canon");
    assert_eq!(
        error.to_string(),
        format!(
            "harness data path {} resolves to {}, outside {}; remove the link, keep the data inside {}, or open it with write = [\"{}\"] if that is intended",
            claude_json.display(),
            nested_canon.display(),
            home_canon.display(),
            home_canon.display(),
            nested_canon.display(),
        )
    );

    // Explicit write skips inside check, but check_write_allowed and check_write_runtime_allowed still fire.
    let runtime_link = Fixture::new();
    let claude_root = runtime_link.home.join(".claude");
    std::fs::create_dir_all(&claude_root).expect("claude root");
    let runtime_target = runtime_link.runtime.join("rt-dir");
    std::fs::create_dir_all(&runtime_target).expect("rt dir");
    symlink(&runtime_target, claude_root.join("projects")).expect("rt symlink");
    let error = confine::plan(
        &ConfineRequest {
            write: vec![runtime_target.to_string_lossy().into_owned()],
            ..ConfineRequest::default()
        },
        Harness::Claude,
        &runtime_link.cwd,
        &runtime_link.host(),
    )
    .err()
    .expect("explicit write still checked against runtime_dir");
    let runtime_target_canon = std::fs::canonicalize(&runtime_target).expect("rt canon");
    assert_eq!(
        error.to_string(),
        format!(
            "writable path {} overlaps the runtime directory",
            runtime_target_canon.display()
        )
    );

    // Check order: inside check fires BEFORE check_write_allowed when not explicit.
    let state_link = Fixture::new();
    let claude_root = state_link.home.join(".claude");
    std::fs::create_dir_all(&claude_root).expect("claude root");
    let state_target = state_link.state.join("sub");
    std::fs::create_dir_all(&state_target).expect("state sub");
    let projects = claude_root.join("projects");
    symlink(&state_target, &projects).expect("state symlink");
    let error = confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &state_link.cwd,
        &state_link.host(),
    )
    .err()
    .expect("inside check fires before check_write_allowed");
    assert!(error.to_string().contains("outside"));

    // Path order: sources -> dirs -> files.
    // OMP: source (~/.omp/agent -> state) fails before dir (~/.omp/cache -> outside).
    let src_vs_dir = Fixture::new();
    let omp_root = src_vs_dir.home.join(".omp");
    std::fs::create_dir_all(&omp_root).expect("omp root");
    symlink(&src_vs_dir.state, omp_root.join("agent")).expect("omp agent state symlink");
    let outside_cache = src_vs_dir._root.path().join("outside-cache");
    std::fs::create_dir_all(&outside_cache).expect("outside cache");
    symlink(&outside_cache, omp_root.join("cache")).expect("omp cache escape");
    let error = confine::plan(
        &ConfineRequest::default(),
        Harness::Omp,
        &src_vs_dir.cwd,
        &src_vs_dir.host(),
    )
    .err()
    .expect("source checked before dirs");
    assert!(
        error.to_string().contains("overlaps the a2amx directory"),
        "expected source (.omp/agent -> state) to fail before dir (.omp/cache): {error}"
    );

    let dir_vs_file = Fixture::new();
    let claude_root = dir_vs_file.home.join(".claude");
    std::fs::create_dir_all(&claude_root).expect("claude root");
    let escape_dir = dir_vs_file._root.path().join("escape-dir");
    let escape_file = dir_vs_file._root.path().join("escape-file");
    std::fs::create_dir_all(&escape_dir).expect("escape dir");
    std::fs::write(&escape_file, "x").expect("escape file");
    let projects = claude_root.join("projects");
    symlink(&escape_dir, &projects).expect("projects escape");
    symlink(&escape_file, claude_root.join("history.jsonl")).expect("history escape");
    let error = confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &dir_vs_file.cwd,
        &dir_vs_file.host(),
    )
    .err()
    .expect("dirs checked before files");
    assert!(
        error.to_string().contains(&projects.display().to_string()),
        "expected projects to fail before history.jsonl: {error}"
    );
}

#[test]
fn ensure_data_dirs_and_files_pin_errors_and_permissions() {
    // ensure_data_dirs: existing dir with 0755 keeps 0755; data path as file is refused with exact text.
    let fixture = Fixture::new();
    let gemini = fixture.home.join(".gemini");
    std::fs::create_dir_all(&gemini).expect("gemini");
    let agy_dir = gemini.join("antigravity-cli");
    std::fs::create_dir_all(&agy_dir).expect("agy dir");
    std::fs::set_permissions(&agy_dir, std::fs::Permissions::from_mode(0o755)).expect("chmod 0755");
    confine::ensure_data_dirs(Harness::Agy, &fixture.home).expect("existing dir");
    assert_eq!(
        std::fs::metadata(&agy_dir)
            .expect("meta")
            .permissions()
            .mode()
            & 0o777,
        0o755
    );

    let file_err = Fixture::new();
    let gemini_file = file_err.home.join(".gemini");
    std::fs::create_dir_all(&gemini_file).expect("gemini");
    let agy_file = gemini_file.join("antigravity-cli");
    std::fs::write(&agy_file, "not a dir").expect("file");
    let error =
        confine::ensure_data_dirs(Harness::Agy, &file_err.home).expect_err("data path is a file");
    assert_eq!(
        error.to_string(),
        format!(
            "harness data path is not a directory: {}",
            agy_file.display()
        )
    );

    // ensure_data_files: keeps existing sidecar mode 0644, and skips a .db directory.
    let omp = Fixture::new();
    let agent = omp.home.join(".omp/agent");
    std::fs::create_dir_all(&agent).expect("agent");
    let db = agent.join("a.db");
    std::fs::write(&db, "db").expect("db");
    let wal = agent.join("a.db-wal");
    std::fs::write(&wal, "wal").expect("wal");
    std::fs::set_permissions(&wal, std::fs::Permissions::from_mode(0o644)).expect("chmod 0644");
    std::fs::create_dir_all(agent.join("dir.db")).expect("dir.db");
    confine::ensure_data_files(Harness::Omp, &omp.home).expect("ensure_data_files");
    assert_eq!(
        std::fs::metadata(&wal)
            .expect("wal meta")
            .permissions()
            .mode()
            & 0o777,
        0o644
    );
    assert!(!agent.join("dir.db-wal").exists());
    assert!(!agent.join("a.db-wal-wal").exists());
    assert!(!agent.join("a.db-wal-shm").exists());
}

#[test]
fn codex_ignores_symlinked_root_and_claude_allows_home_file_link_and_explicit_write_hits_state_guard()
 {
    // F7: symlinked ~/.codex pointing into state is neither refused by validation nor opened as data.
    let codex = Fixture::new();
    let state_codex = codex.state.join("codex-dir");
    std::fs::create_dir_all(&state_codex).expect("state codex");
    symlink(&state_codex, codex.home.join(".codex")).expect("codex symlink");
    let codex_plan = confine::plan(
        &ConfineRequest {
            hide: vec!["~/.codex".into()],
            ..ConfineRequest::default()
        },
        Harness::Codex,
        &codex.cwd,
        &codex.host(),
    )
    .expect("symlinked ~/.codex is not refused by harness data validation");
    assert_eq!(
        codex_plan.writable,
        vec![std::fs::canonicalize(&codex.cwd).expect("cwd")]
    );

    // F8a: symlink at ~/.claude.json pointing to a regular file directly in home is accepted and canonical target is writable.
    let claude_link = Fixture::new();
    std::fs::create_dir_all(claude_link.home.join(".claude")).expect("claude root");
    let real_claude_json = claude_link.home.join("real-claude.json");
    std::fs::write(&real_claude_json, "{}").expect("real claude.json");
    symlink(&real_claude_json, claude_link.home.join(".claude.json"))
        .expect("claude.json home symlink");
    let claude_link_plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &claude_link.cwd,
        &claude_link.host(),
    )
    .expect("symlinked ~/.claude.json to file in home is accepted");
    assert!(
        claude_link_plan
            .writable
            .contains(&std::fs::canonicalize(&real_claude_json).expect("real claude.json canon"))
    );

    // F8b: explicit write entry for a harness data path skips inside check but still hits check_write_allowed
    // in validate_harness_data_paths before bind_operator_writes processes an earlier runtime write entry.
    let explicit_state = Fixture::new();
    let claude_root = explicit_state.home.join(".claude");
    std::fs::create_dir_all(&claude_root).expect("claude root");
    let runtime_first = explicit_state.runtime.join("rt-first");
    std::fs::create_dir_all(&runtime_first).expect("runtime first");
    let state_target = explicit_state.state.join("explicit-target");
    std::fs::create_dir_all(&state_target).expect("state target");
    symlink(&state_target, claude_root.join("projects")).expect("projects state symlink");
    let error = confine::plan(
        &ConfineRequest {
            write: vec![
                runtime_first.to_string_lossy().into_owned(),
                state_target.to_string_lossy().into_owned(),
            ],
            ..ConfineRequest::default()
        },
        Harness::Claude,
        &explicit_state.cwd,
        &explicit_state.host(),
    )
    .err()
    .expect("explicit write under state dir must still fail check_write_allowed");
    let state_target_canon = std::fs::canonicalize(&state_target).expect("state target canon");
    assert_eq!(
        error.to_string(),
        format!(
            "writable path {} overlaps the a2amx directory",
            state_target_canon.display()
        )
    );
}

#[test]
fn existing_data_dir_symlinked_elsewhere_keeps_its_target_mode() {
    let fixture = Fixture::new();
    let gemini = fixture.home.join(".gemini");
    std::fs::create_dir_all(&gemini).expect("gemini");
    let target = fixture.home.join("outside-agy");
    std::fs::create_dir_all(&target).expect("outside target");
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))
        .expect("chmod 0755 target");
    let agy_dir = gemini.join("antigravity-cli");
    symlink(&target, &agy_dir).expect("agy symlink");

    confine::ensure_data_dirs(Harness::Agy, &fixture.home).expect("ensure_data_dirs");
    assert!(
        std::fs::symlink_metadata(&agy_dir)
            .expect("link metadata")
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::metadata(&target)
            .expect("target metadata")
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
}

#[test]
fn dangling_data_dir_symlink_is_left_for_the_plan() {
    let fixture = Fixture::new();
    let gemini = fixture.home.join(".gemini");
    std::fs::create_dir_all(&gemini).expect("gemini");
    let target = fixture.home.join("missing-agy-target");
    let agy_dir = gemini.join("antigravity-cli");
    symlink(&target, &agy_dir).expect("dangling agy symlink");

    confine::ensure_data_dirs(Harness::Agy, &fixture.home).expect("ensure_data_dirs");
    assert!(
        std::fs::symlink_metadata(&agy_dir)
            .expect("link metadata")
            .file_type()
            .is_symlink()
    );
    assert!(std::fs::symlink_metadata(&target).is_err());
}

#[test]
fn missing_data_dir_is_created_private_with_default_parents() {
    let fixture = Fixture::new();
    let omp = fixture.home.join(".omp");
    std::fs::create_dir_all(&omp).expect("omp");
    let agent = omp.join("agent");
    assert!(!agent.exists());

    confine::ensure_data_dirs(Harness::Omp, &fixture.home).expect("ensure_data_dirs");
    assert!(agent.is_dir());
    let sessions = agent.join("sessions");
    assert!(sessions.is_dir());
    assert_eq!(
        std::fs::metadata(&sessions)
            .expect("sessions metadata")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
}

#[test]
fn sidecar_symlinked_to_a_file_keeps_that_files_mode() {
    let fixture = Fixture::new();
    let agent = fixture.home.join(".omp/agent");
    std::fs::create_dir_all(&agent).expect("agent");
    std::fs::write(agent.join("a.db"), "db").expect("db");
    let target = fixture.home.join("outside-wal");
    std::fs::write(&target, "wal").expect("outside wal");
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644))
        .expect("chmod 0644 target");
    let wal = agent.join("a.db-wal");
    symlink(&target, &wal).expect("wal symlink");

    confine::ensure_data_files(Harness::Omp, &fixture.home).expect("ensure_data_files");
    assert!(
        std::fs::symlink_metadata(&wal)
            .expect("link metadata")
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::metadata(&target)
            .expect("target metadata")
            .permissions()
            .mode()
            & 0o777,
        0o644
    );
}

#[test]
fn dangling_sidecar_symlink_still_fails_the_launch() {
    let fixture = Fixture::new();
    let agent = fixture.home.join(".omp/agent");
    std::fs::create_dir_all(&agent).expect("agent");
    std::fs::write(agent.join("a.db"), "db").expect("db");
    let target = fixture.home.join("missing-wal-target");
    let wal = agent.join("a.db-wal");
    symlink(&target, &wal).expect("dangling wal symlink");

    let error =
        confine::ensure_data_files(Harness::Omp, &fixture.home).expect_err("dangling sidecar");
    assert_eq!(
        error
            .downcast_ref::<std::io::Error>()
            .map(std::io::Error::kind),
        Some(std::io::ErrorKind::AlreadyExists)
    );
    assert!(std::fs::symlink_metadata(&target).is_err());
}

#[test]
fn data_dir_symlink_loop_still_fails() {
    let fixture = Fixture::new();
    let gemini = fixture.home.join(".gemini");
    std::fs::create_dir_all(&gemini).expect("gemini");
    let agy_dir = gemini.join("antigravity-cli");
    symlink(&agy_dir, &agy_dir).expect("self-looping symlink");

    assert!(confine::ensure_data_dirs(Harness::Agy, &fixture.home).is_err());
}
