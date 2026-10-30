#![allow(clippy::expect_used, clippy::unwrap_used)]
use anyhow::Context as _;
use std::ffi::OsString;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};

use a2amx::client::Client;
use a2amx::confine::{self, ConfineRequest, Host};
use a2amx::daemon::{Daemon, DaemonConfig};
use a2amx::harness::Harness;
use a2amx::messaging::Limits;
use a2amx::wire::{Request, Response};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::{Duration, timeout};

struct Fixture {
    _root: TempDir,
    home: PathBuf,
    state: PathBuf,
    runtime: PathBuf,
    cwd: PathBuf,
    empty: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("tempdir");
        let home = root.path().join("home");
        let state = root.path().join("state");
        let runtime = root.path().join("runtime");
        let cwd = root.path().join("workspace");
        for path in [&home, &state, &runtime, &cwd] {
            std::fs::create_dir_all(path).expect("fixture directory");
        }
        std::fs::create_dir_all(home.join(".aws")).expect("aws directory");
        std::fs::create_dir_all(home.join(".ssh")).expect("ssh directory");
        std::fs::write(home.join(".ssh/config"), "Host example\n").expect("ssh config");
        let empty = state.join("confine-empty");
        std::fs::write(&empty, []).expect("empty file");
        Self {
            _root: root,
            home,
            state,
            runtime,
            cwd,
            empty,
        }
    }

    fn host(&self) -> Host {
        Host {
            home: self.home.clone(),
            state_dir: self.state.clone(),
            runtime_dir: self.runtime.clone(),
            empty_file: self.empty.clone(),
        }
    }
}

fn bwrap_path_dir(target: &Path) -> TempDir {
    let dir = tempfile::tempdir().expect("bwrap directory");
    symlink(target, dir.path().join("bwrap")).expect("bwrap symlink");
    dir
}

fn stub_path_dir(exit_code: i32) -> TempDir {
    let dir = tempfile::tempdir().expect("stub directory");
    let path = dir.path().join("bwrap");
    std::fs::write(&path, format!("#!/bin/sh\nexit {exit_code}\n")).expect("stub");
    let mut permissions = std::fs::metadata(&path)
        .expect("stub metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).expect("stub permissions");
    dir
}
fn probe_fail_stub_path_dir() -> TempDir {
    let dir = tempfile::tempdir().expect("probe stub directory");
    let path = dir.path().join("bwrap");
    std::fs::write(
        &path,
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then exit 0; fi\nexit 42\n",
    )
    .expect("probe stub");
    let mut permissions = std::fs::metadata(&path)
        .expect("probe stub metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).expect("probe stub permissions");
    dir
}

fn require_bwrap() -> bool {
    std::env::var_os("A2AMX_REQUIRE_BWRAP").is_some_and(|value| value == "1")
}

async fn start_confine_daemon(state: &Path, bwrap_dir: &Path) -> anyhow::Result<Daemon> {
    Daemon::start(DaemonConfig {
        state_dir: state.to_owned(),
        listen: vec!["127.0.0.1:0".parse()?],
        host_name: Some("host-a".into()),
        limits: Limits::default(),
        bwrap_path: Some(OsString::from(bwrap_dir)),
    })
    .await
    .context("start test daemon")
}

fn confined_shell_request(
    cwd: &Path,
    script: &str,
    env: Vec<(String, String)>,
    options: ConfineRequest,
) -> Request {
    Request::Confine {
        confine: options,
        request: Box::new(Request::NewSession {
            argv: vec!["sh".into(), "-c".into(), script.into()],
            cols: 80,
            rows: 24,
            cwd: Some(cwd.to_string_lossy().into_owned()),
            env,
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            activity: None,
            heartbeat: None,
            name: Some("confined-test".into()),
            harness: Harness::Generic,
            deliver: None,
            team: None,
            role: None,
        }),
    }
}

#[test]
fn plan_skips_missing_and_unhides_existing_home_secrets_read_only() {
    let fixture = Fixture::new();
    let request = ConfineRequest {
        unhide: vec![
            "~/.aws".into(),
            "~/.ssh/config".into(),
            "missing-path".into(),
        ],
        team_file: Some("missing-team.toml".into()),
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
fn mandatory_hides_remain_when_default_hides_are_disabled() {
    let fixture = Fixture::new();
    let request = ConfineRequest {
        no_default_hides: true,
        ..ConfineRequest::default()
    };
    let plan =
        confine::plan(&request, Harness::Generic, &fixture.cwd, &fixture.host()).expect("plan");
    assert!(plan.hidden_paths().contains(&fixture.state));
    assert!(plan.hidden_paths().contains(&fixture.runtime));
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
    std::fs::create_dir_all(fixture.state.join("omp")).expect("omp state");
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
    let omp = fixture.state.join("omp").to_string_lossy().into_owned();
    let wrapped = plan
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("wrap");

    let state_hide = wrapped
        .windows(2)
        .position(|window| window == ["--tmpfs", fixture.state.to_string_lossy().as_ref()]);
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
fn wrap_unsets_base_and_container_environment_in_stable_order() {
    let fixture = Fixture::new();
    let restricted = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("plan")
    .with_bwrap(PathBuf::from("bwrap"))
    .wrap(vec!["sh".into(), "-c".into(), "true".into()])
    .expect("wrap");
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
    ] {
        assert!(
            restricted
                .windows(2)
                .any(|window| window == ["--unsetenv", name])
        );
    }
    let allowed = confine::plan(
        &ConfineRequest {
            allow_containers: true,
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("plan")
    .with_bwrap(PathBuf::from("bwrap"))
    .wrap(vec!["true".into()])
    .expect("wrap");
    assert!(
        !allowed
            .windows(2)
            .any(|window| window == ["--unsetenv", "DOCKER_HOST"])
    );
    assert!(
        allowed
            .windows(2)
            .any(|window| window == ["--tmpfs", fixture.runtime.to_string_lossy().as_ref()])
    );
}

#[test]
fn wrap_enables_user_ipc_and_pid_namespaces() {
    let fixture = Fixture::new();
    let wrapped = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("plan")
    .with_bwrap(PathBuf::from("bwrap"))
    .wrap(vec!["true".into()])
    .expect("wrap");
    for flag in ["--unshare-user", "--unshare-ipc", "--unshare-pid"] {
        assert!(wrapped.iter().any(|argument| argument == flag), "{flag}");
    }
}

#[test]
fn remount_ro_options_follow_all_mount_operations() {
    let fixture = Fixture::new();
    let hidden = fixture.cwd.join("hidden");
    std::fs::create_dir_all(&hidden).expect("hidden directory");
    let plan = confine::plan(
        &ConfineRequest {
            hide: vec![hidden.to_string_lossy().into_owned()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("plan");
    let wrapped = plan
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("wrap");
    let last_mount = wrapped
        .iter()
        .enumerate()
        .filter(|(_, argument)| {
            matches!(
                argument.as_str(),
                "--bind" | "--ro-bind" | "--tmpfs" | "--dev-bind" | "--proc" | "--dir"
            )
        })
        .map(|(index, _)| index)
        .max()
        .expect("mount operation");
    let remounts = wrapped
        .iter()
        .enumerate()
        .filter(|(_, argument)| argument.as_str() == "--remount-ro")
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    assert!(!remounts.is_empty());
    assert!(remounts.iter().all(|index| *index > last_mount));
}

#[test]
fn unhide_cannot_lift_container_socket_restriction() {
    let fixture = Fixture::new();
    let docker_socket = fixture.home.join(".docker/run/docker.sock");
    std::fs::create_dir_all(docker_socket.parent().expect("docker parent")).expect("docker dir");
    std::fs::write(&docker_socket, "socket placeholder").expect("docker socket fixture");
    let error = match confine::plan(
        &ConfineRequest {
            unhide: vec!["~/.docker/run/docker.sock".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    ) {
        Ok(_) => panic!("container socket unhide must be refused"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("--confine-allow-containers"));
    let allowed = confine::plan(
        &ConfineRequest {
            unhide: vec!["~/.docker/run/docker.sock".into()],
            allow_containers: true,
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("allow-containers switch");
    assert!(
        allowed
            .readonly
            .contains(&std::fs::canonicalize(&docker_socket).expect("socket canonical"))
    );
    let plan = confine::plan(
        &ConfineRequest {
            unhide: vec!["~/.docker".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("parent unhide");
    assert!(
        plan.hidden_paths()
            .contains(&std::fs::canonicalize(docker_socket).expect("socket canonical"))
    );
}
#[test]
fn state_omp_cannot_be_operator_unhidden() {
    let fixture = Fixture::new();
    let omp_state = fixture.state.join("omp");
    std::fs::create_dir_all(&omp_state).expect("omp state");
    for harness in [Harness::Generic, Harness::Claude] {
        let result = confine::plan(
            &ConfineRequest {
                unhide: vec![omp_state.to_string_lossy().into_owned()],
                ..ConfineRequest::default()
            },
            harness,
            &fixture.cwd,
            &fixture.host(),
        );
        assert!(result.is_err(), "{harness:?} must not unhide state/omp");
    }
    let plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Omp,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("omp plan");
    let wrapped = plan
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("wrapped omp plan");
    let state = std::fs::canonicalize(omp_state).expect("omp state canonical");
    assert!(wrapped.windows(3).any(|window| {
        window
            == [
                "--ro-bind",
                state.to_string_lossy().as_ref(),
                state.to_string_lossy().as_ref(),
            ]
    }));
}

#[test]
fn unhide_guard_canonicalizes_symlinked_state_runtime_and_container_paths() {
    let fixture = Fixture::new();
    let state_link = fixture.cwd.join("state-link");
    let runtime_link = fixture.cwd.join("runtime-link");
    symlink(&fixture.state, &state_link).expect("state symlink");
    symlink(&fixture.runtime, &runtime_link).expect("runtime symlink");
    for link in [&state_link, &runtime_link] {
        assert!(
            confine::check_unhide_allowed(link, &fixture.state, &fixture.runtime, &fixture.home)
                .is_err(),
            "symlinked protected path must be refused: {}",
            link.display()
        );
    }

    let socket = fixture.home.join(".docker/run/docker.sock");
    std::fs::create_dir_all(socket.parent().expect("socket parent")).expect("socket directory");
    std::fs::write(&socket, "socket").expect("socket fixture");
    let socket_link = fixture.cwd.join("socket-link");
    symlink(&socket, &socket_link).expect("socket symlink");
    assert!(
        confine::plan(
            &ConfineRequest {
                unhide: vec![socket_link.to_string_lossy().into_owned()],
                ..ConfineRequest::default()
            },
            Harness::Generic,
            &fixture.cwd,
            &fixture.host()
        )
        .is_err(),
        "symlinked container socket must be refused"
    );

    let secret = fixture._root.path().join("secret");
    std::fs::write(&secret, "secret").expect("secret fixture");
    let secret_link = fixture.cwd.join("secret-link");
    symlink(&secret, &secret_link).expect("secret symlink");
    let plan = confine::plan(
        &ConfineRequest {
            unhide: vec![secret_link.to_string_lossy().into_owned()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("symlinked secret unhide");
    let secret = std::fs::canonicalize(secret).expect("secret canonical");
    assert!(plan.readonly.contains(&secret));
    assert!(!plan.readonly.contains(&secret_link));
}

#[test]
fn writable_paths_override_exact_hides_but_not_container_sockets() {
    let fixture = Fixture::new();
    let ssh = std::fs::canonicalize(fixture.home.join(".ssh")).expect("ssh canonical");
    let plan = confine::plan(
        &ConfineRequest {
            write: vec!["~/.ssh".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("ssh write");
    assert!(plan.writable.contains(&ssh));
    assert!(!plan.hidden_paths().contains(&ssh));
    assert!(!plan.default_hidden.contains(&ssh));

    let socket = fixture.home.join(".docker/run/docker.sock");
    std::fs::create_dir_all(socket.parent().expect("socket parent")).expect("socket directory");
    std::fs::write(&socket, "socket placeholder").expect("socket fixture");
    let error = match confine::plan(
        &ConfineRequest {
            write: vec!["~/.docker/run/docker.sock".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    ) {
        Ok(_) => panic!("container socket write must be refused"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("--confine-allow-containers"));
    let allowed = confine::plan(
        &ConfineRequest {
            write: vec!["~/.docker/run/docker.sock".into()],
            allow_containers: true,
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("container socket write allowed");
    assert!(
        allowed
            .writable
            .contains(&std::fs::canonicalize(socket).expect("socket canonical"))
    );
}

#[test]
fn container_socket_hides_are_bound_and_can_be_disabled() {
    let fixture = Fixture::new();
    let sockets = [
        fixture.home.join(".docker/run/docker.sock"),
        fixture.home.join(".docker/desktop/docker.sock"),
    ];
    for socket in &sockets {
        std::fs::create_dir_all(socket.parent().expect("socket parent")).expect("socket dir");
        std::fs::write(socket, "socket placeholder").expect("socket fixture");
    }
    let plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("container hide plan");
    for socket in &sockets {
        let socket = std::fs::canonicalize(socket).expect("socket canonical");
        assert!(plan.hidden_paths().contains(&socket));
    }
    let wrapped = plan
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("wrapped container plan");
    for socket in &sockets {
        let socket = std::fs::canonicalize(socket).expect("socket canonical");
        assert!(wrapped.windows(3).any(|window| {
            window
                == [
                    "--ro-bind",
                    fixture.empty.to_string_lossy().as_ref(),
                    socket.to_string_lossy().as_ref(),
                ]
        }));
    }
    let allowed = confine::plan(
        &ConfineRequest {
            allow_containers: true,
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("allow containers plan");
    for socket in &sockets {
        let socket = std::fs::canonicalize(socket).expect("socket canonical");
        assert!(!allowed.hidden_paths().contains(&socket));
    }
    let allowed_wrapped = allowed
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("wrapped allow containers plan");
    for socket in &sockets {
        let socket = std::fs::canonicalize(socket).expect("socket canonical");
        assert!(
            !allowed_wrapped
                .iter()
                .any(|argument| argument == socket.to_string_lossy().as_ref())
        );
    }
}

#[test]
fn codex_is_refused_and_write_guards_are_reciprocal() {
    let fixture = Fixture::new();
    assert!(
        confine::plan(
            &ConfineRequest::default(),
            Harness::Codex,
            &fixture.cwd,
            &fixture.host()
        )
        .is_err()
    );
    assert!(confine::check_write_allowed(&fixture.state, &fixture.home, &fixture.state).is_err());
    assert!(
        confine::check_write_allowed(
            fixture.state.parent().expect("parent"),
            &fixture.home,
            &fixture.state
        )
        .is_err()
    );
    assert!(confine::check_write_allowed(&fixture.runtime, &fixture.home, &fixture.state).is_ok());
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
    std::fs::write(fixture.state.join("admin.token"), "admin").expect("admin token");
    for path in [
        fixture.state.join("admin.token"),
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
fn harness_data_allowlists_do_not_bind_roots_or_code_paths() {
    let claude = Fixture::new();
    let claude_root = claude.home.join(".claude");
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
        std::fs::create_dir_all(claude_root.join(name)).expect("Claude data directory");
    }
    for name in ["history.jsonl", "stats-cache.json", ".credentials.json"] {
        std::fs::write(claude_root.join(name), "fixture").expect("Claude data file");
    }
    std::fs::write(claude.home.join(".claude.json"), "fixture").expect("Claude user data file");
    for path in [
        claude_root.join("settings.json"),
        claude_root.join("hooks"),
        claude_root.join("commands"),
        claude_root.join("agents"),
        claude_root.join("skills"),
    ] {
        if path.extension().is_some() {
            std::fs::write(path, "fixture").expect("Claude code path");
        } else {
            std::fs::create_dir_all(path).expect("Claude code directory");
        }
    }
    let claude_plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &claude.cwd,
        &claude.host(),
    )
    .expect("Claude plan");
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
        "history.jsonl",
        "stats-cache.json",
        ".credentials.json",
    ] {
        let path = std::fs::canonicalize(claude_root.join(name)).expect("Claude allow-list path");
        assert!(
            claude_plan.writable.contains(&path),
            "Claude data path must be writable: {path:?}"
        );
    }
    let claude_json =
        std::fs::canonicalize(claude.home.join(".claude.json")).expect("Claude JSON path");
    assert!(claude_plan.writable.contains(&claude_json));
    for path in [
        claude_root.clone(),
        claude_root.join("settings.json"),
        claude_root.join("hooks"),
        claude_root.join("commands"),
        claude_root.join("agents"),
        claude_root.join("skills"),
    ] {
        let path = std::fs::canonicalize(path).expect("Claude protected path");
        assert!(
            !claude_plan.writable.contains(&path),
            "Claude code path must remain read-only: {path:?}"
        );
    }

    let omp = Fixture::new();
    let omp_root = omp.home.join(".omp");
    for name in [
        "cache",
        "logs",
        "run",
        "agent/sessions",
        "agent/projects",
        "agent/cache",
        "agent/blobs",
    ] {
        std::fs::create_dir_all(omp_root.join(name)).expect("OMP data directory");
    }
    for path in [
        omp_root.join("agent/a.db"),
        omp_root.join("agent/a.db-wal"),
        omp_root.join("agent/a.db-shm"),
        omp_root.join("stats.db"),
        omp_root.join("stats.db-wal"),
        omp_root.join("stats.db-shm"),
    ] {
        std::fs::write(path, "fixture").expect("OMP data file");
    }
    std::fs::create_dir_all(omp_root.join("extensions")).expect("OMP extension directory");
    std::fs::write(omp_root.join("config.yml"), "fixture").expect("OMP config");
    let omp_plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Omp,
        &omp.cwd,
        &omp.host(),
    )
    .expect("OMP plan");
    for name in [
        "cache",
        "logs",
        "run",
        "agent/sessions",
        "agent/projects",
        "agent/cache",
        "agent/blobs",
        "agent/a.db",
        "agent/a.db-wal",
        "agent/a.db-shm",
        "stats.db",
        "stats.db-wal",
        "stats.db-shm",
    ] {
        let path = std::fs::canonicalize(omp_root.join(name)).expect("OMP allow-list path");
        assert!(
            omp_plan.writable.contains(&path),
            "OMP data path must be writable: {path:?}"
        );
    }
    for path in [
        omp_root.clone(),
        omp_root.join("agent"),
        omp_root.join("extensions"),
        omp_root.join("config.yml"),
    ] {
        let path = std::fs::canonicalize(path).expect("OMP protected path");
        assert!(
            !omp_plan.writable.contains(&path),
            "OMP code/config path must remain read-only: {path:?}"
        );
    }
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
    let below_total_cap = vec!["x".repeat(130_000), "y".repeat(130_000)];
    assert!(plan().wrap(below_total_cap).is_ok());
}

#[test]
fn canonical_paths_cover_symlinked_workspace_and_hide_targets() {
    let fixture = Fixture::new();
    let cwd_link = fixture.cwd.with_file_name("workspace-link");
    symlink(&fixture.cwd, &cwd_link).expect("cwd symlink");
    let hide_target = fixture.cwd.with_file_name("hide-target");
    std::fs::create_dir_all(&hide_target).expect("hide target");
    let hide_link = fixture.cwd.with_file_name("hide-link");
    symlink(&hide_target, &hide_link).expect("hide symlink");
    let state_link = fixture.cwd.with_file_name("state-link");
    symlink(&fixture.state, &state_link).expect("state symlink");
    assert!(confine::check_write_allowed(&state_link, &fixture.home, &fixture.state).is_err());
    let runtime_link = fixture.cwd.with_file_name("runtime-link");
    symlink(&fixture.runtime, &runtime_link).expect("runtime symlink");
    assert!(
        confine::plan(
            &ConfineRequest {
                write: vec![runtime_link.to_string_lossy().into_owned()],
                ..ConfineRequest::default()
            },
            Harness::Generic,
            &fixture.cwd,
            &fixture.host()
        )
        .is_err()
    );
    let plan = confine::plan(
        &ConfineRequest {
            hide: vec![hide_link.to_string_lossy().into_owned()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &cwd_link,
        &fixture.host(),
    )
    .expect("plan");
    assert_eq!(
        plan.cwd,
        std::fs::canonicalize(&fixture.cwd).expect("cwd canonical")
    );
    assert!(
        plan.hidden_paths()
            .contains(&std::fs::canonicalize(hide_link).expect("hide canonical"))
    );
}

#[test]
fn git_common_directory_and_exact_writes_override_protection() {
    let fixture = Fixture::new();
    let repo = fixture.cwd.join("repo");
    let nested = repo.join("src");
    let git = repo.join(".git");
    let git_config = git.join("config");
    let team_file = nested.join("team.toml");
    std::fs::create_dir_all(&git).expect("git directory");
    std::fs::create_dir_all(&nested).expect("repo subdirectory");
    std::fs::write(&git_config, "[core]\n").expect("git config");
    std::fs::write(&team_file, "[[session]]\nname = \"worker\"\n").expect("team file");

    let plan = confine::plan(
        &ConfineRequest {
            write: vec![git_config.to_string_lossy().into_owned()],
            team_file: Some(team_file.to_string_lossy().into_owned()),
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &nested,
        &fixture.host(),
    )
    .expect("plan");
    let repo_git = std::fs::canonicalize(&git).expect("git canonical");
    let config = std::fs::canonicalize(&git_config).expect("config canonical");
    let team = std::fs::canonicalize(&team_file).expect("team canonical");
    let repo = std::fs::canonicalize(&repo).expect("repo canonical");
    assert!(plan.writable.contains(&repo_git));
    assert!(plan.writable.contains(&config));
    assert!(!plan.writable.contains(&repo));
    assert!(!plan.readonly.contains(&config));
    assert!(!plan.protected.contains(&config));
    assert!(plan.protected.contains(&team));
    let wrapped = plan
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("wrapped plan");
    let writable_index = wrapped
        .windows(3)
        .position(|window| {
            window
                == [
                    "--bind",
                    nested.to_string_lossy().as_ref(),
                    nested.to_string_lossy().as_ref(),
                ]
        })
        .expect("cwd writable bind");
    let readonly_index = wrapped
        .windows(3)
        .position(|window| {
            window
                == [
                    "--ro-bind",
                    team.to_string_lossy().as_ref(),
                    team.to_string_lossy().as_ref(),
                ]
        })
        .expect("team protected bind");
    assert!(writable_index < readonly_index);
}
#[test]
fn exact_writes_replace_default_hides_and_equal_depth_hides_precede_binds() {
    let fixture = Fixture::new();
    let hidden = fixture.home.join("operator-hidden");
    let writable = fixture.home.join("operator-writable");
    std::fs::create_dir_all(&hidden).expect("operator hidden directory");
    std::fs::create_dir_all(&writable).expect("operator writable directory");
    assert_eq!(hidden.components().count(), writable.components().count());

    let plan = confine::plan(
        &ConfineRequest {
            write: vec!["~/.aws".into(), writable.to_string_lossy().into_owned()],
            hide: vec![hidden.to_string_lossy().into_owned()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("plan");
    let wrapped = plan
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("wrapped plan");

    let aws = fixture.home.join(".aws").to_string_lossy().into_owned();
    assert_eq!(
        wrapped
            .windows(3)
            .filter(|window| *window == ["--bind", aws.as_str(), aws.as_str()])
            .count(),
        1,
        "exact writable default-hide override must leave one bind"
    );
    assert!(
        !wrapped
            .windows(2)
            .any(|window| window == ["--tmpfs", aws.as_str()]),
        "exact writable default-hide override must remove the hide"
    );

    let hidden = hidden.to_string_lossy().into_owned();
    let writable = writable.to_string_lossy().into_owned();
    let hide_index = wrapped
        .windows(2)
        .position(|window| window == ["--tmpfs", hidden.as_str()])
        .expect("operator hide mount");
    let bind_index = wrapped
        .windows(3)
        .position(|window| window == ["--bind", writable.as_str(), writable.as_str()])
        .expect("operator writable mount");
    assert!(
        hide_index < bind_index,
        "equal-depth hides must precede binds in the literal argv"
    );
}

#[test]
fn git_hooks_and_core_hooks_path_are_protected_inside_writable_repo() {
    let fixture = Fixture::new();
    let repo = fixture.cwd.join("repo");
    let git = repo.join(".git");
    let git_hooks = git.join("hooks");
    let husky = repo.join(".husky");
    std::fs::create_dir_all(&git_hooks).expect("git hooks");
    std::fs::create_dir_all(&husky).expect("husky hooks");
    std::fs::write(git.join("config"), "[core]\nhooksPath = .husky\n").expect("git config");

    let plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &repo,
        &fixture.host(),
    )
    .expect("plan");
    assert!(
        plan.writable
            .contains(&std::fs::canonicalize(&repo).expect("repo canonical"))
    );
    assert!(
        plan.protected
            .contains(&std::fs::canonicalize(git_hooks).expect("git hooks canonical"))
    );
    assert!(
        plan.protected
            .contains(&std::fs::canonicalize(git.join("config")).expect("config canonical"))
    );
    assert!(
        plan.protected
            .contains(&std::fs::canonicalize(husky).expect("husky canonical"))
    );
}

#[test]
fn git_worktree_and_submodule_common_dirs_are_bound() {
    let fixture = Fixture::new();

    let worktree = fixture.cwd.join("worktree");
    let metadata = fixture.cwd.join("meta/worktree");
    let common = fixture.cwd.join("common.git");
    std::fs::create_dir_all(&worktree).expect("worktree");
    std::fs::create_dir_all(&metadata).expect("worktree metadata");
    std::fs::create_dir_all(&common).expect("common git");
    std::fs::write(worktree.join(".git"), "gitdir: ../meta/worktree\n").expect("gitdir marker");
    std::fs::write(metadata.join("commondir"), "../../common.git\n").expect("commondir");
    let worktree_plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &worktree,
        &fixture.host(),
    )
    .expect("worktree plan");
    assert!(
        worktree_plan
            .writable
            .contains(&std::fs::canonicalize(&common).expect("common canonical"))
    );

    let submodule = fixture.cwd.join("submodule");
    let submodule_git = fixture.cwd.join("submodule-meta");
    std::fs::create_dir_all(&submodule).expect("submodule");
    std::fs::create_dir_all(&submodule_git).expect("submodule git");
    std::fs::write(submodule.join(".git"), "gitdir: ../submodule-meta\n")
        .expect("submodule marker");
    let submodule_plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &submodule,
        &fixture.host(),
    )
    .expect("submodule plan");
    assert!(
        submodule_plan
            .writable
            .contains(&std::fs::canonicalize(&submodule_git).expect("submodule canonical"))
    );
}

#[test]
fn home_repository_is_not_bound_and_outside_team_file_stays_hidden() {
    let fixture = Fixture::new();
    std::fs::create_dir_all(fixture.home.join(".git")).expect("home git");
    let child = fixture.home.join("project");
    std::fs::create_dir_all(&child).expect("home project");
    let plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &child,
        &fixture.host(),
    )
    .expect("home child plan");
    assert!(
        plan.writable
            .iter()
            .all(|path| !path.starts_with(fixture.home.join(".git")))
    );
    let team_file = fixture.state.join("team.toml");
    std::fs::write(&team_file, "team = \"private\"\n").expect("state team file");
    let plan = confine::plan(
        &ConfineRequest {
            team_file: Some(team_file.to_string_lossy().into_owned()),
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("outside team plan");
    assert!(!plan.readonly.contains(&team_file));
    assert!(!plan.protected.contains(&team_file));
    assert!(plan.hidden_paths().contains(&fixture.state));
}

#[test]
fn notice_describes_restrictions_and_keeps_closing_sentence() {
    let fixture = Fixture::new();
    let restricted = confine::plan(
        &ConfineRequest {
            hide: vec!["missing".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("plan");
    let restricted_notice = confine::notice(&restricted);
    assert!(restricted_notice.contains("Skipped missing paths:"));
    assert!(restricted_notice.contains("Container engines are restricted."));
    assert!(restricted_notice.contains("/tmp is private"));
    assert!(restricted_notice.ends_with(confine::CLOSING_NOTICE));
    let allowed = confine::plan(
        &ConfineRequest {
            allow_containers: true,
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("plan");
    assert!(confine::notice(&allowed).contains("Container engines are allowed."));
}

#[test]
fn notice_caps_each_category_at_twenty_entries() {
    let fixture = Fixture::new();
    for (count, suffix) in [(21, "and 1 more"), (25, "and 5 more")] {
        let mut plan = confine::plan(
            &ConfineRequest::default(),
            Harness::Generic,
            &fixture.cwd,
            &fixture.host(),
        )
        .expect("plan");
        plan.writable.clear();
        plan.hidden.clear();
        plan.writable
            .extend((0..count).map(|index| PathBuf::from(format!("/cap-writable-{index}"))));
        plan.hidden
            .extend((0..count).map(|index| PathBuf::from(format!("/cap-hidden-{index}"))));
        let text = confine::notice(&plan);
        assert_eq!(text.matches("/cap-writable-").count(), 20);
        assert_eq!(text.matches("/cap-hidden-").count(), 20);
        assert!(text.contains(suffix));
        assert!(text.len() <= 12 * 1024);
        assert!(text.ends_with(confine::CLOSING_NOTICE));
    }
    let mut under_cap = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("under-cap plan");
    under_cap.writable.clear();
    under_cap
        .writable
        .extend((0..19).map(|index| PathBuf::from(format!("/under-cap-{index}"))));
    assert!(!confine::notice(&under_cap).contains("and 0 more"));
}
#[test]
fn notice_truncation_handles_multibyte_paths_without_panicking() {
    let fixture = Fixture::new();
    let mut plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("plan");
    for index in 0..20 {
        plan.writable.push(PathBuf::from(format!(
            "/path-{index}-{}",
            "é".repeat(2_000)
        )));
    }
    let text = confine::notice(&plan);
    assert!(text.len() <= 12 * 1024);
    assert!(text.ends_with(confine::CLOSING_NOTICE));
}

#[test]
fn prompt_notice_preserves_bubblewrap_separator() {
    let wrapped = vec![
        "bwrap".into(),
        "--ro-bind".into(),
        "/".into(),
        "/".into(),
        "--".into(),
        "claude".into(),
        "--append-system-prompt=old".into(),
        "--".into(),
        "after".into(),
    ];
    let result =
        a2amx::harness::append_system_prompt(wrapped, "notice", Harness::Claude, Path::new("."))
            .expect("append");
    assert_eq!(result[4], "--");
    assert_eq!(result[6], "--append-system-prompt=old\n\nnotice");
    assert_eq!(result[7], "--");
    assert_eq!(result[8], "after");
}
#[test]
fn prompt_file_options_are_refused_for_confined_harnesses() {
    assert!(
        a2amx::harness::append_system_prompt(
            vec![
                "bwrap".into(),
                "--".into(),
                "claude".into(),
                "--append-system-prompt-file=prompt.txt".into(),
            ],
            "notice",
            Harness::Claude,
            Path::new("."),
        )
        .is_err()
    );
    let file = tempfile::NamedTempFile::new().expect("prompt file");
    assert!(
        a2amx::harness::append_system_prompt(
            vec![
                "bwrap".into(),
                "--".into(),
                "omp".into(),
                "--append-system-prompt".into(),
                file.path().to_string_lossy().into_owned(),
            ],
            "notice",
            Harness::Omp,
            Path::new("."),
        )
        .is_err()
    );
    let root = tempfile::tempdir().expect("prompt directory");
    std::fs::write(root.path().join("prompt.txt"), "prompt").expect("relative prompt");
    assert!(
        a2amx::harness::append_system_prompt(
            vec![
                "bwrap".into(),
                "--".into(),
                "omp".into(),
                "--append-system-prompt=prompt.txt".into(),
            ],
            "notice",
            Harness::Omp,
            root.path(),
        )
        .is_err()
    );
    let claude_after_separator = vec![
        "claude".into(),
        "--".into(),
        "--append-system-prompt-file=prompt.txt".into(),
    ];
    assert!(
        a2amx::harness::validate_prompt_options(
            &claude_after_separator,
            Harness::Claude,
            Path::new(".")
        )
        .is_ok()
    );
    let omp_after_separator = vec![
        "omp".into(),
        "--".into(),
        "--append-system-prompt=prompt.txt".into(),
    ];
    assert!(
        a2amx::harness::validate_prompt_options(&omp_after_separator, Harness::Omp, root.path())
            .is_ok()
    );
}

#[test]
fn preflight_rejects_missing_bwrap() {
    let missing = Path::new("/definitely/missing/a2amx-bwrap");
    assert!(confine::preflight(missing).is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_bwrap_confines_the_session_process() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let state = root.path().join("state");
    let cwd = root.path().join("workspace");
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME not set")?);
    let external = tempfile::tempdir_in(&home)?;
    let outside = external.path().join("outside.txt");
    let chmod_target = external.path().join("chmod-target");
    let hidden = external.path().join("hidden");
    let hidden_write = hidden.join("created");
    let result = cwd.join("result.txt");
    let inside = cwd.join("inside.txt");
    let fd_list = cwd.join("fds.txt");
    let no_new_privs = cwd.join("no-new-privs.txt");
    let done = cwd.join("done");
    let admin = state.join("admin.token");
    std::fs::create_dir_all(&cwd)?;
    std::fs::create_dir_all(&hidden)?;
    std::fs::create_dir_all(&state)?;
    std::fs::write(hidden.join("secret"), "hidden")?;
    std::fs::write(&outside, "unconfined")?;
    std::fs::write(&chmod_target, "chmod")?;
    std::fs::set_permissions(&chmod_target, std::fs::Permissions::from_mode(0o644))?;
    assert_eq!(std::fs::read_to_string(&outside)?, "unconfined");
    std::fs::remove_file(&outside)?;
    assert_eq!(std::fs::read_to_string(hidden.join("secret"))?, "hidden");
    std::fs::write(&admin, "admin")?;
    let bwrap_dir = bwrap_path_dir(Path::new("/usr/bin/bwrap"));
    let daemon = match start_confine_daemon(&state, bwrap_dir.path()).await {
        Ok(daemon) => daemon,
        Err(_error) if !require_bwrap() => return Ok(()),
        Err(error) => return Err(error),
    };
    let mut client = Client::connect(&state).await.context("connect client")?;
    let script = format!(
        "if printf outside > {outside}; then printf outside-ok > {result}; else printf outside-denied > {result}; fi; if printf hidden > {hidden_write}; then printf hidden-write-ok >> {result}; else printf hidden-write-denied >> {result}; fi; if chmod 600 {chmod_target}; then printf chmod-visible >> {result}; else printf chmod-denied >> {result}; fi; printf inside > {inside}; if test -e {hidden}; then printf visible >> {result}; else printf hidden >> {result}; fi; if test -r {admin}; then printf admin-visible >> {result}; else printf admin-hidden >> {result}; fi; if test -e /proc/{host_pid}; then printf proc-visible >> {result}; else printf proc-hidden >> {result}; fi; printf '%s' \"$(for fd in /proc/self/fd/*; do basename \"$fd\"; done)\" > {fd_list}; awk '/NoNewPrivs/{{print $2}}' /proc/self/status > {no_new_privs}; touch {done}; sleep 2",
        outside = outside.display(),
        chmod_target = chmod_target.display(),
        result = result.display(),
        inside = inside.display(),
        hidden = hidden.display(),
        hidden_write = hidden_write.display(),
        admin = admin.display(),
        fd_list = fd_list.display(),
        no_new_privs = no_new_privs.display(),
        done = done.display(),
        host_pid = std::process::id(),
    );
    let response = client
        .request(confined_shell_request(
            &cwd,
            &script,
            vec![],
            ConfineRequest {
                hide: vec![hidden.to_string_lossy().into_owned()],
                ..ConfineRequest::default()
            },
        ))
        .await
        .context("create confined session")?;
    let Response::Created { .. } = response else {
        if require_bwrap() {
            panic!("confined session creation failed: {response:?}");
        }
        daemon.shutdown().await?;
        return Ok(());
    };
    for _ in 0..100 {
        if done.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let result_text = std::fs::read_to_string(&result).context("reading result marker")?;
    assert!(
        result_text.contains("outside-denied"),
        "result={result_text:?}"
    );
    assert!(!outside.exists(), "confined write escaped the workspace");
    assert!(result_text.contains("hidden-write-denied"));
    assert!(!hidden_write.exists(), "hidden tmpfs accepted a write");
    assert!(result_text.contains("chmod-denied"));
    assert_eq!(
        std::fs::metadata(&chmod_target)?.permissions().mode() & 0o777,
        0o644
    );
    assert!(inside.exists());
    assert!(result_text.contains("hidden"));
    assert!(result_text.contains("admin-hidden"));
    assert!(result_text.contains("proc-hidden"));
    let fd_values = std::fs::read_to_string(&fd_list)?
        .split_whitespace()
        .filter_map(|value| value.parse::<u32>().ok())
        .collect::<Vec<_>>();
    assert!(
        fd_values.iter().all(|fd| *fd <= 3),
        "unexpected descriptors: {fd_values:?}"
    );
    assert_eq!(std::fs::read_to_string(&no_new_privs)?.trim(), "1");
    daemon.shutdown().await?;
    Ok(())
}
#[test]
fn confined_session_survives_blocking_pool_thread_retirement() -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_keep_alive(Duration::from_millis(100))
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let root = tempfile::tempdir()?;
        let state = root.path().join("state");
        let cwd = root.path().join("workspace");
        std::fs::create_dir_all(&cwd)?;
        let bwrap_dir = bwrap_path_dir(Path::new("/usr/bin/bwrap"));
        let daemon = match start_confine_daemon(&state, bwrap_dir.path()).await {
            Ok(daemon) => daemon,
            Err(_error) if !require_bwrap() => return Ok(()),
            Err(error) => return Err(error),
        };
        let mut client = Client::connect(&state).await.context("connect client")?;
        let response = client
            .request(confined_shell_request(
                &cwd,
                "sleep 300",
                vec![],
                ConfineRequest::default(),
            ))
            .await
            .context("create confined session")?;
        let session_id = match response {
            Response::Created { session } => session,
            response => {
                if !require_bwrap() {
                    daemon.shutdown().await?;
                    return Ok(());
                }
                anyhow::bail!("confined session creation failed: {response:?}");
            }
        };
        for _ in 0..4 {
            tokio::task::spawn_blocking(|| {}).await?;
        }
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let sessions = match client.request(Request::List).await? {
            Response::Sessions { sessions } => sessions,
            response => anyhow::bail!("unexpected list response: {response:?}"),
        };
        let session = sessions
            .iter()
            .find(|session| session.id == session_id)
            .context("confined session missing from list")?;
        assert!(
            session.exit_code.is_none(),
            "confined session exited after blocking-pool retirement: {session:?}"
        );
        daemon.shutdown().await?;
        Ok(())
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn late_created_workspace_socket_is_visible_inside_confined_session() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let state = root.path().join("state");
    let cwd = root.path().join("workspace");
    std::fs::create_dir_all(&cwd)?;
    let socket_path = cwd.join("late.sock");
    let late_file = cwd.join("late-file");
    let result = cwd.join("socket-result");
    let bwrap_dir = bwrap_path_dir(Path::new("/usr/bin/bwrap"));
    let daemon = match start_confine_daemon(&state, bwrap_dir.path()).await {
        Ok(daemon) => daemon,
        Err(_error) if !require_bwrap() => return Ok(()),
        Err(error) => return Err(error),
    };
    let mut client = Client::connect(&state).await.context("connect client")?;
    let script = "until python3 -c 'import os,socket; s=socket.socket(socket.AF_UNIX); s.connect(os.environ[\"SOCKET\"]); s.send(b\"x\"); open(os.environ[\"RESULT\"], \"wb\").write(open(os.environ[\"FILE\"], \"rb\").read()+s.recv(1))'; do sleep 0.02; done";
    let response = client
        .request(confined_shell_request(
            &cwd,
            script,
            vec![
                ("SOCKET".into(), socket_path.to_string_lossy().into_owned()),
                ("RESULT".into(), result.to_string_lossy().into_owned()),
                ("FILE".into(), late_file.to_string_lossy().into_owned()),
            ],
            ConfineRequest::default(),
        ))
        .await?;
    assert!(matches!(response, Response::Created { .. }));
    std::fs::write(&late_file, "file")?;
    let listener = tokio::net::UnixListener::bind(&socket_path)?;
    let (mut stream, _) = timeout(Duration::from_secs(5), listener.accept()).await??;
    let mut received = [0; 1];
    stream.read_exact(&mut received).await?;
    assert_eq!(received, [b'x']);
    stream.write_all(b"y").await?;
    for _ in 0..100 {
        if result.metadata().is_ok_and(|metadata| metadata.len() > 0) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(std::fs::read(&result)?, b"filey");
    daemon.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn confined_request_without_cwd_is_refused() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let state = root.path().join("state");
    std::fs::create_dir_all(&state)?;
    let bwrap_dir = bwrap_path_dir(Path::new("/usr/bin/bwrap"));
    let daemon = match start_confine_daemon(&state, bwrap_dir.path()).await {
        Ok(daemon) => daemon,
        Err(_error) if !require_bwrap() => return Ok(()),
        Err(error) => return Err(error),
    };
    let mut client = Client::connect(&state).await.context("connect client")?;
    let request = Request::Confine {
        confine: ConfineRequest::default(),
        request: Box::new(Request::NewSession {
            argv: vec!["true".into()],
            cols: 80,
            rows: 24,
            cwd: None,
            env: vec![],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            activity: None,
            heartbeat: None,
            name: None,
            harness: Harness::Generic,
            deliver: None,
            team: None,
            role: None,
        }),
    };
    let response = client.request(request).await?;
    assert!(matches!(
        response,
        Response::Error { message } if message.contains("explicit cwd")
    ));
    daemon.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_bwrap_probe_refuses_confined_session() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let state = root.path().join("state");
    let cwd = root.path().join("workspace");
    std::fs::create_dir_all(&cwd)?;
    let stub_dir = stub_path_dir(42);
    let daemon = start_confine_daemon(&state, stub_dir.path()).await?;
    let mut client = Client::connect(&state).await?;
    let marker = cwd.join("marker");
    let response = client
        .request(confined_shell_request(
            &cwd,
            &format!("printf ran > {}", marker.display()),
            vec![],
            ConfineRequest::default(),
        ))
        .await?;
    let Response::Error { message } = response else {
        panic!("failed probe unexpectedly created a session: {response:?}");
    };
    assert!(message.contains("bubblewrap") || message.contains("exit"));
    assert!(!marker.exists());
    daemon.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bwrap_probe_that_fails_after_version_refuses_confined_session() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let state = root.path().join("state");
    let cwd = root.path().join("workspace");
    std::fs::create_dir_all(&cwd)?;
    let stub_dir = probe_fail_stub_path_dir();
    let daemon = start_confine_daemon(&state, stub_dir.path()).await?;
    let mut client = Client::connect(&state).await?;
    let response = client
        .request(confined_shell_request(
            &cwd,
            "printf ran > marker",
            vec![],
            ConfineRequest::default(),
        ))
        .await?;
    assert!(matches!(response, Response::Error { .. }));
    assert!(!cwd.join("marker").exists());
    daemon.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_bwrap_path_refuses_confined_session() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let state = root.path().join("state");
    let cwd = root.path().join("workspace");
    std::fs::create_dir_all(&cwd)?;
    let empty_dir = tempfile::tempdir()?;
    let daemon = start_confine_daemon(&state, empty_dir.path()).await?;
    let mut client = Client::connect(&state).await?;
    let response = client
        .request(confined_shell_request(
            &cwd,
            "printf ran > marker",
            vec![],
            ConfineRequest::default(),
        ))
        .await?;
    assert!(matches!(response, Response::Error { .. }));
    assert!(!cwd.join("marker").exists());
    daemon.shutdown().await?;
    Ok(())
}
