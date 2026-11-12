#![allow(clippy::expect_used, clippy::unwrap_used)]
mod common;

use anyhow::Context as _;
use std::ffi::{OsStr, OsString};
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc as std_mpsc;

use a2amx::client::Client;
use a2amx::confine::{self, ConfineRequest, Host};
use a2amx::daemon::{Daemon, DaemonConfig};
use a2amx::grant;
use a2amx::harness::Harness;
use a2amx::messaging::Limits;
use a2amx::team;
use a2amx::wire::{ClientFrame, Request, Response};
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

fn bwrap_probe_fail_stub_path_dir() -> TempDir {
    let dir = tempfile::tempdir().expect("bwrap probe stub directory");
    let path = dir.path().join("bwrap");
    std::fs::write(
        &path,
        "#!/bin/sh\ncase \"$1\" in\n--version|--ignore-signal*) exit 0 ;;\n*) exit 42 ;;\nesac\n",
    )
    .expect("bwrap probe stub");
    let mut permissions = std::fs::metadata(&path)
        .expect("bwrap probe stub metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).expect("bwrap probe stub permissions");
    dir
}

fn require_bwrap() -> bool {
    std::env::var_os("A2AMX_REQUIRE_BWRAP").is_some_and(|value| value == "1")
}

async fn start_confine_daemon(state: &Path, bwrap_dir: &Path) -> anyhow::Result<Daemon> {
    start_confine_daemon_with_env(state, bwrap_dir, None).await
}

async fn start_confine_daemon_with_env(
    state: &Path,
    bwrap_dir: &Path,
    env_path: Option<PathBuf>,
) -> anyhow::Result<Daemon> {
    Daemon::start(DaemonConfig {
        state_dir: state.to_owned(),
        listen: vec!["127.0.0.1:0".parse()?],
        host_name: Some("host-a".into()),
        limits: Limits::default(),
        bwrap_path: Some(OsString::from(bwrap_dir)),
        env_path,
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
            remote: None,
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

#[test]
fn runtime_grants_select_sockets_and_environment() {
    let fixture = Fixture::new();
    let docker_socket = fixture.home.join(".docker/run/docker.sock");
    std::fs::create_dir_all(docker_socket.parent().expect("docker parent")).expect("docker dir");
    std::fs::write(&docker_socket, "socket placeholder").expect("docker socket fixture");
    let restricted = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("restricted plan");
    let restricted_wrapped = restricted
        .wrap(vec!["true".into()])
        .expect("restricted wrapper");
    assert!(
        restricted_wrapped
            .windows(2)
            .any(|window| window == ["--unsetenv", "DOCKER_HOST"])
    );
    assert!(
        restricted_wrapped
            .windows(2)
            .any(|window| window == ["--unsetenv", "CONTAINER_HOST"])
    );
    let docker = confine::plan(
        &ConfineRequest {
            grants: vec!["docker".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("docker plan");
    let docker_socket = std::fs::canonicalize(docker_socket).expect("docker socket canonical");
    assert!(!docker.hidden_paths().contains(&docker_socket));
    let podman = PathBuf::from("/run/podman/podman.sock");
    let docker_host_socket = PathBuf::from("/run/docker.sock");
    let podman_canonical = std::fs::canonicalize(&podman).ok();
    let docker_canonical = std::fs::canonicalize(&docker_host_socket).ok();
    if let (Some(podman_target), Some(docker_target)) =
        (podman_canonical.as_ref(), docker_canonical.as_ref())
    {
        if podman_target == docker_target {
            assert!(!docker.hidden_paths().contains(podman_target));
            assert!(!docker.skipped_paths().contains(&podman));
        } else {
            assert!(
                docker.hidden_paths().contains(podman_target)
                    || docker.skipped_paths().contains(&podman),
                "podman socket must stay hidden or skipped"
            );
        }
    } else {
        let hidden_podman = podman_canonical
            .as_ref()
            .is_some_and(|target| docker.hidden_paths().contains(target));
        assert!(
            hidden_podman || docker.skipped_paths().contains(&podman),
            "podman socket must stay hidden or skipped"
        );
    }
    assert!(!docker.hidden_paths().contains(&docker_host_socket));
    assert!(!docker.skipped_paths().contains(&docker_host_socket));
    let docker_wrapped = docker.wrap(vec!["true".into()]).expect("docker wrapper");
    assert!(
        !docker_wrapped
            .windows(2)
            .any(|window| window == ["--unsetenv", "DOCKER_HOST"])
    );
    assert!(
        !docker_wrapped
            .windows(2)
            .any(|window| window == ["--unsetenv", "DOCKER_CONTEXT"])
    );
    assert!(
        docker_wrapped
            .windows(2)
            .any(|window| window == ["--unsetenv", "CONTAINER_HOST"])
    );
}

#[test]
fn credential_grants_lift_named_paths_writable() {
    let fixture = Fixture::new();
    let gh = fixture.home.join(".config/gh");
    std::fs::create_dir_all(&gh).expect("gh directory");
    std::fs::write(gh.join("hosts.yml"), "github.com:\n").expect("gh hosts");
    let gh = std::fs::canonicalize(gh).expect("gh canonical");

    let restricted = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("restricted plan");
    assert!(restricted.hidden_paths().contains(&gh));

    let granted = confine::plan(
        &ConfineRequest {
            grants: vec!["gh".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("gh plan");
    assert!(granted.writable.contains(&gh));
    assert!(!granted.readonly.contains(&gh));
    assert!(!granted.protected.contains(&gh));
    assert!(!granted.hidden_paths().contains(&gh));
    let gh_string = gh.to_string_lossy().into_owned();
    let wrapped = granted
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("gh wrapper");
    assert!(
        wrapped
            .windows(3)
            .any(|window| window == ["--bind", gh_string.as_str(), gh_string.as_str()])
    );
    assert!(
        !wrapped
            .windows(3)
            .any(|window| window == ["--ro-bind", gh_string.as_str(), gh_string.as_str()])
    );
    assert!(
        !wrapped
            .windows(2)
            .any(|window| window == ["--tmpfs", gh_string.as_str()])
    );
}

#[test]
fn credential_grant_stays_writable_inside_writable_parent() {
    let fixture = Fixture::new();
    let config = fixture.home.join(".config");
    let gh = config.join("gh");
    std::fs::create_dir_all(&gh).expect("gh directory");
    std::fs::write(gh.join("hosts.yml"), "github.com:\n").expect("gh hosts");
    let config = std::fs::canonicalize(config).expect("config canonical");
    let gh = std::fs::canonicalize(gh).expect("gh canonical");

    let granted = confine::plan(
        &ConfineRequest {
            write: vec!["~/.config".into()],
            grants: vec!["gh".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("gh writable-parent plan");
    assert!(granted.writable.contains(&config));
    assert!(granted.writable.contains(&gh));
    assert!(!granted.protected.contains(&gh));
    assert!(!granted.readonly.contains(&gh));
    let config_string = config.to_string_lossy().into_owned();
    let gh_string = gh.to_string_lossy().into_owned();
    let wrapped = granted
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("gh writable-parent wrapper");
    assert!(
        wrapped
            .windows(3)
            .any(|window| window == ["--bind", config_string.as_str(), config_string.as_str()])
    );
    assert!(
        wrapped
            .windows(3)
            .any(|window| window == ["--bind", gh_string.as_str(), gh_string.as_str()])
    );
    assert!(
        !wrapped
            .windows(3)
            .any(|window| window == ["--ro-bind", gh_string.as_str(), gh_string.as_str()])
    );
}

#[test]
fn exact_write_lifts_a_granted_credential_path() {
    let fixture = Fixture::new();
    let gh = fixture.home.join(".config/gh");
    std::fs::create_dir_all(&gh).expect("gh directory");
    std::fs::write(gh.join("hosts.yml"), "github.com:\n").expect("gh hosts");
    let gh = std::fs::canonicalize(gh).expect("gh canonical");
    let plan = confine::plan(
        &ConfineRequest {
            write: vec!["~/.config/gh".into()],
            grants: vec!["gh".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("exact-write plan");
    assert!(plan.writable.contains(&gh));
    assert!(!plan.readonly.contains(&gh));
}

#[test]
fn cargo_grant_keeps_credentials_hidden_and_path_bin_protected() {
    let fixture = Fixture::new();
    let cargo = fixture.home.join(".cargo");
    let credentials = cargo.join("credentials.toml");
    let cargo_bin = cargo.join("bin");
    std::fs::create_dir_all(&cargo_bin).expect("cargo bin");
    std::fs::write(&credentials, "credentials").expect("cargo credentials");
    std::fs::write(cargo_bin.join("cargo-install"), "binary").expect("cargo binary");
    let path_var = OsString::from(format!("{}:/usr/bin", cargo_bin.display()));
    let plan = confine::plan_with_path(
        &ConfineRequest {
            grants: vec!["cargo".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
        Some(path_var),
    )
    .expect("cargo grant plan");
    let cargo = std::fs::canonicalize(cargo).expect("cargo canonical");
    let credentials = std::fs::canonicalize(credentials).expect("credentials canonical");
    let cargo_bin = std::fs::canonicalize(cargo_bin).expect("cargo bin canonical");
    assert!(plan.writable.contains(&cargo));
    assert!(plan.hidden_paths().contains(&credentials));
    assert!(plan.protected.contains(&cargo_bin));
    assert!(!plan.writable.contains(&credentials));
}

#[test]
fn cargo_token_grant_lifts_only_its_credential_files() {
    let fixture = Fixture::new();
    let cargo = fixture.home.join(".cargo");
    let credentials = cargo.join("credentials");
    let credentials_toml = cargo.join("credentials.toml");
    std::fs::create_dir_all(&cargo).expect("cargo directory");
    std::fs::write(&credentials, "credentials").expect("cargo credentials");
    std::fs::write(&credentials_toml, "credentials").expect("cargo credentials toml");
    let cargo = std::fs::canonicalize(cargo).expect("cargo canonical");
    let credentials = std::fs::canonicalize(credentials).expect("credentials canonical");
    let credentials_toml =
        std::fs::canonicalize(credentials_toml).expect("credentials toml canonical");

    let cargo_only = confine::plan(
        &ConfineRequest {
            grants: vec!["cargo".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("cargo-only grant plan");
    assert!(cargo_only.writable.contains(&cargo));
    assert!(cargo_only.hidden_paths().contains(&credentials));
    assert!(cargo_only.hidden_paths().contains(&credentials_toml));

    let cargo_and_token = confine::plan(
        &ConfineRequest {
            grants: vec!["cargo".into(), "cargo-token".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("cargo and token grant plan");
    for path in [&cargo, &credentials, &credentials_toml] {
        assert!(
            cargo_and_token.writable.contains(path),
            "not writable: {path:?}"
        );
        assert!(
            !cargo_and_token.hidden_paths().contains(path),
            "still hidden: {path:?}"
        );
    }

    let token_only = confine::plan(
        &ConfineRequest {
            grants: vec!["cargo-token".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("token-only grant plan");
    assert!(!token_only.writable.contains(&cargo));
    assert!(token_only.writable.contains(&credentials));
    assert!(token_only.writable.contains(&credentials_toml));
}
#[test]
fn cache_grant_keeps_the_default_hides() {
    let fixture = Fixture::new();
    let cache = fixture.home.join(".cache");
    let token = cache.join("huggingface/token");
    std::fs::create_dir_all(token.parent().expect("token parent")).expect("cache directories");
    std::fs::write(&token, "token").expect("huggingface token");
    let cache = std::fs::canonicalize(cache).expect("cache canonical");
    let token = std::fs::canonicalize(token).expect("token canonical");

    let plan = confine::plan(
        &ConfineRequest {
            grants: vec!["cache".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("cache grant plan");
    assert!(plan.writable.contains(&cache));
    assert!(plan.hidden_paths().contains(&token));
}

#[test]
fn huggingface_token_grant_opens_only_the_token() {
    let fixture = Fixture::new();
    let cache = fixture.home.join(".cache");
    let token = cache.join("huggingface/token");
    std::fs::create_dir_all(token.parent().expect("token parent")).expect("cache directories");
    std::fs::write(&token, "token").expect("huggingface token");
    let cache = std::fs::canonicalize(cache).expect("cache canonical");
    let token = std::fs::canonicalize(token).expect("token canonical");

    let cache_and_token = confine::plan(
        &ConfineRequest {
            grants: vec!["cache".into(), "huggingface-token".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("cache and token grant plan");
    assert!(cache_and_token.writable.contains(&cache));
    assert!(!cache_and_token.hidden_paths().contains(&token));
    let wrapped = cache_and_token
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("cache and token wrapper");
    for name in ["HF_TOKEN", "HUGGING_FACE_HUB_TOKEN"] {
        assert!(
            !wrapped
                .windows(2)
                .any(|window| window == ["--unsetenv", name]),
            "{name} remains unset"
        );
    }

    let token_only = confine::plan(
        &ConfineRequest {
            grants: vec!["huggingface-token".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("token-only grant plan");
    assert!(!token_only.writable.contains(&cache));
    assert!(token_only.writable.contains(&token));
    assert!(!token_only.hidden_paths().contains(&token));
}

#[test]
fn cache_grant_with_narrower_grants() {
    let fixture = Fixture::new();
    let cache = fixture.home.join(".cache");
    for path in [
        cache.join("go-build"),
        cache.join("pip"),
        cache.join("uv"),
        cache.join("pypoetry"),
        cache.join("virtualenv"),
        fixture.home.join(".local/share/uv"),
        fixture.home.join(".local/share/pipx"),
    ] {
        std::fs::create_dir_all(path).expect("cache grant directory");
    }
    let cache = std::fs::canonicalize(cache).expect("cache canonical");
    let plan = confine::plan(
        &ConfineRequest {
            grants: vec!["cache".into(), "go".into(), "python".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("overlapping cache grants plan");
    assert!(plan.writable.contains(&cache));
}

#[test]
fn cache_grant_refuses_a_state_directory_under_it() {
    let fixture = Fixture::new();
    let cache = fixture.home.join(".cache");
    std::fs::create_dir_all(&cache).expect("cache directory");
    let state = cache.join("state");
    let empty = confine::ensure_empty_file(&state).expect("cache state empty file");
    let mut host = fixture.host();
    host.state_dir = state;
    host.empty_file = empty;
    let request = ConfineRequest {
        grants: vec!["cache".into()],
        ..ConfineRequest::default()
    };
    let error = match confine::plan(&request, Harness::Generic, &fixture.cwd, &host) {
        Ok(_) => panic!("cache grant must reject a nested state directory"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("overlaps the state directory"));

    let raw_write = confine::plan(
        &ConfineRequest {
            write: vec!["~/.cache".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &host,
    );
    assert!(raw_write.is_ok());
}

#[test]
fn cache_grant_preflight_refuses_a_state_directory_under_it() {
    let fixture = Fixture::new();
    let cache = fixture.home.join(".cache");
    std::fs::create_dir_all(&cache).expect("cache directory");
    let state = cache.join("state");
    let empty = confine::ensure_empty_file(&state).expect("cache state empty file");
    let mut host = fixture.host();
    host.state_dir = state;
    host.empty_file = empty;
    let request = ConfineRequest {
        grants: vec!["cache".into()],
        ..ConfineRequest::default()
    };
    let preflight = confine::ensure_grant_dirs(&request, Harness::Generic, &host, true);
    let error = match confine::plan_with_preflight(
        &request,
        Harness::Generic,
        &fixture.cwd,
        &host,
        None,
        preflight,
    ) {
        Ok(_) => panic!("cache grant must reject a nested state directory"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("overlaps the state directory"));
}

#[test]
fn missing_node_grant_path_is_skipped_after_preflight_without_creation() {
    let fixture = Fixture::new();
    let npm = fixture.home.join(".npm");
    let request = ConfineRequest {
        grants: vec!["node".into()],
        ..ConfineRequest::default()
    };
    let preflight = confine::ensure_grant_dirs(&request, Harness::Generic, &fixture.host(), false);
    assert!(preflight.is_empty());
    let plan = confine::plan_with_preflight(
        &request,
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
        None,
        preflight,
    )
    .expect("missing node grant path plan");
    assert!(!npm.exists());
    assert!(plan.skipped_paths().contains(&npm));
}
#[test]
fn gh_grant_opens_only_its_paths_and_conflicts_with_exact_hides() {
    let fixture = Fixture::new();
    let config = fixture.home.join(".config");
    let gh = config.join("gh");
    std::fs::create_dir_all(&gh).expect("gh directory");
    std::fs::write(gh.join("hosts.yml"), "github.com:\n").expect("gh hosts");
    std::fs::write(
        fixture.home.join(".git-credentials"),
        "https://user:pass@example\n",
    )
    .expect("git credentials");
    let git_config = config.join("git");
    std::fs::create_dir_all(&git_config).expect("git config directory");
    std::fs::write(
        git_config.join("credentials"),
        "https://user:pass@example\n",
    )
    .expect("git config credentials");
    let gh = std::fs::canonicalize(gh).expect("gh canonical");
    let config = std::fs::canonicalize(config).expect("config canonical");
    let plan = confine::plan(
        &ConfineRequest {
            grants: vec!["gh".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("gh-only plan");
    assert!(!plan.hidden_paths().contains(&gh));
    assert!(plan.hidden_paths().contains(&fixture.home.join(".ssh")));
    assert!(!plan.writable.contains(&config));
    assert!(!plan.readonly.contains(&config));
    assert!(
        plan.hidden_paths()
            .contains(&fixture.home.join(".git-credentials"))
    );
    assert!(
        plan.hidden_paths()
            .iter()
            .any(|path| path.ends_with(".config/git/credentials"))
    );

    for request in [
        ConfineRequest {
            grants: vec!["gh".into()],
            hide: vec!["~/.config/gh".into()],
            ..ConfineRequest::default()
        },
        ConfineRequest {
            grants: vec!["gh".into()],
            hide: vec!["~/.config/gh".into()],
            write: vec!["~/.config/gh".into()],
            ..ConfineRequest::default()
        },
    ] {
        let error = match confine::plan(&request, Harness::Generic, &fixture.cwd, &fixture.host()) {
            Ok(_) => panic!("exact hide must conflict with gh grant"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("confinement grant \"gh\" conflicts with the hide of"),
            "unexpected conflict: {error}"
        );
    }
}

#[test]
fn git_credentials_grant_lifts_both_files_only() {
    let fixture = Fixture::new();
    let gh = fixture.home.join(".config/gh");
    std::fs::create_dir_all(&gh).expect("gh directory");
    std::fs::write(gh.join("hosts.yml"), "github.com:\n").expect("gh hosts");
    std::fs::write(
        fixture.home.join(".git-credentials"),
        "https://user:pass@example\n",
    )
    .expect("git credentials");
    let git_config = fixture.home.join(".config/git");
    std::fs::create_dir_all(&git_config).expect("git config directory");
    std::fs::write(
        git_config.join("credentials"),
        "https://user:pass@example\n",
    )
    .expect("git config credentials");
    let git_credentials = std::fs::canonicalize(fixture.home.join(".git-credentials"))
        .expect("git credentials canonical");
    let git_config_credentials =
        std::fs::canonicalize(git_config.join("credentials")).expect("git config credentials");
    let gh = std::fs::canonicalize(gh).expect("gh canonical");
    let plan = confine::plan(
        &ConfineRequest {
            grants: vec!["git-credentials".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("git credentials plan");
    for path in [&git_credentials, &git_config_credentials] {
        assert!(plan.writable.contains(path));
        assert!(!plan.readonly.contains(path));
        assert!(!plan.protected.contains(path));
    }
    assert!(!plan.hidden_paths().contains(&git_credentials));
    assert!(!plan.hidden_paths().contains(&git_config_credentials));
    assert!(plan.hidden_paths().contains(&gh));
    let wrapped = plan
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("git credentials wrapper");
    for path in [&git_credentials, &git_config_credentials] {
        let path = path.to_string_lossy();
        assert!(
            wrapped
                .windows(3)
                .any(|window| { window[0] == "--bind" && window[1] == path && window[2] == path })
        );
        assert!(
            !wrapped.windows(3).any(|window| {
                window[0] == "--ro-bind" && window[1] == path && window[2] == path
            })
        );
    }
}

#[test]
fn granted_paths_may_be_missing_without_becoming_unresolved() {
    let fixture = Fixture::new();
    let missing = fixture.home.join(".config/gh");
    let plan = confine::plan(
        &ConfineRequest {
            grants: vec!["gh".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("missing credential path plan");
    assert!(plan.unresolved.is_empty());
    assert!(plan.skipped_paths().contains(&missing));
}

#[test]
fn grant_alias_to_hidden_or_unhidden_path_is_rejected() {
    let fixture = Fixture::new();
    let config = fixture.home.join(".config");
    std::fs::create_dir_all(&config).expect("config directory");
    let hidden_alias = config.join("gh");
    symlink(fixture.home.join(".ssh"), &hidden_alias).expect("gh hidden alias");
    let error = match confine::plan(
        &ConfineRequest {
            grants: vec!["gh".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    ) {
        Ok(_) => panic!("gh alias to ssh must fail"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("confinement grant \"gh\" path ~/.config/gh passes through the symlink")
    );
    assert!(
        error
            .to_string()
            .contains(&hidden_alias.display().to_string())
    );

    let plain = Fixture::new();
    let plain_config = plain.home.join(".config");
    let target = plain.home.join("unhidden-target");
    std::fs::create_dir_all(&plain_config).expect("plain config directory");
    std::fs::create_dir_all(&target).expect("plain target");
    let plain_alias = plain_config.join("gh");
    symlink(&target, &plain_alias).expect("gh plain alias");
    let error = match confine::plan(
        &ConfineRequest {
            grants: vec!["gh".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &plain.cwd,
        &plain.host(),
    ) {
        Ok(_) => panic!("gh alias to a plain path must fail"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("confinement grant \"gh\" path ~/.config/gh passes through the symlink")
    );
    assert!(
        error
            .to_string()
            .contains(&plain_alias.display().to_string())
    );
}

#[test]
fn grant_symlink_paths_are_rejected_for_toolchain_and_token_grants() {
    for (grant_name, entry, target_is_file) in [
        ("python", "~/.cache/pip", false),
        ("cargo", "~/.cargo", false),
        ("npm-token", "~/.npmrc", true),
    ] {
        let fixture = Fixture::new();
        let source = fixture
            .home
            .join(entry.strip_prefix("~/").expect("home entry"));
        let target = fixture._root.path().join("plain-target");
        if target_is_file {
            std::fs::write(&target, "target").expect("plain target file");
        } else {
            std::fs::create_dir_all(&target).expect("plain target directory");
        }
        std::fs::create_dir_all(source.parent().expect("symlink parent"))
            .expect("symlink parent directory");
        symlink(&target, &source).expect("grant symlink");
        let error = match confine::plan(
            &ConfineRequest {
                grants: vec![grant_name.into()],
                ..ConfineRequest::default()
            },
            Harness::Generic,
            &fixture.cwd,
            &fixture.host(),
        ) {
            Ok(_) => panic!("{grant_name} symlink must fail"),
            Err(error) => error,
        };
        let text = error.to_string();
        assert!(
            text.contains(&format!(
                "confinement grant \"{grant_name}\" path {entry} passes through the symlink"
            )),
            "unexpected error: {text}"
        );
        assert!(
            text.contains(&source.display().to_string()),
            "unexpected error: {text}"
        );
    }
}

#[test]
fn grant_symlinked_home_ancestor_is_accepted_after_home_canonicalization() {
    let fixture = Fixture::new();
    let config = fixture.home.join(".config/gh");
    std::fs::create_dir_all(&config).expect("gh directory");
    let home_link = fixture._root.path().join("home-link");
    symlink(&fixture.home, &home_link).expect("home link");
    let mut host = fixture.host();
    host.home = home_link;
    let plan = confine::plan(
        &ConfineRequest {
            grants: vec!["gh".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &host,
    )
    .expect("canonical home plan");
    assert!(
        plan.writable
            .contains(&std::fs::canonicalize(config).expect("gh canonical"))
    );
}

#[test]
fn grant_alias_to_home_is_rejected_as_a_symlink() {
    let fixture = Fixture::new();
    let config = fixture.home.join(".config");
    std::fs::create_dir_all(&config).expect("config directory");
    let alias = config.join("gh");
    symlink(&fixture.home, &alias).expect("home alias");
    let error = match confine::plan(
        &ConfineRequest {
            grants: vec!["gh".into()],
            no_default_hides: true,
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    ) {
        Ok(_) => panic!("grant alias to home must be rejected"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("passes through the symlink"));
    assert!(error.to_string().contains(&alias.display().to_string()));
}

#[test]
fn grant_alias_into_state_is_rejected_as_a_symlink() {
    let fixture = Fixture::new();
    let target = fixture.state.join("inside");
    std::fs::create_dir_all(&target).expect("state target");
    let config = fixture.home.join(".config");
    std::fs::create_dir_all(&config).expect("config directory");
    let alias = config.join("gh");
    symlink(&target, &alias).expect("state alias");
    let error = match confine::plan(
        &ConfineRequest {
            grants: vec!["gh".into()],
            no_default_hides: true,
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    ) {
        Ok(_) => panic!("grant alias into state must be rejected"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("passes through the symlink"));
    assert!(error.to_string().contains(&alias.display().to_string()));
}

#[test]
fn grant_alias_into_install_or_harness_is_rejected_as_a_symlink() {
    let install = Fixture::new();
    let install_target = install.home.join(".a2amx/bin");
    std::fs::create_dir_all(&install_target).expect("install target");
    let install_config = install.home.join(".config");
    std::fs::create_dir_all(&install_config).expect("config directory");
    let install_alias = install_config.join("gh");
    symlink(&install_target, &install_alias).expect("install alias");
    let error = match confine::plan(
        &ConfineRequest {
            grants: vec!["gh".into()],
            no_default_hides: true,
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &install.cwd,
        &install.host(),
    ) {
        Ok(_) => panic!("grant alias into .a2amx must be rejected"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("passes through the symlink"));
    assert!(
        error
            .to_string()
            .contains(&install_alias.display().to_string())
    );

    let harness = Fixture::new();
    let harness_root = harness.home.join(".claude");
    std::fs::create_dir_all(&harness_root).expect("Claude root");
    let harness_config = harness.home.join(".config");
    std::fs::create_dir_all(&harness_config).expect("config directory");
    let harness_alias = harness_config.join("gh");
    symlink(&harness_root, &harness_alias).expect("harness alias");
    let error = match confine::plan(
        &ConfineRequest {
            grants: vec!["gh".into()],
            no_default_hides: true,
            ..ConfineRequest::default()
        },
        Harness::Claude,
        &harness.cwd,
        &harness.host(),
    ) {
        Ok(_) => panic!("grant alias into harness root must be rejected"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("passes through the symlink"));
    assert!(
        error
            .to_string()
            .contains(&harness_alias.display().to_string())
    );
}

#[test]
fn grant_alias_to_path_directory_is_rejected_as_a_symlink() {
    let fixture = Fixture::new();
    let path_dir = fixture.home.join(".local/bin");
    std::fs::create_dir_all(&path_dir).expect("PATH directory");
    let config = fixture.home.join(".config");
    std::fs::create_dir_all(&config).expect("config directory");
    let alias = config.join("gh");
    symlink(&path_dir, &alias).expect("PATH alias");
    let path_var = OsString::from(format!("{}:/usr/bin", path_dir.display()));
    let error = match confine::plan_with_path(
        &ConfineRequest {
            grants: vec!["gh".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
        Some(path_var),
    ) {
        Ok(_) => panic!("grant alias into PATH must be rejected"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("passes through the symlink"));
    assert!(error.to_string().contains(&alias.display().to_string()));
}

#[test]
fn new_grants_open_only_existing_paths_and_release_only_their_environment() {
    const CREDENTIAL_ENV: [&str; 29] = [
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
    ];
    let cases: [(&str, &[&str]); 19] = [
        ("cargo", &["~/.cargo", "~/.rustup"]),
        ("go", &["~/go", "~/.cache/go-build"]),
        (
            "python",
            &[
                "~/.cache/pip",
                "~/.cache/uv",
                "~/.local/share/uv",
                "~/.cache/pypoetry",
                "~/.cache/virtualenv",
                "~/.local/share/pipx",
            ],
        ),
        (
            "node",
            &[
                "~/.npm",
                "~/.yarn/berry",
                "~/.local/share/pnpm",
                "~/.bun/install/cache",
            ],
        ),
        ("java", &["~/.m2/repository", "~/.gradle"]),
        (
            "c-cpp",
            &["~/.cache/ccache", "~/.ccache", "~/.conan2", "~/.cmake"],
        ),
        ("semgrep", &["~/.semgrep"]),
        ("sonar", &["~/.sonar"]),
        ("aws", &["~/.aws"]),
        ("gcloud", &["~/.config/gcloud"]),
        ("azure", &["~/.azure"]),
        ("kube", &["~/.kube"]),
        ("terraform", &["~/.terraform.d"]),
        ("npm-token", &["~/.npmrc"]),
        ("pypi-token", &["~/.pypirc"]),
        (
            "cargo-token",
            &["~/.cargo/credentials", "~/.cargo/credentials.toml"],
        ),
        ("netrc", &["~/.netrc"]),
        ("cache", &["~/.cache"]),
        ("huggingface-token", &["~/.cache/huggingface/token"]),
    ];

    for (grant_name, entries) in cases {
        let fixture = Fixture::new();
        let mut expected = Vec::new();
        for &entry in entries {
            let path = fixture
                .home
                .join(entry.strip_prefix("~/").expect("home-relative grant path"));
            let is_file = matches!(
                entry,
                "~/.npmrc"
                    | "~/.pypirc"
                    | "~/.netrc"
                    | "~/.cargo/credentials"
                    | "~/.cargo/credentials.toml"
                    | "~/.cache/huggingface/token"
            );
            if is_file {
                std::fs::create_dir_all(path.parent().expect("grant file parent"))
                    .expect("grant file parent");
                std::fs::write(&path, "credential").expect("grant file");
            } else {
                std::fs::create_dir_all(&path).expect("grant directory");
            }
            expected.push(std::fs::canonicalize(path).expect("grant path canonical"));
        }
        let baseline = confine::plan(
            &ConfineRequest {
                ..ConfineRequest::default()
            },
            Harness::Generic,
            &fixture.cwd,
            &fixture.host(),
        )
        .expect("baseline grant matrix plan");
        let granted = confine::plan(
            &ConfineRequest {
                grants: vec![grant_name.into()],
                ..ConfineRequest::default()
            },
            Harness::Generic,
            &fixture.cwd,
            &fixture.host(),
        )
        .expect("grant matrix plan");
        let mut added = granted
            .writable
            .iter()
            .filter(|path| !baseline.writable.contains(path))
            .cloned()
            .collect::<Vec<_>>();
        added.sort();
        expected.sort();
        assert_eq!(
            added, expected,
            "unexpected writable paths for {grant_name}"
        );

        let ssh = std::fs::canonicalize(fixture.home.join(".ssh")).expect("ssh canonical");
        assert!(
            granted.hidden_paths().contains(&ssh),
            "unrelated ssh hide lifted by {grant_name}"
        );
        let aws = std::fs::canonicalize(fixture.home.join(".aws")).expect("aws canonical");
        if grant_name == "aws" {
            assert!(!granted.hidden_paths().contains(&aws));
        } else {
            assert!(
                granted.hidden_paths().contains(&aws),
                "unrelated aws hide lifted by {grant_name}"
            );
        }

        let wrapped = granted
            .with_bwrap(PathBuf::from("bwrap"))
            .wrap(vec!["true".into()])
            .expect("grant matrix wrapper");
        let row = grant::lookup(grant_name).expect("grant row");
        for name in CREDENTIAL_ENV {
            let unset = wrapped
                .windows(2)
                .any(|window| window == ["--unsetenv", name]);
            assert_eq!(
                unset,
                !row.env.contains(&name),
                "unexpected environment release for {grant_name}: {name}"
            );
        }
    }
}

#[test]
fn team_file_cargo_token_grant_releases_environment() {
    let sessions = team::parse(
        r#"
[confine]
enabled = true
grants = ["cargo-token"]
[[session]]
name = "worker"
command = ["cat"]
"#,
    )
    .expect("team cargo-token grant");
    let request = sessions[0]
        .confine
        .clone()
        .expect("team confinement request");
    assert_eq!(request.grants, vec!["cargo-token"]);
    let fixture = Fixture::new();
    let plan = confine::plan(&request, Harness::Generic, &fixture.cwd, &fixture.host())
        .expect("team cargo-token plan");
    let wrapped = plan
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("team cargo-token wrapper");
    assert!(
        wrapped
            .windows(2)
            .any(|window| window == ["--unsetenv", "NPM_TOKEN"])
    );
    assert!(
        !wrapped
            .windows(2)
            .any(|window| window == ["--unsetenv", "CARGO_REGISTRY_TOKEN"])
    );
    assert!(
        !wrapped
            .windows(2)
            .any(|window| window == ["--unsetenv", "CARGO_REGISTRIES_CRATES_IO_TOKEN"])
    );
}

#[test]
fn exact_unhide_of_grant_path_stays_writable() {
    let fixture = Fixture::new();
    let gh = fixture.home.join(".config/gh");
    std::fs::create_dir_all(&gh).expect("gh directory");
    let gh = std::fs::canonicalize(gh).expect("gh canonical");
    let plan = confine::plan(
        &ConfineRequest {
            grants: vec!["gh".into()],
            unhide: vec!["~/.config/gh".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("exact grant unhide plan");
    assert!(plan.writable.contains(&gh));
    assert!(!plan.readonly.contains(&gh));
}

#[test]
fn nested_unhide_inside_grant_stays_readonly_and_protected() {
    let fixture = Fixture::new();
    let ssh = fixture.home.join(".ssh");
    let config = std::fs::canonicalize(ssh.join("config")).expect("ssh config canonical");
    let plan = confine::plan(
        &ConfineRequest {
            grants: vec!["ssh".into()],
            unhide: vec!["~/.ssh/config".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("nested grant unhide plan");
    assert!(plan.readonly.contains(&config));
    assert!(plan.protected.contains(&config));
    let config = config.to_string_lossy();
    let wrapped = plan
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("nested grant unhide wrapper");
    assert!(
        wrapped.windows(3).any(|window| {
            window[0] == "--ro-bind" && window[1] == config && window[2] == config
        })
    );
}

#[test]
fn gh_grant_releases_its_environment_and_notice_names() {
    let fixture = Fixture::new();
    let plan = confine::plan(
        &ConfineRequest {
            grants: vec!["gh".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("gh environment plan");
    let notice = confine::notice(&plan);
    let wrapped = plan
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("gh environment wrapper");
    for name in ["GH_TOKEN", "GITHUB_TOKEN", "GH_ENTERPRISE_TOKEN"] {
        assert!(
            !wrapped
                .windows(2)
                .any(|window| window == ["--unsetenv", name]),
            "granted environment variable {name} was unset"
        );
    }
    for name in [
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
        assert!(
            wrapped
                .windows(2)
                .any(|window| window == ["--unsetenv", name]),
            "unrelated credential environment variable {name} was not unset"
        );
    }
    let line = notice
        .lines()
        .find(|line| line.starts_with("Credential environment variables are unset:"))
        .expect("credential notice line");
    let names = line
        .strip_prefix("Credential environment variables are unset: ")
        .expect("credential notice prefix")
        .strip_suffix('.')
        .expect("credential notice suffix");
    assert_eq!(names.split(", ").count(), 26);
    assert!(!names.contains("GH_TOKEN"));
    assert!(!names.contains("GITHUB_TOKEN"));
    assert!(!names.contains("GH_ENTERPRISE_TOKEN"));
}

#[test]
fn ssh_grant_unhides_home_ssh_and_stands_in_host_config() {
    let fixture = Fixture::new();
    let ssh_dir = std::fs::canonicalize(fixture.home.join(".ssh")).expect("ssh directory");
    let plain = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("plain plan");
    assert!(plain.hidden_paths().contains(&ssh_dir));

    let config_source = Path::new("/etc/ssh/ssh_config");
    let config = std::fs::canonicalize(config_source).ok();
    assert!(
        !plain.skipped_paths().contains(&config_source.to_owned()),
        "plain plans must not skip the SSH stand-in"
    );
    if let Some(config) = &config {
        assert!(!plain.hidden_paths().contains(config));
    }

    for no_default_hides in [false, true] {
        let plan = confine::plan(
            &ConfineRequest {
                grants: vec!["ssh".into()],
                no_default_hides,
                ..ConfineRequest::default()
            },
            Harness::Generic,
            &fixture.cwd,
            &fixture.host(),
        )
        .expect("ssh grant plan");
        assert!(plan.writable.contains(&ssh_dir));
        assert!(!plan.readonly.contains(&ssh_dir));
        assert!(!plan.protected.contains(&ssh_dir));
        assert!(!plan.hidden_paths().contains(&ssh_dir));
        let config_hidden = config
            .as_ref()
            .is_some_and(|config| plan.hidden_paths().contains(config));
        let config_skipped = plan.skipped_paths().contains(&config_source.to_owned());
        let wrapped = plan
            .with_bwrap(PathBuf::from("bwrap"))
            .wrap(vec!["true".into()])
            .expect("ssh grant wrapper");
        let ssh_dir = ssh_dir.to_string_lossy();
        assert!(wrapped.windows(3).any(|window| {
            window[0] == "--bind" && window[1] == ssh_dir && window[2] == ssh_dir
        }));
        assert!(!wrapped.windows(3).any(|window| {
            window[0] == "--ro-bind" && window[1] == ssh_dir && window[2] == ssh_dir
        }));
        match &config {
            Some(config) => {
                assert!(config_hidden);
                let empty = fixture.empty.to_string_lossy();
                let config = config.to_string_lossy();
                assert!(wrapped.windows(3).any(|window| {
                    window[0] == "--ro-bind" && window[1] == empty && window[2] == config
                }));
            }
            None => assert!(
                config_skipped,
                "missing SSH config must be reported as skipped"
            ),
        }
    }
}

#[test]
fn ssh_grant_operator_visibility_wins_and_key_hides_remain_supported() {
    let fixture = Fixture::new();
    let config_source = Path::new("/etc/ssh/ssh_config");
    let Some(config) = std::fs::canonicalize(config_source).ok() else {
        eprintln!("skipping SSH operator override assertions: host config is absent");
        return;
    };

    let unhidden = confine::plan(
        &ConfineRequest {
            grants: vec!["ssh".into()],
            unhide: vec![config_source.display().to_string()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("SSH unhide override");
    assert!(!unhidden.hidden_paths().contains(&config));

    let writable = confine::plan(
        &ConfineRequest {
            grants: vec!["ssh".into()],
            write: vec![config_source.display().to_string()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("SSH write override");
    assert!(!writable.hidden_paths().contains(&config));
    let wrapped = writable
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("SSH write wrapper");
    let config = config.to_string_lossy();
    assert!(
        wrapped
            .windows(3)
            .any(|window| { window[0] == "--bind" && window[1] == config && window[2] == config })
    );

    let whole_directory = match confine::plan(
        &ConfineRequest {
            grants: vec!["ssh".into()],
            hide: vec!["~/.ssh".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    ) {
        Ok(_) => panic!("whole SSH directory hide must conflict"),
        Err(error) => error,
    };
    assert!(
        whole_directory
            .to_string()
            .contains("conflicts with the hide")
    );

    let key = fixture.home.join(".ssh/id_work");
    std::fs::write(&key, "private key").expect("SSH key fixture");
    let key = std::fs::canonicalize(key).expect("SSH key path");
    let per_key = confine::plan(
        &ConfineRequest {
            grants: vec!["ssh".into()],
            hide: vec![key.display().to_string()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("per-key SSH hide");
    assert!(per_key.hidden_paths().contains(&key));
}

#[test]
fn ssh_grant_runs_ssh_query_through_bwrap() -> anyhow::Result<()> {
    let bwrap = match confine::resolve_bwrap(None) {
        Ok(path) => path,
        Err(error) if !require_bwrap() => {
            eprintln!("skipping SSH bwrap probe: {error:#}");
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    match Command::new("ssh").arg("-V").output() {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("skipping SSH bwrap probe: ssh is not installed");
            return Ok(());
        }
        Err(error) => return Err(error).context("probe ssh availability"),
    }
    let fixture = Fixture::new();
    let plan = confine::plan(
        &ConfineRequest {
            grants: vec!["ssh".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )?
    .with_bwrap(bwrap);
    let wrapped = plan.wrap(vec!["ssh".into(), "-G".into(), "example.invalid".into()])?;
    match Command::new(&wrapped[0]).args(&wrapped[1..]).status() {
        Ok(status) => {
            assert_eq!(status.code(), Some(0));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("skipping SSH bwrap probe: ssh is not installed");
        }
        Err(error) => return Err(error).context("run SSH bwrap probe"),
    }
    Ok(())
}

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
        "/run/dbus",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    assert_eq!(generic.len(), 31);
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
        "/run/dbus",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    assert_eq!(claude.len(), 30);
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
        "/run/dbus",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    assert_eq!(codex.len(), 30);
    assert_eq!(confine::default_hide_names(Harness::Codex), codex);

    let fixture = Fixture::new();
    let claude_credentials = fixture.home.join(".claude/.credentials.json");
    let codex_credentials = fixture.home.join(".codex/auth.json");
    std::fs::create_dir_all(claude_credentials.parent().expect("Claude parent"))
        .expect("Claude directory");
    std::fs::create_dir_all(codex_credentials.parent().expect("Codex parent"))
        .expect("Codex directory");
    std::fs::write(&claude_credentials, "fixture").expect("Claude credentials");
    std::fs::write(&codex_credentials, "fixture").expect("Codex credentials");

    let dbus = std::fs::canonicalize("/run/dbus").ok();
    for (harness, expected_credentials) in [
        (Harness::Claude, vec![&codex_credentials]),
        (
            Harness::Generic,
            vec![&claude_credentials, &codex_credentials],
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
            grants: vec!["docker".into()],
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
        !allowed
            .windows(2)
            .any(|window| window == ["--unsetenv", "DOCKER_CONTEXT"])
    );
    for name in [
        "CONTAINER_HOST",
        "CONTAINER_CONNECTION",
        "CONTAINERD_ADDRESS",
        "BUILDKIT_HOST",
        "LIBVIRT_DEFAULT_URI",
    ] {
        assert!(
            allowed
                .windows(2)
                .any(|window| window == ["--unsetenv", name])
        );
    }
    assert!(
        allowed
            .windows(2)
            .any(|window| window == ["--tmpfs", fixture.runtime.to_string_lossy().as_ref()])
    );
}
#[test]
fn wrap_unsets_every_credential_environment_variable_by_default() {
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
    for name in [
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
        assert!(
            wrapped
                .windows(2)
                .any(|window| window == ["--unsetenv", name]),
            "missing credential environment variable {name}"
        );
    }
    for name in [
        "ANTHROPIC_API_KEY",
        "A2AMX_TOKEN",
        "A2AMX_ADDR",
        "OPENAI_API_KEY",
    ] {
        assert!(
            !wrapped
                .windows(2)
                .any(|window| window == ["--unsetenv", name]),
            "harness environment variable {name} was unset"
        );
    }
}
#[test]
fn wrap_keeps_requested_credentials_and_never_unsets_harness_environment() {
    let fixture = Fixture::new();
    let kept = confine::plan(
        &ConfineRequest {
            keep_env: vec!["GITHUB_TOKEN".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("kept plan")
    .with_bwrap(PathBuf::from("bwrap"))
    .wrap(vec!["true".into()])
    .expect("kept wrap");
    for name in [
        "GH_TOKEN",
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
        assert!(
            kept.windows(2).any(|window| window == ["--unsetenv", name]),
            "missing unset for non-kept credential {name}"
        );
    }
    assert!(
        !kept
            .windows(2)
            .any(|window| window == ["--unsetenv", "GITHUB_TOKEN"])
    );
    for name in [
        "ANTHROPIC_API_KEY",
        "A2AMX_TOKEN",
        "A2AMX_ADDR",
        "OPENAI_API_KEY",
    ] {
        assert!(
            !kept.windows(2).any(|window| window == ["--unsetenv", name]),
            "harness environment variable {name} was unset"
        );
    }
}
#[test]
fn plan_sorts_kept_credential_names() {
    let fixture = Fixture::new();
    let plan = confine::plan(
        &ConfineRequest {
            keep_env: vec!["NPM_TOKEN".into(), "GH_TOKEN".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("plan");
    assert_eq!(plan.keep_env, ["GH_TOKEN", "NPM_TOKEN"]);
}

#[test]
fn keep_env_validation_is_closed_and_bounded() {
    let fixture = Fixture::new();
    let error_for = |keep_env| match confine::plan(
        &ConfineRequest {
            keep_env,
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    ) {
        Ok(_) => panic!("keep_env should be rejected"),
        Err(error) => error.to_string(),
    };
    assert!(error_for(vec!["ANTHROPIC_API_KEY".into()]).contains(
        "keep_env \"ANTHROPIC_API_KEY\" is not unset by default, so there is nothing to keep"
    ));
    assert!(error_for(vec!["DOCKER_HOST".into()]).contains(
        "keep_env \"DOCKER_HOST\" is not unset by default, so there is nothing to keep; it returns with the docker grant"
    ));
    assert_eq!(
        error_for(vec!["GITHUB_TOKEN".into(), "GITHUB_TOKEN".into()]),
        "keep_env \"GITHUB_TOKEN\" is repeated"
    );
    assert_eq!(
        error_for(vec!["GITHUB_TOKEN".into(); 65]),
        "keep_env has more than 64 entries"
    );
}

#[test]
fn notice_lists_credential_environment_variables_and_kept_names() {
    let fixture = Fixture::new();
    let default_notice = confine::notice(
        &confine::plan(
            &ConfineRequest::default(),
            Harness::Generic,
            &fixture.cwd,
            &fixture.host(),
        )
        .expect("default plan"),
    );
    assert!(default_notice.contains(
        "Credential environment variables are unset: GH_TOKEN, GITHUB_TOKEN, GH_ENTERPRISE_TOKEN, GITLAB_TOKEN, GITLAB_PRIVATE_TOKEN, CI_JOB_TOKEN, BITBUCKET_TOKEN, NPM_TOKEN, NODE_AUTH_TOKEN, PYPI_TOKEN, TWINE_PASSWORD, CARGO_REGISTRY_TOKEN, CARGO_REGISTRIES_CRATES_IO_TOKEN, AZURE_CLIENT_SECRET, ARM_CLIENT_SECRET, VAULT_TOKEN, HF_TOKEN, HUGGING_FACE_HUB_TOKEN, SONAR_TOKEN, SENTRY_AUTH_TOKEN, DIGITALOCEAN_ACCESS_TOKEN, HCLOUD_TOKEN, CLOUDFLARE_API_TOKEN, NETLIFY_AUTH_TOKEN, VERCEL_TOKEN, FLY_API_TOKEN, HEROKU_API_KEY, PGPASSWORD, MYSQL_PWD."
    ));
    let kept_notice = confine::notice(
        &confine::plan(
            &ConfineRequest {
                keep_env: vec!["GITHUB_TOKEN".into()],
                ..ConfineRequest::default()
            },
            Harness::Generic,
            &fixture.cwd,
            &fixture.host(),
        )
        .expect("kept plan"),
    );
    assert!(kept_notice.contains(
        "Credential environment variables are unset: GH_TOKEN, GH_ENTERPRISE_TOKEN, GITLAB_TOKEN, GITLAB_PRIVATE_TOKEN, CI_JOB_TOKEN, BITBUCKET_TOKEN, NPM_TOKEN, NODE_AUTH_TOKEN, PYPI_TOKEN, TWINE_PASSWORD, CARGO_REGISTRY_TOKEN, CARGO_REGISTRIES_CRATES_IO_TOKEN, AZURE_CLIENT_SECRET, ARM_CLIENT_SECRET, VAULT_TOKEN, HF_TOKEN, HUGGING_FACE_HUB_TOKEN, SONAR_TOKEN, SENTRY_AUTH_TOKEN, DIGITALOCEAN_ACCESS_TOKEN, HCLOUD_TOKEN, CLOUDFLARE_API_TOKEN, NETLIFY_AUTH_TOKEN, VERCEL_TOKEN, FLY_API_TOKEN, HEROKU_API_KEY, PGPASSWORD, MYSQL_PWD; kept: GITHUB_TOKEN."
    ));
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
    assert!(error.to_string().contains("docker grant"));
    let wrong_grant = match confine::plan(
        &ConfineRequest {
            unhide: vec!["~/.docker/run/docker.sock".into()],
            grants: vec!["podman".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    ) {
        Ok(_) => panic!("podman must not authorize a docker socket"),
        Err(error) => error,
    };
    assert!(
        wrong_grant
            .to_string()
            .contains("requires the docker grant")
    );
    let allowed = confine::plan(
        &ConfineRequest {
            unhide: vec!["~/.docker/run/docker.sock".into()],
            grants: vec!["docker".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("allow-docker switch");
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
    assert!(error.to_string().contains("docker grant"));
    let allowed = confine::plan(
        &ConfineRequest {
            write: vec!["~/.docker/run/docker.sock".into()],
            grants: vec!["docker".into()],
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
            grants: vec!["docker".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("allow docker plan");
    for socket in &sockets {
        let socket = std::fs::canonicalize(socket).expect("socket canonical");
        assert!(!allowed.hidden_paths().contains(&socket));
    }
    let allowed_wrapped = allowed
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("wrapped allow docker plan");
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
    .with_codex_dir(fixture.state.join("codex/s1"));
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
    let state = fixture.state.to_string_lossy().into_owned();
    let runtime = fixture.runtime.to_string_lossy().into_owned();
    let home = fixture.home.to_string_lossy().into_owned();
    let cwd = fixture.cwd.to_string_lossy().into_owned();
    let empty = fixture
        .state
        .join("confine-empty")
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
        "--tmpfs".into(),
        state.clone(),
        "--bind".into(),
        cwd.clone(),
        cwd.clone(),
        "--tmpfs".into(),
        format!("{home}/.aws"),
        "--tmpfs".into(),
        format!("{home}/.ssh"),
        "--bind".into(),
        format!("{home}/.codex"),
        format!("{home}/.codex"),
    ]);
    for path in &host_paths {
        expected.extend(["--remount-ro".into(), path.clone()]);
    }
    expected.extend([
        "--remount-ro".into(),
        runtime,
        "--remount-ro".into(),
        state,
        "--remount-ro".into(),
        format!("{home}/.aws"),
        "--remount-ro".into(),
        format!("{home}/.ssh"),
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
        .state
        .join("codex/s1")
        .to_string_lossy()
        .into_owned();
    let socket = fixture
        .state
        .join("codex/s1/sock")
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
    assert!(
        confine::check_write_allowed(&state_link, &fixture.home, &fixture.state, false).is_err()
    );
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

fn plan_with_timeout(repo: PathBuf, host: Host) -> anyhow::Result<confine::Plan> {
    let (sender, receiver) = std_mpsc::channel();
    std::thread::spawn(move || {
        let result = confine::plan(&ConfineRequest::default(), Harness::Generic, &repo, &host);
        let _ = sender.send(result);
    });
    receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("plan timed out")
}

#[test]
fn plan_does_not_hang_on_fifo_git_config() {
    let fixture = Fixture::new();
    let repo = fixture.cwd.join("repo");
    let git = repo.join(".git");
    let git_hooks = git.join("hooks");
    std::fs::create_dir_all(&git_hooks).expect("git hooks");
    let status = Command::new("mkfifo")
        .arg(git.join("config"))
        .status()
        .expect("mkfifo");
    assert!(status.success(), "mkfifo status: {status}");

    let plan = plan_with_timeout(repo, fixture.host()).expect("plan");
    assert!(
        plan.protected
            .contains(&std::fs::canonicalize(git.join("config")).expect("config canonical"))
    );
    assert!(
        plan.protected
            .contains(&std::fs::canonicalize(git_hooks).expect("git hooks canonical"))
    );
}

#[test]
fn plan_rejects_oversized_git_config() {
    let limit = 1024 * 1024;
    let fixture = Fixture::new();
    let repo = fixture.cwd.join("repo");
    let git = repo.join(".git");
    std::fs::create_dir_all(git.join("hooks")).expect("git hooks");
    std::fs::write(git.join("config"), vec![b'x'; limit + 1]).expect("oversized git config");
    let error = match plan_with_timeout(repo, fixture.host()) {
        Ok(_) => panic!("oversized config accepted"),
        Err(error) => error,
    };
    let text = error.to_string();
    assert!(text.contains("larger than"), "unexpected error: {text}");
    assert!(text.contains("hooksPath"), "unexpected error: {text}");

    let fixture = Fixture::new();
    let repo = fixture.cwd.join("repo");
    let git = repo.join(".git");
    let husky = repo.join(".husky");
    std::fs::create_dir_all(git.join("hooks")).expect("git hooks");
    std::fs::create_dir_all(&husky).expect("husky hooks");
    let prefix = "[user]\n\tpad = ";
    let suffix = "\n[core]\n\thooksPath = .husky\n";
    let padding = limit - prefix.len() - suffix.len();
    let config = format!("{prefix}{}{suffix}", "a".repeat(padding));
    assert_eq!(config.len(), limit);
    std::fs::write(git.join("config"), config).expect("exact-limit git config");
    let plan = plan_with_timeout(repo, fixture.host()).expect("exact-limit plan");
    assert!(
        plan.protected
            .contains(&std::fs::canonicalize(husky).expect("husky canonical"))
    );
}

#[test]
fn plan_protects_hooks_path_in_config_with_invalid_utf8() {
    let fixture = Fixture::new();
    let repo = fixture.cwd.join("repo");
    let git = repo.join(".git");
    let husky = repo.join(".husky");
    std::fs::create_dir_all(git.join("hooks")).expect("git hooks");
    std::fs::create_dir_all(&husky).expect("husky hooks");
    std::fs::write(
        git.join("config"),
        b"[user]\n\tname = \xe9\n[core]\n\thooksPath = .husky\n",
    )
    .expect("git config");

    let plan = plan_with_timeout(repo, fixture.host()).expect("plan");
    assert!(
        plan.protected
            .contains(&std::fs::canonicalize(husky).expect("husky canonical"))
    );
}

#[test]
fn plan_ignores_directory_git_config() {
    let fixture = Fixture::new();
    let repo = fixture.cwd.join("repo");
    let git = repo.join(".git");
    let git_hooks = git.join("hooks");
    std::fs::create_dir_all(&git_hooks).expect("git hooks");
    std::fs::create_dir(git.join("config")).expect("config directory");

    let plan = plan_with_timeout(repo, fixture.host()).expect("plan");
    assert!(
        plan.protected
            .contains(&std::fs::canonicalize(git.join("config")).expect("config canonical"))
    );
    assert!(
        plan.protected
            .contains(&std::fs::canonicalize(git_hooks).expect("git hooks canonical"))
    );
}

#[test]
fn plan_finds_hooks_path_after_lines_without_equals() {
    let fixture = Fixture::new();
    let repo = fixture.cwd.join("repo");
    let git = repo.join(".git");
    let husky = repo.join(".husky");
    std::fs::create_dir_all(git.join("hooks")).expect("git hooks");
    std::fs::create_dir_all(&husky).expect("husky hooks");
    std::fs::write(
        git.join("config"),
        "[core]\n\n\tbare\n# note\n\thooksPath = .husky\n",
    )
    .expect("git config");

    let plan = plan_with_timeout(repo, fixture.host()).expect("plan");
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
    std::fs::write(common.join("HEAD"), "ref: refs/heads/main\n").expect("common HEAD");
    std::fs::create_dir(common.join("objects")).expect("common objects");
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
    std::fs::write(submodule_git.join("HEAD"), "ref: refs/heads/main\n").expect("submodule HEAD");
    std::fs::create_dir(submodule_git.join("objects")).expect("submodule objects");
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
fn implicit_git_writes_inside_default_hides_are_refused() {
    let fixture = Fixture::new();
    let hidden_git = fixture.home.join(".ssh");
    std::fs::write(hidden_git.join("HEAD"), "ref: refs/heads/main\n").expect("git HEAD");
    std::fs::create_dir(hidden_git.join("objects")).expect("git objects");

    let direct = fixture.cwd.join("direct");
    std::fs::create_dir_all(&direct).expect("direct repository");
    std::fs::write(
        direct.join(".git"),
        format!("gitdir: {}\n", hidden_git.display()),
    )
    .expect("direct git pointer");
    let error = match confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &direct,
        &fixture.host(),
    ) {
        Ok(_) => panic!("git directory inside a default hide was writable"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("git directory"));
    assert!(error.to_string().contains("resolves to the hidden path"));

    let metadata = fixture.cwd.join("metadata");
    let commondir_repo = fixture.cwd.join("commondir-repo");
    std::fs::create_dir_all(&metadata).expect("metadata directory");
    std::fs::create_dir_all(&commondir_repo).expect("commondir repository");
    std::fs::write(
        commondir_repo.join(".git"),
        format!("gitdir: {}\n", metadata.display()),
    )
    .expect("commondir git pointer");
    std::fs::write(
        metadata.join("commondir"),
        format!("{}\n", hidden_git.display()),
    )
    .expect("commondir pointer");
    let error = match confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &commondir_repo,
        &fixture.host(),
    ) {
        Ok(_) => panic!("commondir inside a default hide was writable"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("git directory"));
    assert!(error.to_string().contains("resolves to the hidden path"));

    let explicit = confine::plan(
        &ConfineRequest {
            write: vec![hidden_git.to_string_lossy().into_owned()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &direct,
        &fixture.host(),
    )
    .expect("explicit git write");
    assert!(
        explicit
            .writable
            .contains(&hidden_git.canonicalize().expect("hidden git"))
    );

    let no_default_hides = confine::plan(
        &ConfineRequest {
            no_default_hides: true,
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &direct,
        &fixture.host(),
    )
    .expect("git write with default hides disabled");
    assert!(
        no_default_hides
            .writable
            .contains(&hidden_git.canonicalize().expect("hidden git"))
    );
}

#[test]
fn harness_data_links_stay_inside_expected_locations() {
    let hidden_dir = Fixture::new();
    let hidden_root = hidden_dir.home.join(".claude");
    std::fs::create_dir_all(&hidden_root).expect("Claude root");
    symlink(hidden_dir.home.join(".ssh"), hidden_root.join("todos")).expect("hidden todos symlink");
    let error = match confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &hidden_dir.cwd,
        &hidden_dir.host(),
    ) {
        Ok(_) => panic!("harness data symlink escaped into a hidden directory"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("harness data path"));
    assert!(error.to_string().contains("outside"));

    let hidden_file = Fixture::new();
    let hidden_file_root = hidden_file.home.join(".claude");
    std::fs::create_dir_all(&hidden_file_root).expect("Claude root");
    std::fs::write(hidden_file.home.join(".netrc"), "credentials").expect("netrc");
    symlink(
        hidden_file.home.join(".netrc"),
        hidden_file_root.join("history.jsonl"),
    )
    .expect("hidden history symlink");
    let error = match confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &hidden_file.cwd,
        &hidden_file.host(),
    ) {
        Ok(_) => panic!("harness data file escaped into a hidden file"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("harness data path"));
    assert!(error.to_string().contains("outside"));

    let outside = Fixture::new();
    let outside_root = outside.home.join(".claude");
    std::fs::create_dir_all(&outside_root).expect("Claude root");
    std::fs::create_dir_all(outside.home.join(".config")).expect("config");
    symlink(outside.home.join(".config"), outside_root.join("todos"))
        .expect("outside todos symlink");
    let error = match confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &outside.cwd,
        &outside.host(),
    ) {
        Ok(_) => panic!("harness data symlink escaped its root"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("harness data path"));
    assert!(error.to_string().contains("outside"));

    let explicit_config = Fixture::new();
    let explicit_config_root = explicit_config.home.join(".claude");
    std::fs::create_dir_all(&explicit_config_root).expect("Claude root");
    std::fs::create_dir_all(explicit_config.home.join(".config")).expect("config");
    symlink(
        explicit_config.home.join(".config"),
        explicit_config_root.join("todos"),
    )
    .expect("explicit config symlink");
    let explicit_config_plan = confine::plan(
        &ConfineRequest {
            write: vec![
                explicit_config
                    .home
                    .join(".config")
                    .to_string_lossy()
                    .into_owned(),
            ],
            ..ConfineRequest::default()
        },
        Harness::Claude,
        &explicit_config.cwd,
        &explicit_config.host(),
    )
    .expect("explicit harness config target");
    assert!(
        explicit_config_plan.writable.contains(
            &explicit_config
                .home
                .join(".config")
                .canonicalize()
                .expect("config canonical")
        )
    );

    let explicit = Fixture::new();
    let symlinked_root = Fixture::new();
    let dotfiles_root = symlinked_root.home.join("dotfiles/claude");
    std::fs::create_dir_all(dotfiles_root.join("todos")).expect("dotfiles Claude root");
    symlink(&dotfiles_root, symlinked_root.home.join(".claude")).expect("dotfiles Claude symlink");
    let symlinked_plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &symlinked_root.cwd,
        &symlinked_root.host(),
    )
    .expect("symlinked Claude root");
    assert!(
        symlinked_plan
            .writable
            .contains(&dotfiles_root.canonicalize().expect("dotfiles canonical"))
    );

    let explicit_root = explicit.home.join(".claude");
    std::fs::create_dir_all(&explicit_root).expect("Claude root");
    symlink(explicit.home.join(".ssh"), explicit_root.join("todos"))
        .expect("explicit todos symlink");
    let explicit_plan = confine::plan(
        &ConfineRequest {
            write: vec![explicit.home.join(".ssh").to_string_lossy().into_owned()],
            ..ConfineRequest::default()
        },
        Harness::Claude,
        &explicit.cwd,
        &explicit.host(),
    )
    .expect("explicit harness data target");
    assert!(
        explicit_plan.writable.contains(
            &explicit
                .home
                .join(".ssh")
                .canonicalize()
                .expect("SSH canonical")
        )
    );

    let in_root = Fixture::new();
    let in_root_claude = in_root.home.join(".claude");
    let ide = in_root_claude.join("ide");
    std::fs::create_dir_all(&ide).expect("Claude IDE directory");
    symlink(&ide, in_root_claude.join("todos")).expect("in-root todos symlink");
    let in_root_plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &in_root.cwd,
        &in_root.host(),
    )
    .expect("in-root harness data target");
    let ide = ide.canonicalize().expect("IDE canonical");
    assert!(in_root_plan.hidden_paths().contains(&ide));
    assert!(!in_root_plan.writable.contains(&ide));

    let home_file = Fixture::new();
    std::fs::write(home_file.home.join(".claude.json"), "{}").expect("Claude home file");
    confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &home_file.cwd,
        &home_file.host(),
    )
    .expect("Claude home data file");
}

#[test]
fn symlinked_home_with_claude_home_file_is_accepted() {
    let fixture = Fixture::new();
    let real_home = fixture._root.path().join("real-home");
    std::fs::rename(&fixture.home, &real_home).expect("real home");
    symlink(&real_home, &fixture.home).expect("home symlink");
    std::fs::write(real_home.join(".claude.json"), "{}").expect("Claude home file");

    confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("symlinked home with Claude home file");
}

#[test]
fn implicit_workspace_writes_inside_default_hides_are_refused() {
    let fixture = Fixture::new();
    let cwd = fixture.home.join(".password-store/sub");
    std::fs::create_dir_all(&cwd).expect("password store cwd");
    let error = match confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &cwd,
        &fixture.host(),
    ) {
        Ok(_) => panic!("workspace inside a default hide was writable"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("working directory"));
    assert!(error.to_string().contains("resolves to the hidden path"));

    let combined = Fixture::new();
    let combined_cwd = combined.home.join(".password-store/sub");
    let combined_git = combined_cwd.join(".git");
    std::fs::create_dir_all(&combined_git).expect("password store git directory");
    std::fs::write(combined_git.join("config"), "[core]\n").expect("git config");
    let error = match confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &combined_cwd,
        &combined.host(),
    ) {
        Ok(_) => panic!("workspace and git directory inside a default hide were writable"),
        Err(error) => error,
    };
    let text = error.to_string();
    assert!(text.contains("working directory"));
    assert!(text.contains("git directory"));

    confine::plan(
        &ConfineRequest {
            write: vec![
                combined_cwd.to_string_lossy().into_owned(),
                combined_git.to_string_lossy().into_owned(),
            ],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &combined_cwd,
        &combined.host(),
    )
    .expect("explicit workspace and git writes");
}

#[test]
fn git_common_dir_pointer_requires_git_shape() {
    let fixture = Fixture::new();
    let metadata = fixture.cwd.join("metadata");
    let worktree = fixture.cwd.join("worktree");
    std::fs::create_dir_all(&metadata).expect("metadata");
    std::fs::create_dir_all(&worktree).expect("worktree");
    std::fs::write(worktree.join(".git"), "gitdir: ../metadata\n").expect("git marker");
    let home = fixture.home.canonicalize().expect("home canonical");
    assert!(
        confine::git_common_dir(&worktree.canonicalize().expect("worktree canonical"), &home)
            .is_none()
    );

    std::fs::create_dir(metadata.join("objects")).expect("objects");
    assert!(
        confine::git_common_dir(&worktree.canonicalize().expect("worktree canonical"), &home)
            .is_none()
    );

    std::fs::remove_dir(metadata.join("objects")).expect("remove objects");
    std::fs::write(metadata.join("HEAD"), "ref: refs/heads/main\n").expect("HEAD");
    assert!(
        confine::git_common_dir(&worktree.canonicalize().expect("worktree canonical"), &home)
            .is_none()
    );

    std::fs::write(metadata.join("objects"), "not a directory").expect("objects file");
    assert!(
        confine::git_common_dir(&worktree.canonicalize().expect("worktree canonical"), &home)
            .is_none()
    );
    std::fs::remove_file(metadata.join("objects")).expect("remove objects file");

    std::fs::create_dir(metadata.join("objects")).expect("objects");
    assert!(
        confine::git_common_dir(&worktree.canonicalize().expect("worktree canonical"), &home)
            .is_some()
    );

    let config_worktree = fixture.cwd.join("config-worktree");
    std::fs::create_dir_all(&config_worktree).expect("config worktree");
    std::fs::write(
        config_worktree.join(".git"),
        format!("gitdir: {}\n", fixture.home.join(".config").display()),
    )
    .expect("config pointer");
    std::fs::create_dir_all(fixture.home.join(".config")).expect("config directory");
    assert!(
        confine::git_common_dir(
            &config_worktree
                .canonicalize()
                .expect("config worktree canonical"),
            &home
        )
        .is_none()
    );

    let plain = fixture.cwd.join("plain");
    std::fs::create_dir_all(&plain).expect("plain worktree");
    std::fs::write(
        plain.join(".git"),
        format!("gitdir: {}\n", fixture.home.join(".ssh").display()),
    )
    .expect("plain pointer");
    assert!(
        confine::git_common_dir(&plain.canonicalize().expect("plain canonical"), &home).is_none()
    );
    let plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &plain,
        &fixture.host(),
    )
    .expect("plain pointer plan");
    let ssh = fixture
        .home
        .join(".ssh")
        .canonicalize()
        .expect("SSH canonical");
    assert!(plan.hidden_paths().contains(&ssh));
    assert!(!plan.writable.contains(&ssh));
}

#[test]
fn git_common_dir_canonicalizes_commondir_targets() {
    let fixture = Fixture::new();
    let metadata = fixture.cwd.join("commondir-meta/nested");
    let worktree = fixture.cwd.join("commondir-worktree");
    let common = fixture.cwd.join("common.git");
    std::fs::create_dir_all(&metadata).expect("commondir metadata");
    std::fs::create_dir_all(&worktree).expect("commondir worktree");
    std::fs::create_dir_all(&common).expect("common git");
    std::fs::write(common.join("HEAD"), "ref: refs/heads/main\n").expect("common HEAD");
    std::fs::create_dir(common.join("objects")).expect("common objects");
    std::fs::write(
        worktree.join(".git"),
        format!("gitdir: {}\n", metadata.display()),
    )
    .expect("commondir git pointer");
    std::fs::write(metadata.join("commondir"), "../../common.git\n").expect("relative commondir");
    let home = fixture.home.canonicalize().expect("home canonical");
    let worktree = worktree.canonicalize().expect("worktree canonical");
    let expected = common.canonicalize().expect("common canonical");
    let (actual, _) = confine::git_common_dir(&worktree, &home).expect("relative common dir");
    assert_eq!(actual, expected);

    std::fs::write(
        metadata.join("commondir"),
        format!(
            "{}/commondir-meta/nested/../../common.git\n",
            fixture.cwd.display()
        ),
    )
    .expect("absolute commondir");
    let (actual, _) = confine::git_common_dir(&worktree, &home).expect("absolute common dir");
    assert_eq!(actual, expected);

    let home_metadata = fixture.cwd.join("home-meta/nested");
    let home_worktree = fixture.cwd.join("home-worktree");
    std::fs::create_dir_all(&home_metadata).expect("home metadata");
    std::fs::create_dir_all(&home_worktree).expect("home worktree");
    std::fs::write(
        home_worktree.join(".git"),
        format!("gitdir: {}\n", home_metadata.display()),
    )
    .expect("home git pointer");
    std::fs::write(
        home_metadata.join("commondir"),
        format!("{}\n", fixture.home.join("../home").display()),
    )
    .expect("home commondir");
    std::fs::write(fixture.home.join("HEAD"), "ref: refs/heads/main\n").expect("home HEAD");
    std::fs::create_dir(fixture.home.join("objects")).expect("home objects");
    assert!(
        confine::git_common_dir(
            &home_worktree
                .canonicalize()
                .expect("home worktree canonical"),
            &home
        )
        .is_none()
    );
}

#[test]
fn git_common_dir_rejects_nonregular_head_without_hanging() {
    let fixture = Fixture::new();
    let metadata = fixture.cwd.join("metadata");
    let worktree = fixture.cwd.join("worktree");
    std::fs::create_dir_all(&metadata).expect("metadata");
    std::fs::create_dir_all(&worktree).expect("worktree");
    std::fs::create_dir(metadata.join("HEAD")).expect("HEAD directory");
    std::fs::create_dir(metadata.join("objects")).expect("objects");
    std::fs::write(worktree.join(".git"), "gitdir: ../metadata\n").expect("git marker");
    let result = git_common_dir_with_timeout(
        worktree.canonicalize().expect("worktree canonical"),
        fixture.home.canonicalize().expect("home canonical"),
    );
    assert!(result.is_none());

    std::fs::remove_dir(metadata.join("HEAD")).expect("remove HEAD directory");
    let status = Command::new("mkfifo")
        .arg(metadata.join("HEAD"))
        .status()
        .expect("mkfifo");
    assert!(status.success(), "mkfifo status: {status}");
    let result = git_common_dir_with_timeout(
        worktree.canonicalize().expect("worktree canonical"),
        fixture.home.canonicalize().expect("home canonical"),
    );
    assert!(result.is_none());
}

fn git_common_dir_with_timeout(cwd: PathBuf, home: PathBuf) -> Option<(PathBuf, PathBuf)> {
    let (sender, receiver) = std_mpsc::channel();
    std::thread::spawn(move || {
        let result = confine::git_common_dir(&cwd, &home);
        let _ = sender.send(result);
    });
    receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("git common directory lookup timed out")
}

#[test]
fn git_common_dir_rejects_fifo_commondir_without_hanging() {
    let fixture = Fixture::new();
    let worktree = fixture.cwd.join("worktree");
    let metadata = fixture.cwd.join("meta/worktree");
    std::fs::create_dir_all(&worktree).expect("worktree");
    std::fs::create_dir_all(&metadata).expect("worktree metadata");
    std::fs::write(worktree.join(".git"), "gitdir: ../meta/worktree\n").expect("gitdir marker");
    let fifo = metadata.join("commondir");
    let status = Command::new("mkfifo").arg(&fifo).status().expect("mkfifo");
    assert!(status.success(), "mkfifo status: {status}");
    let result = git_common_dir_with_timeout(
        std::fs::canonicalize(&worktree).expect("canonical worktree"),
        std::fs::canonicalize(&fixture.home).expect("canonical home"),
    );
    assert!(result.is_none());
}

#[test]
fn git_common_dir_rejects_oversized_git_marker() {
    let fixture = Fixture::new();
    let worktree = fixture.cwd.join("worktree");
    let metadata = fixture.cwd.join("metadata");
    std::fs::create_dir_all(&worktree).expect("worktree");
    std::fs::create_dir_all(&metadata).expect("gitdir");
    std::fs::write(
        worktree.join(".git"),
        format!("gitdir: ../metadata\n{}", "x".repeat(4096)),
    )
    .expect("oversized git marker");
    let result = git_common_dir_with_timeout(
        std::fs::canonicalize(&worktree).expect("canonical worktree"),
        std::fs::canonicalize(&fixture.home).expect("canonical home"),
    );
    assert!(result.is_none());
}

#[test]
fn git_common_dir_rejects_directory_commondir() {
    let fixture = Fixture::new();
    let worktree = fixture.cwd.join("worktree");
    let metadata = fixture.cwd.join("meta/worktree");
    std::fs::create_dir_all(&worktree).expect("worktree");
    std::fs::create_dir_all(metadata.join("commondir")).expect("commondir directory");
    std::fs::write(worktree.join(".git"), "gitdir: ../meta/worktree\n").expect("gitdir marker");
    let result = git_common_dir_with_timeout(
        std::fs::canonicalize(&worktree).expect("canonical worktree"),
        std::fs::canonicalize(&fixture.home).expect("canonical home"),
    );
    assert!(result.is_none());
}

#[test]
fn git_common_dir_rejects_oversized_commondir() {
    let fixture = Fixture::new();
    let worktree = fixture.cwd.join("worktree");
    let metadata = fixture.cwd.join("meta/worktree");
    let common = fixture.cwd.join("common.git");
    std::fs::create_dir_all(&worktree).expect("worktree");
    std::fs::create_dir_all(&metadata).expect("worktree metadata");
    std::fs::create_dir_all(&common).expect("common git");
    std::fs::write(common.join("HEAD"), "ref: refs/heads/main\n").expect("common HEAD");
    std::fs::create_dir(common.join("objects")).expect("common objects");
    std::fs::write(worktree.join(".git"), "gitdir: ../meta/worktree\n").expect("gitdir marker");
    std::fs::write(
        metadata.join("commondir"),
        format!("../../common.git\n{}", " ".repeat(4096)),
    )
    .expect("oversized commondir");
    let result = git_common_dir_with_timeout(
        worktree.canonicalize().expect("worktree canonical"),
        fixture.home.canonicalize().expect("home canonical"),
    );
    assert!(result.is_none());
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
            hide: vec![fixture.cwd.join("missing").to_string_lossy().into_owned()],
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
            grants: vec!["docker".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("plan");
    assert!(
        confine::notice(&allowed)
            .contains("Container engines allowed: docker. Other container engines are restricted.")
    );
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
fn notice_always_names_the_working_directory() {
    let fixture = Fixture::new();
    let mut plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("plan");
    plan.writable.clear();
    plan.writable
        .extend((0..25).map(|index| PathBuf::from(format!("/a-{index:02}"))));
    plan.writable.push(plan.cwd.clone());
    let text = confine::notice(&plan);
    let writable_line = text
        .lines()
        .find(|line| line.starts_with("Writable locations: "))
        .expect("writable notice line");
    assert!(writable_line.starts_with(&format!("Writable locations: {},", plan.cwd.display())));
    assert!(writable_line.contains("and 6 more"));

    plan.writable.retain(|path| path != &plan.cwd);
    let text = confine::notice(&plan);
    let writable_line = text
        .lines()
        .find(|line| line.starts_with("Writable locations: "))
        .expect("writable notice line");
    assert!(!writable_line.contains(&plan.cwd.display().to_string()));
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
    // Keep this outside the workspace and sandbox-private /tmp so it stays writable with a read-only $HOME.
    let external = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR"))?;
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
    let session_id = match response {
        Response::Created { session } => session,
        response => {
            if require_bwrap() {
                panic!("confined session creation failed: {response:?}");
            }
            daemon.shutdown().await?;
            return Ok(());
        }
    };
    std::fs::write(&late_file, "file")?;
    let listener = tokio::net::UnixListener::bind(&socket_path)?;
    let accepted = tokio::select! {
        accepted = timeout(Duration::from_secs(10), listener.accept()) => match accepted {
            Ok(result) => Some(result?),
            Err(_) => None,
        },
        exited = async {
            loop {
                let sessions = match client.request(Request::List).await? {
                    Response::Sessions { sessions } => sessions,
                    response => anyhow::bail!("unexpected list response: {response:?}"),
                };
                if let Some(summary) = sessions
                    .into_iter()
                    .find(|summary| summary.id == session_id && summary.exit_code.is_some())
                {
                    break Ok::<_, anyhow::Error>(summary);
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        } => {
            let summary = exited?;
            panic!(
                "confined session {session_id} exited before accepting socket: exit_code={:?}",
                summary.exit_code
            );
        }
    };
    let (mut stream, _) = match accepted {
        Some(accepted) => accepted,
        None => {
            let summary = match client.request(Request::List).await? {
                Response::Sessions { sessions } => sessions
                    .into_iter()
                    .find(|summary| summary.id == session_id),
                response => anyhow::bail!("unexpected list response: {response:?}"),
            };
            let exit_code = summary.as_ref().and_then(|summary| summary.exit_code);
            let hold_reason = summary
                .as_ref()
                .and_then(|summary| summary.hold_reason.as_deref());
            panic!(
                "confined session {session_id} did not accept socket: exit_code={exit_code:?}, hold_reason={hold_reason:?}"
            );
        }
    };
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
            remote: None,
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn confined_generic_session_is_refused_when_env_lacks_the_signal_flags() -> anyhow::Result<()>
{
    let root = tempfile::tempdir()?;
    let state = root.path().join("state");
    let cwd = root.path().join("workspace");
    std::fs::create_dir_all(&cwd)?;
    let bwrap_dir = bwrap_probe_fail_stub_path_dir();
    let env_dir = stub_path_dir(42);
    let daemon =
        start_confine_daemon_with_env(&state, bwrap_dir.path(), Some(env_dir.path().join("bwrap")))
            .await?;
    let mut client = Client::connect(&state).await?;
    let response = client
        .request(confined_shell_request(
            &cwd,
            "printf ran > marker",
            vec![],
            ConfineRequest::default(),
        ))
        .await?;
    let Response::Error { message } = response else {
        panic!("expected confinement preflight error");
    };
    assert!(message.contains("--ignore-signal"));
    assert!(message.contains("--default-signal"));
    assert!(!cwd.join("marker").exists());
    daemon.shutdown().await?;
    Ok(())
}

async fn run_ctrl_c_case(
    state: &Path,
    cwd: &Path,
    script: String,
    ready: &Path,
    marker: Option<&Path>,
    expected_exit: i32,
) -> anyhow::Result<bool> {
    let bwrap_dir = bwrap_path_dir(Path::new("/usr/bin/bwrap"));
    let daemon = match start_confine_daemon(state, bwrap_dir.path()).await {
        Ok(daemon) => daemon,
        Err(_error) if !require_bwrap() => return Ok(false),
        Err(error) => return Err(error),
    };
    let mut client = Client::connect(state).await.context("connect client")?;
    let response = client
        .request(confined_shell_request(
            cwd,
            &script,
            vec![],
            ConfineRequest::default(),
        ))
        .await
        .context("create confined session")?;
    let session_id = match response {
        Response::Created { session } => session,
        response => {
            daemon.shutdown().await?;
            if !require_bwrap() {
                return Ok(false);
            }
            anyhow::bail!("confined session creation failed: {response:?}");
        }
    };

    common::eventually(|| async { ready.exists().then_some(()) }).await;
    let mut attachment = Client::connect(state)
        .await
        .context("connect attach client")?
        .attach(&session_id, false, 80, 24)
        .await
        .context("attach confined session")?;
    attachment
        .send(ClientFrame::Input(vec![0x03]))
        .await
        .context("send Ctrl-C input")?;
    drop(attachment);

    if let Some(marker) = marker {
        common::eventually(|| async { marker.exists().then_some(()) }).await;
    }
    let state_for_poll = state.to_owned();
    let session_id_for_poll = session_id.clone();
    let summary = common::eventually(|| {
        let state = state_for_poll.clone();
        let session_id = session_id_for_poll.clone();
        async move {
            let mut poll_client = Client::connect(&state).await.ok()?;
            let Response::Sessions { sessions } = poll_client.request(Request::List).await.ok()?
            else {
                return None;
            };
            sessions.into_iter().find(|session| {
                session.id == session_id && session.exit_code == Some(expected_exit)
            })
        }
    })
    .await;
    assert_eq!(summary.exit_code, Some(expected_exit));
    daemon.shutdown().await?;
    Ok(true)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ctrl_c_runs_the_trap_of_a_confined_shell() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let state = root.path().join("state");
    let cwd = root.path().join("workspace");
    std::fs::create_dir_all(&cwd)?;
    let ready = cwd.join("trap-ready");
    let marker = cwd.join("trap-marker");
    let script = format!(
        "trap 'echo got-int > {}; exit 7' INT; echo ready > {}; while :; do sleep 0.2; done",
        marker.display(),
        ready.display()
    );
    if !run_ctrl_c_case(&state, &cwd, script, &ready, Some(&marker), 7).await? {
        return Ok(());
    }
    assert_eq!(std::fs::read_to_string(marker)?, "got-int\n");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ctrl_c_does_not_end_a_confined_command_that_ignores_it() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let state = root.path().join("state");
    let cwd = root.path().join("workspace");
    std::fs::create_dir_all(&cwd)?;
    let ready = cwd.join("ignore-ready");
    let marker = cwd.join("ignore-marker");
    let script = format!(
        "trap '' INT; echo ready > {}; sleep 2; echo survived > {}",
        ready.display(),
        marker.display()
    );
    if !run_ctrl_c_case(&state, &cwd, script, &ready, Some(&marker), 0).await? {
        return Ok(());
    }
    assert_eq!(std::fs::read_to_string(marker)?, "survived\n");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ctrl_c_still_ends_a_confined_command_without_a_trap() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let state = root.path().join("state");
    let cwd = root.path().join("workspace");
    std::fs::create_dir_all(&cwd)?;
    let ready = cwd.join("sleep-ready");
    let script = format!("echo ready > {}; sleep 30", ready.display());
    if !run_ctrl_c_case(&state, &cwd, script, &ready, None, 130).await? {
        return Ok(());
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ctrl_c_reaches_a_confined_raw_mode_reader_as_input() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let state = root.path().join("state");
    let cwd = root.path().join("workspace");
    std::fs::create_dir_all(&cwd)?;
    let ready = cwd.join("raw-ready");
    let marker = cwd.join("raw-marker");
    let script = format!(
        "stty raw -echo; echo ready > {}; head -c1 > {}",
        ready.display(),
        marker.display()
    );
    if !run_ctrl_c_case(&state, &cwd, script, &ready, Some(&marker), 0).await? {
        return Ok(());
    }
    assert_eq!(std::fs::read(marker)?, [0x03]);
    Ok(())
}

#[test]
fn launch_argv_prefixes_generic_with_env_wrappers() {
    let fixture = Fixture::new();
    let plan = |harness| {
        confine::plan(
            &ConfineRequest::default(),
            harness,
            &fixture.cwd,
            &fixture.host(),
        )
        .expect("plan")
        .with_bwrap(PathBuf::from("/usr/bin/bwrap"))
    };
    let generic = confine::launch_argv(
        &plan(Harness::Generic),
        "",
        vec!["sh".into(), "-c".into(), "true".into()],
        Harness::Generic,
    )
    .expect("generic launch argv");
    assert_eq!(generic[0], "/usr/bin/env");
    assert_eq!(generic[1], "--ignore-signal=INT");
    assert_eq!(generic[2], "/usr/bin/bwrap");
    assert!(generic.windows(4).any(|window| {
        window
            .iter()
            .map(String::as_str)
            .eq(["--", "/usr/bin/env", "--default-signal=INT", "sh"])
    }));

    for harness in [Harness::Claude, Harness::Omp] {
        let argv = confine::launch_argv(
            &plan(harness),
            "",
            vec!["sh".into(), "-c".into(), "true".into()],
            harness,
        )
        .expect("harness launch argv");
        assert_eq!(argv.first().map(String::as_str), Some("/usr/bin/bwrap"));
        assert!(
            !argv
                .iter()
                .any(|argument| argument == "--default-signal=INT")
        );
    }
}

#[test]
fn preflight_env_refuses_an_env_without_the_flags() {
    let failing = stub_path_dir(42);
    let failing_env = failing.path().join("bwrap");
    let error = confine::preflight_env(&failing_env).expect_err("failing env probe");
    let message = error.to_string();
    assert!(message.contains("--ignore-signal"));
    assert!(message.contains("--default-signal"));

    let passing = stub_path_dir(0);
    assert!(confine::preflight_env(&passing.path().join("bwrap")).is_ok());

    let recorder = tempfile::tempdir().expect("env recorder directory");
    let recorder_script = recorder.path().join("env");
    std::fs::write(
        &recorder_script,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"${0%/*}/args\"\n",
    )
    .expect("env recorder script");
    let mut permissions = std::fs::metadata(&recorder_script)
        .expect("env recorder metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&recorder_script, permissions).expect("env recorder permissions");
    confine::preflight_env(&recorder_script).expect("recording env probe");
    let recorded = std::fs::read_to_string(recorder.path().join("args"))
        .expect("env recorder output")
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let recorder_path = recorder_script.to_string_lossy().into_owned();
    assert_eq!(
        recorded,
        vec![
            "--ignore-signal=INT".to_owned(),
            recorder_path.clone(),
            "--default-signal=INT".to_owned(),
            recorder_path,
        ]
    );

    let missing = Path::new("/definitely/missing/a2amx-env");
    let error = confine::preflight_env(missing).expect_err("missing env probe");
    assert!(error.to_string().contains("a2amx-env"));
}

#[test]
fn grants_are_validated_and_reported_by_the_plan() {
    let fixture = Fixture::new();
    let plan = confine::plan(
        &ConfineRequest {
            grants: vec!["docker".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("docker grant");
    assert_eq!(plan.grants, ["docker"]);
    assert!(confine::notice(&plan).contains("Granted: docker."));

    let error = match confine::plan(
        &ConfineRequest {
            grants: vec!["container".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    ) {
        Ok(_) => panic!("unknown grant"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("closest match: containerd"));
}

#[test]
fn grant_count_limit_precedes_grant_lookup() {
    let fixture = Fixture::new();
    let error = match confine::plan(
        &ConfineRequest {
            grants: (0..65).map(|_| "docker".to_owned()).collect(),
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    ) {
        Ok(_) => panic!("grant count limit was not enforced"),
        Err(error) => error,
    };
    assert_eq!(error.to_string(), "grants has more than 64 entries");
}
#[test]
fn write_home_is_allowed_and_keeps_hides_on_top() {
    let fixture = Fixture::new();
    let root = std::fs::canonicalize(fixture._root.path()).expect("fixture root");
    let netrc = fixture.home.join(".netrc");
    std::fs::write(&netrc, []).expect("netrc");
    let plan = confine::plan(
        &ConfineRequest {
            write: vec![root.to_string_lossy().into_owned()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("ancestor write plan");
    let root = root.to_string_lossy().into_owned();
    let empty = std::fs::canonicalize(&fixture.empty)
        .expect("empty canonical")
        .to_string_lossy()
        .into_owned();
    let hidden = plan.hidden_paths().to_vec();
    let wrapped = plan
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("wrapped ancestor plan");
    let writable_index = wrapped
        .windows(3)
        .position(|window| window == ["--bind", root.as_str(), root.as_str()])
        .expect("ancestor writable bind");
    for path in [
        fixture.state.clone(),
        fixture.runtime.clone(),
        fixture.home.join(".ssh"),
        netrc,
    ] {
        let path = std::fs::canonicalize(path).expect("hidden path");
        assert!(hidden.contains(&path), "missing hidden path {path:?}");
        let path = path.to_string_lossy().into_owned();
        let hide_index = wrapped
            .windows(2)
            .position(|window| window == ["--tmpfs", path.as_str()])
            .or_else(|| {
                wrapped
                    .windows(3)
                    .position(|window| window == ["--ro-bind", empty.as_str(), path.as_str()])
            })
            .expect("hide mount");
        assert!(
            hide_index > writable_index,
            "hide mount must follow ancestor writable mount"
        );
    }

    let plan = confine::plan(
        &ConfineRequest {
            write: vec!["~".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("home write plan");
    assert!(plan.writable.contains(&fixture.home));
    for path in [fixture.home.join(".ssh"), fixture.home.join(".netrc")] {
        assert!(
            plan.hidden_paths()
                .contains(&std::fs::canonicalize(path).expect("home hide"))
        );
    }
}

#[test]
fn write_home_keeps_the_launch_inputs_read_only() {
    let fixture = Fixture::new();
    let install = fixture.home.join(".a2amx");
    let path_dir = fixture.home.join(".local/bin");
    let relative = fixture.home.join("relative");
    std::fs::create_dir_all(&install).expect("install directory");
    std::fs::create_dir_all(&path_dir).expect("PATH directory");
    std::fs::create_dir_all(&relative).expect("relative PATH directory");
    let path_var = OsString::from(format!("{}:/usr/bin", path_dir.display()));
    let plan = |write: Vec<String>, path_var: OsString| {
        confine::plan_with_path(
            &ConfineRequest {
                write,
                ..ConfineRequest::default()
            },
            Harness::Generic,
            &fixture.cwd,
            &fixture.host(),
            Some(path_var),
        )
        .expect("launch-input plan")
    };

    let base = plan(vec!["~".into()], path_var.clone());
    let install = std::fs::canonicalize(&install).expect("install canonical");
    let path_dir = std::fs::canonicalize(&path_dir).expect("PATH canonical");
    let relative = std::fs::canonicalize(&relative).expect("relative canonical");
    assert!(base.protected.contains(&install));
    assert!(base.protected.contains(&path_dir));
    assert!(
        !base
            .readonly
            .contains(&std::fs::canonicalize("/usr/bin").expect("usr bin"))
    );
    assert!(
        !base
            .protected
            .contains(&std::fs::canonicalize("/usr/bin").expect("usr bin"))
    );

    let install_open = plan(vec!["~".into(), "~/.a2amx".into()], path_var.clone());
    assert!(!install_open.protected.contains(&install));
    assert!(install_open.protected.contains(&path_dir));

    let path_open = plan(vec!["~".into(), "~/.local/bin".into()], path_var.clone());
    assert!(path_open.protected.contains(&install));
    assert!(!path_open.protected.contains(&path_dir));

    let relative_path = plan(
        vec!["~".into()],
        OsString::from(format!("relative::{}", path_dir.display())),
    );
    assert!(!relative_path.readonly.contains(&relative));
    assert!(!relative_path.protected.contains(&relative));
}
#[test]
fn grant_create_paths_are_preflighted_and_writable() {
    let fixture = Fixture::new();
    let host = fixture.host();
    let request = ConfineRequest {
        grants: vec!["cargo".into(), "cache".into()],
        ..ConfineRequest::default()
    };
    let preflight = confine::ensure_grant_dirs(&request, Harness::Generic, &host, true);
    assert!(preflight.is_empty());
    let plan = confine::plan_with_preflight(
        &request,
        Harness::Generic,
        &fixture.cwd,
        &host,
        None,
        preflight,
    )
    .expect("grant plan");
    let cargo = std::fs::canonicalize(fixture.home.join(".cargo")).expect("created cargo");
    let cache = std::fs::canonicalize(fixture.home.join(".cache")).expect("created cache");
    assert!(plan.writable.contains(&cargo));
    assert!(plan.writable.contains(&cache));
}

#[test]
fn grant_create_path_inside_state_is_unresolved() {
    let fixture = Fixture::new();
    let cache = fixture.home.join(".cache");
    let state = cache.join("state");
    std::fs::create_dir_all(&state).expect("nested state");
    let empty_file = state.join("confine-empty");
    std::fs::write(&empty_file, []).expect("empty file");
    let host = Host {
        home: fixture.home.clone(),
        state_dir: state,
        runtime_dir: fixture.runtime.clone(),
        empty_file,
    };
    let request = ConfineRequest {
        grants: vec!["cache".into()],
        ..ConfineRequest::default()
    };
    let preflight = confine::ensure_grant_dirs(&request, Harness::Generic, &host, true);
    let expected = std::fs::canonicalize(cache).expect("cache path");
    assert!(preflight.iter().any(|item| {
        item.option == "grant" && item.path == expected && item.reason.contains("protected")
    }));
}

#[test]
fn grant_create_matrix_is_safe_and_explicit() {
    let node = Fixture::new();
    let node_request = ConfineRequest {
        grants: vec!["node".into()],
        ..ConfineRequest::default()
    };
    let unresolved =
        confine::ensure_grant_dirs(&node_request, Harness::Generic, &node.host(), true);
    assert!(unresolved.is_empty());
    assert!(node.home.join(".npm").is_dir());

    let aws = Fixture::new();
    let aws_request = ConfineRequest {
        grants: vec!["aws".into()],
        ..ConfineRequest::default()
    };
    std::fs::remove_dir_all(aws.home.join(".aws")).expect("remove fixture aws directory");
    confine::ensure_grant_dirs(&aws_request, Harness::Generic, &aws.host(), true);
    assert!(!aws.home.join(".aws").exists());

    let dry = Fixture::new();
    confine::ensure_grant_dirs(&node_request, Harness::Generic, &dry.host(), false);
    assert!(!dry.home.join(".npm").exists());

    let symlinked = Fixture::new();
    let target = symlinked._root.path().join("npm-target");
    std::fs::create_dir_all(&target).expect("npm target");
    symlink(&target, symlinked.home.join(".npm")).expect("npm symlink");
    let unresolved =
        confine::ensure_grant_dirs(&node_request, Harness::Generic, &symlinked.host(), true);
    assert!(
        unresolved
            .iter()
            .any(|item| { item.option == "grant" && item.path == symlinked.home.join(".npm") })
    );
    assert!(!target.join("created").exists());
}

#[test]
fn grant_paths_keep_nested_credentials_hidden() {
    let fixture = Fixture::new();
    let cargo = fixture.home.join(".cargo");
    let cache = fixture.home.join(".cache");
    let credentials = cargo.join("credentials.toml");
    let token = cache.join("huggingface/token");
    std::fs::create_dir_all(&cargo).expect("cargo directory");
    std::fs::create_dir_all(token.parent().expect("token parent")).expect("cache directories");
    std::fs::write(&credentials, "credentials").expect("cargo credentials");
    std::fs::write(&token, "token").expect("huggingface token");
    let request = ConfineRequest {
        grants: vec!["cargo".into(), "cache".into()],
        ..ConfineRequest::default()
    };
    let plan = confine::plan(&request, Harness::Generic, &fixture.cwd, &fixture.host())
        .expect("grant path plan");
    let cargo = std::fs::canonicalize(cargo).expect("cargo path");
    let cache = std::fs::canonicalize(cache).expect("cache path");
    let credentials = std::fs::canonicalize(credentials).expect("credentials path");
    let token = std::fs::canonicalize(token).expect("token path");
    assert!(plan.writable.contains(&cargo));
    assert!(plan.writable.contains(&cache));
    assert!(plan.hidden_paths().contains(&credentials));
    assert!(plan.hidden_paths().contains(&token));
}

#[test]
fn real_bubblewrap_cargo_grant_keeps_credentials_and_bin_protected() -> anyhow::Result<()> {
    let fixture = Fixture::new();
    let cargo = fixture.home.join(".cargo");
    let credentials = cargo.join("credentials.toml");
    let cargo_bin = cargo.join("bin");
    std::fs::create_dir_all(&cargo_bin)?;
    std::fs::write(&credentials, "credentials")?;
    std::fs::write(cargo_bin.join("installed"), "binary")?;
    let bwrap = match confine::resolve_bwrap(None) {
        Ok(path) => path,
        Err(error) if !require_bwrap() => {
            eprintln!("skipping cargo grant bwrap test: {error:#}");
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let path_var = OsString::from(format!("{}:/usr/bin", cargo_bin.display()));
    let plan = confine::plan_with_path(
        &ConfineRequest {
            grants: vec!["cargo".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
        Some(path_var),
    )?
    .with_bwrap(bwrap);
    let new_file = cargo.join("new");
    let bin_file = cargo_bin.join("new");
    let script = format!(
        "set -eu; test ! -s '{}'; ! printf x > '{}'; ! printf x > '{}'; printf y > '{}'",
        credentials.display(),
        credentials.display(),
        bin_file.display(),
        new_file.display()
    );
    let wrapped = plan.wrap(vec!["sh".into(), "-c".into(), script])?;
    let status = Command::new(&wrapped[0])
        .args(&wrapped[1..])
        .status()
        .context("run cargo grant bwrap probe")?;
    assert_eq!(status.code(), Some(0));
    assert_eq!(std::fs::read_to_string(&credentials)?, "credentials");
    assert_eq!(std::fs::read_to_string(&new_file)?, "y");
    assert!(!bin_file.exists());
    Ok(())
}

#[test]
fn real_bubblewrap_hides_nested_credentials_but_allows_cache_writes() -> anyhow::Result<()> {
    let fixture = Fixture::new();
    let cargo = fixture.home.join(".cargo");
    std::fs::create_dir_all(&cargo)?;
    std::fs::write(cargo.join("credentials.toml"), "credentials")?;
    std::fs::write(fixture.cwd.join("Cargo.toml"), "")?;
    let bwrap = match confine::resolve_bwrap(None) {
        Ok(path) => path,
        Err(error) if !require_bwrap() => {
            eprintln!("skipping nested credential bwrap test: {error:#}");
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let plan = confine::plan_with_path(
        &ConfineRequest {
            grants: vec!["cargo".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
        None,
    )?
    .with_bwrap(bwrap);
    let credentials = cargo.join("credentials.toml");
    let new_file = cargo.join("new");
    let script = format!(
        "set -eu; test ! -s '{}'; printf x > '{}'",
        credentials.display(),
        new_file.display()
    );
    let wrapped = plan.wrap(vec!["sh".into(), "-c".into(), script])?;
    let status = Command::new(&wrapped[0])
        .args(&wrapped[1..])
        .status()
        .context("run nested credential bwrap probe")?;
    assert_eq!(status.code(), Some(0));
    assert_eq!(std::fs::read_to_string(&credentials)?, "credentials");
    assert_eq!(std::fs::read_to_string(&new_file)?, "x");
    Ok(())
}

#[test]
fn real_bubblewrap_cache_grant_hides_the_token_and_writes_the_cache() -> anyhow::Result<()> {
    let fixture = Fixture::new();
    let cache = fixture.home.join(".cache");
    let token = cache.join("huggingface/token");
    std::fs::create_dir_all(token.parent().expect("token parent"))?;
    std::fs::write(&token, "token")?;
    let bwrap = match confine::resolve_bwrap(None) {
        Ok(path) => path,
        Err(error) if !require_bwrap() => {
            eprintln!("skipping cache grant bwrap test: {error:#}");
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let plan = confine::plan_with_path(
        &ConfineRequest {
            grants: vec!["cache".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
        None,
    )?
    .with_bwrap(bwrap);
    let new_file = cache.join("new");
    let script = format!(
        "set -eu; test ! -s '{}'; printf x > '{}'",
        token.display(),
        new_file.display()
    );
    let wrapped = plan.wrap(vec!["sh".into(), "-c".into(), script])?;
    let status = Command::new(&wrapped[0])
        .args(&wrapped[1..])
        .status()
        .context("run cache grant bwrap probe")?;
    assert_eq!(status.code(), Some(0));
    assert_eq!(std::fs::read_to_string(&token)?, "token");
    assert_eq!(std::fs::read_to_string(&new_file)?, "x");
    Ok(())
}

#[test]
fn grant_create_ancestor_does_not_release_hidden_path() {
    let fixture = Fixture::new();
    std::fs::create_dir_all(fixture.home.join(".cargo")).expect("cargo directory");
    symlink(
        fixture.home.join(".ssh"),
        fixture.home.join(".cargo/registry"),
    )
    .expect("registry symlink");
    let host = fixture.host();
    let request = ConfineRequest {
        grants: vec!["cargo".into()],
        ..ConfineRequest::default()
    };
    let preflight = confine::ensure_grant_dirs(&request, Harness::Generic, &host, true);
    assert!(preflight.is_empty());
    let plan = confine::plan_with_preflight(
        &request,
        Harness::Generic,
        &fixture.cwd,
        &host,
        None,
        preflight,
    )
    .expect("grant path plan");
    let ssh = std::fs::canonicalize(fixture.home.join(".ssh")).expect("ssh path");
    assert!(!plan.writable.contains(&ssh));
    assert!(plan.hidden_paths().contains(&ssh));
}

#[test]
fn grant_create_symlink_ancestor_is_not_created() {
    let fixture = Fixture::new();
    let target = fixture._root.path().join("outside");
    std::fs::create_dir_all(&target).expect("outside directory");
    symlink(&target, fixture.home.join(".cargo")).expect("cargo symlink");
    let host = fixture.host();
    let request = ConfineRequest {
        grants: vec!["cargo".into()],
        ..ConfineRequest::default()
    };
    let preflight = confine::ensure_grant_dirs(&request, Harness::Generic, &host, true);
    let error = match confine::plan_with_preflight(
        &request,
        Harness::Generic,
        &fixture.cwd,
        &host,
        None,
        preflight,
    ) {
        Ok(_) => panic!("symlinked cargo grant must fail"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("passes through the symlink"));
}

#[test]
fn write_slash_and_inside_state_dir_stay_refused() {
    let fixture = Fixture::new();
    let nested = fixture.state.join("nested");
    std::fs::create_dir_all(&nested).expect("nested state path");
    for path in [
        "/",
        fixture.state.to_str().expect("state"),
        nested.to_str().expect("nested"),
        fixture.runtime.to_str().expect("runtime"),
    ] {
        assert!(
            confine::plan(
                &ConfineRequest {
                    write: vec![path.into()],
                    ..ConfineRequest::default()
                },
                Harness::Generic,
                &fixture.cwd,
                &fixture.host(),
            )
            .is_err(),
            "write must be refused: {path}"
        );
    }
}

#[test]
fn workspace_in_home_is_still_refused() {
    let fixture = Fixture::new();
    let error = match confine::plan(
        &ConfineRequest {
            write: vec!["~".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.home,
        &fixture.host(),
    ) {
        Ok(_) => panic!("home cwd must remain refused"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("writable path $HOME is not allowed")
    );

    let outer = tempfile::tempdir().expect("outer root");
    let home = outer.path().join("home");
    std::fs::create_dir_all(&home).expect("outer home");
    let protected = tempfile::tempdir().expect("protected root");
    let state = protected.path().join("state");
    let runtime = protected.path().join("runtime");
    std::fs::create_dir_all(&state).expect("outer state");
    std::fs::create_dir_all(&runtime).expect("outer runtime");
    let empty = state.join("confine-empty");
    std::fs::write(&empty, []).expect("outer empty");
    let host = Host {
        home,
        state_dir: state,
        runtime_dir: runtime,
        empty_file: empty,
    };
    let error = match confine::plan(
        &ConfineRequest {
            write: vec!["~".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        outer.path(),
        &host,
    ) {
        Ok(_) => panic!("ancestor cwd must remain refused"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("writable path $HOME is not allowed")
    );
}

#[test]
fn find_executable_ignores_relative_and_empty_entries() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let expected = std::fs::canonicalize(manifest.join("scripts/prepush-fast.sh"))
        .expect("tracked executable");
    assert!(
        confine::find_executable("prepush-fast.sh", OsStr::new("scripts")).is_none(),
        "relative PATH entry must be ignored"
    );
    assert!(
        confine::find_executable("scripts/prepush-fast.sh", OsStr::new("")).is_none(),
        "empty PATH entry must be ignored"
    );
    assert!(
        confine::find_executable("scripts/prepush-fast.sh", OsStr::new(":")).is_none(),
        "empty PATH entries must be ignored"
    );
    let absolute_path = OsString::from(format!("{}:/usr/bin", manifest.join("scripts").display()));
    assert_eq!(
        confine::find_executable("prepush-fast.sh", &absolute_path),
        Some(expected)
    );
}

#[test]
fn resolve_bwrap_refuses_a_relative_file_path() {
    let error = confine::resolve_bwrap(Some(OsStr::new("scripts/prepush-fast.sh")))
        .expect_err("relative file path must not select bwrap");
    assert_eq!(error.to_string(), "bubblewrap was not found in PATH");
    let error =
        confine::resolve_bwrap(Some(OsStr::new("scripts"))).expect_err("directory is not bwrap");
    assert_eq!(error.to_string(), "bubblewrap was not found in PATH");
}

#[test]
fn harness_config_default_replaces_data_overlays() {
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
    let wrapped = claude_plan
        .wrap(vec!["true".into()])
        .expect("Claude wrapper");
    let root = claude_root.to_string_lossy();
    assert!(
        wrapped
            .windows(3)
            .any(|window| window[0] == "--bind" && window[1] == root && window[2] == root)
    );

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
}

#[test]
fn overlay_is_not_planned_over_a_hidden_written_or_unhidden_parent() {
    let fixture = Fixture::new();
    populate_claude_overlay(&fixture, true);
    let parent = std::fs::canonicalize(fixture.home.join(".claude")).expect("Claude root");
    for (request, operation) in [
        (
            ConfineRequest {
                hide: vec!["~/.claude".into()],
                ..ConfineRequest::default()
            },
            "--tmpfs",
        ),
        (
            ConfineRequest {
                write: vec!["~/.claude".into()],
                ..ConfineRequest::default()
            },
            "--bind",
        ),
        (
            ConfineRequest {
                unhide: vec!["~/.claude".into()],
                ..ConfineRequest::default()
            },
            "--ro-bind",
        ),
    ] {
        let plan = confine::plan(&request, Harness::Claude, &fixture.cwd, &fixture.host())
            .expect("Claude exclusion plan");
        assert!(!plan.overlay_parents().contains(&parent));
        let wrapped = plan.wrap(vec!["true".into()]).expect("operator wrapper");
        let parent = parent.to_string_lossy();
        let retained = if operation == "--tmpfs" {
            wrapped
                .windows(2)
                .any(|window| window[0] == operation && window[1] == parent)
        } else {
            wrapped
                .windows(3)
                .any(|window| window[0] == operation && window[1] == parent && window[2] == parent)
        };
        assert!(retained, "operator mount was lost: {request:?}");
    }

    let omp = Fixture::new();
    populate_omp_overlay(&omp);
    let omp_root = std::fs::canonicalize(omp.home.join(".omp")).expect("OMP root");
    let plan = confine::plan(
        &ConfineRequest {
            hide: vec!["~/.omp".into()],
            ..ConfineRequest::default()
        },
        Harness::Omp,
        &omp.cwd,
        &omp.host(),
    )
    .expect("OMP exclusion plan");
    assert!(!plan.overlay_parents().contains(&omp_root));
    let wrapped = plan.wrap(vec!["true".into()]).expect("OMP hidden wrapper");
    let omp_root_text = omp_root.to_string_lossy();
    assert!(
        wrapped
            .windows(2)
            .any(|window| window[0] == "--tmpfs" && window[1] == omp_root_text)
    );
}

#[test]
fn harness_config_default_saves_missing_data_files() {
    let claude = Fixture::new();
    populate_claude_overlay(&claude, false);
    let plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &claude.cwd,
        &claude.host(),
    )
    .expect("Claude default plan");
    assert!(plan.unsaved_files().is_empty());
    assert!(plan.overlay_parents().is_empty());

    let omp = Fixture::new();
    populate_omp_overlay(&omp);
    let plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Omp,
        &omp.cwd,
        &omp.host(),
    )
    .expect("OMP default plan");
    assert!(plan.unsaved_files().is_empty());
    assert!(plan.overlay_parents().is_empty());
}

#[test]
fn without_overlays_keeps_the_default_harness_root_bind() {
    let fixture = Fixture::new();
    populate_claude_overlay(&fixture, true);
    let plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("Claude plan");
    let without = plan.without_overlays("x");
    assert!(without.overlay_parents().is_empty());
    assert!(
        without
            .writable
            .contains(&std::fs::canonicalize(fixture.home.join(".claude")).expect("Claude root"))
    );
    assert!(
        confine::notice(&without).contains("harness login cannot be renewed in this session: x")
    );
}

#[test]
fn harness_data_files_are_saved_through_default_root() -> anyhow::Result<()> {
    let root = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR"))?;
    let home = root.path().join("home");
    let state = root.path().join("state");
    let runtime = root.path().join("runtime");
    let cwd = root.path().join("workspace");
    let claude_root = home.join(".claude");
    let credentials = claude_root.join(".credentials.json");
    let settings = claude_root.join("settings.json");
    let plugins = claude_root.join("plugins");
    let hooks_target = root.path().join("external-hooks");
    for path in [
        &home,
        &state,
        &runtime,
        &cwd,
        &claude_root,
        &plugins,
        &hooks_target,
    ] {
        std::fs::create_dir_all(path)?;
    }
    std::fs::write(&credentials, "old")?;
    std::fs::write(&settings, "settings")?;
    std::os::unix::fs::symlink(&hooks_target, claude_root.join("hooks"))?;
    let empty_file = state.join("confine-empty");
    confine::ensure_data_dirs(Harness::Claude, &home)?;
    std::fs::write(&empty_file, [])?;
    let host = Host {
        home,
        state_dir: state,
        runtime_dir: runtime,
        empty_file,
    };
    let bwrap = match confine::resolve_bwrap(None) {
        Ok(path) => path,
        Err(error) if !require_bwrap() => {
            eprintln!("skipping overlay write test: {error:#}");
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let plan = confine::plan(&ConfineRequest::default(), Harness::Claude, &cwd, &host)?;
    let script = format!(
        "set -u; credentials='{}'; root='{}'; settings='{}'; tmp=\"$credentials.tmp.1\"; \
         printf renewed > \"$tmp\"; mv \"$tmp\" \"$credentials\"; \
         printf new > \"$root/new-file\"; printf changed > \"$settings\"",
        credentials.display(),
        claude_root.display(),
        settings.display()
    );
    let wrapped = plan
        .with_bwrap(bwrap)
        .wrap(vec!["sh".into(), "-c".into(), script])?;
    let status = Command::new(&wrapped[0])
        .args(&wrapped[1..])
        .status()
        .context("run real bubblewrap overlay write probe")?;
    assert_eq!(status.code(), Some(0));
    assert_eq!(std::fs::read_to_string(&credentials)?, "renewed");
    assert!(root.path().join("home/.claude/new-file").exists());
    assert!(
        !root
            .path()
            .join("home/.claude/.credentials.json.tmp.1")
            .exists()
    );
    assert_eq!(std::fs::read_to_string(&settings)?, "changed");
    Ok(())
}

#[test]
fn harness_config_grant_opens_the_own_directory() {
    let claude = Fixture::new();
    populate_claude_overlay(&claude, true);
    let claude_root = std::fs::canonicalize(claude.home.join(".claude")).expect("Claude root");
    std::fs::create_dir_all(claude_root.join("ide")).expect("Claude IDE directory");
    let path_dir = claude_root.join("local/bin");
    std::fs::create_dir_all(&path_dir).expect("PATH directory");
    let path_var = OsString::from(format!("{}:/usr/bin", path_dir.display()));
    let claude_plan = confine::plan_with_path(
        &ConfineRequest {
            grants: vec!["harness-config".into()],
            unhide: vec!["~/.claude/settings.json".into()],
            ..ConfineRequest::default()
        },
        Harness::Claude,
        &claude.cwd,
        &claude.host(),
        Some(path_var),
    )
    .expect("Claude harness-config plan");
    assert!(claude_plan.writable.contains(&claude_root));
    assert!(claude_plan.overlay_parents().is_empty());
    let wrapped = claude_plan
        .wrap(vec!["true".into()])
        .expect("Claude wrapper");
    let root_text = claude_root.to_string_lossy();
    let root_index = wrapped
        .windows(3)
        .position(|window| {
            window[0] == "--bind" && window[1] == root_text && window[2] == root_text
        })
        .expect("Claude root writable bind");
    let path_dir = std::fs::canonicalize(path_dir).expect("PATH canonical");
    assert!(claude_plan.protected.contains(&path_dir));
    let path_dir_text = path_dir.to_string_lossy();
    let path_dir_index = wrapped
        .windows(3)
        .position(|window| {
            window[0] == "--ro-bind" && window[1] == path_dir_text && window[2] == path_dir_text
        })
        .expect("nested 3bo protected launch input");
    assert!(root_index < path_dir_index);
    let ide = claude_root.join("ide");
    let ide_text = ide.to_string_lossy();
    let ide_index = wrapped
        .windows(2)
        .position(|window| window[0] == "--tmpfs" && window[1] == ide_text)
        .expect("Claude IDE hide");
    let settings = claude_root.join("settings.json");
    let settings_text = settings.to_string_lossy();
    let settings_index = wrapped
        .windows(3)
        .position(|window| {
            window[0] == "--ro-bind" && window[1] == settings_text && window[2] == settings_text
        })
        .expect("Claude read-only settings bind");
    assert!(root_index < ide_index);
    assert!(root_index < settings_index);
    for path in [
        claude_root.join("projects"),
        claude_root.join("history.jsonl"),
        claude_root.join(".credentials.json"),
    ] {
        assert!(
            !wrapped
                .iter()
                .any(|argument| argument == path.to_string_lossy().as_ref()),
            "harness data path must be covered by the root bind: {}",
            path.display()
        );
    }
    assert!(confine::notice(&claude_plan).contains(&format!(
        "{}: harness config, writable and persistent (grant harness-config).",
        claude_root.display()
    )));

    let plain = confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &claude.cwd,
        &claude.host(),
    )
    .expect("Claude plain plan");
    assert!(confine::notice(&plain).contains("harness config, writable and persistent"));
    assert!(plain.writable.contains(&claude_root));
    assert!(plain.overlay_parents().is_empty());

    let omp = Fixture::new();
    populate_omp_overlay(&omp);
    let omp_root = std::fs::canonicalize(omp.home.join(".omp")).expect("OMP root");
    let omp_plan = confine::plan(
        &ConfineRequest {
            grants: vec!["harness-config".into()],
            ..ConfineRequest::default()
        },
        Harness::Omp,
        &omp.cwd,
        &omp.host(),
    )
    .expect("OMP harness-config plan");
    assert!(omp_plan.writable.contains(&omp_root));
    assert!(confine::notice(&omp_plan).contains(&format!(
        "{}: harness config, writable and persistent (grant harness-config).",
        omp_root.display()
    )));
    assert!(omp_plan.overlay_parents().is_empty());
    let wrapped = omp_plan.wrap(vec!["true".into()]).expect("OMP wrapper");
    let root_text = omp_root.to_string_lossy();
    assert!(wrapped.windows(3).any(|window| {
        window[0] == "--bind" && window[1] == root_text && window[2] == root_text
    }));
    for path in [
        omp_root.join("agent/a.db"),
        omp_root.join("stats.db"),
        omp_root.join("config.yml"),
    ] {
        assert!(
            !wrapped
                .iter()
                .any(|argument| argument == path.to_string_lossy().as_ref()),
            "OMP harness data path must be covered by the root bind: {}",
            path.display()
        );
    }
    assert!(
        !wrapped
            .iter()
            .any(|argument| { argument == omp_root.join("agent").to_string_lossy().as_ref() })
    );
}

#[test]
fn harness_config_is_default_for_claude_and_omp() {
    let claude = Fixture::new();
    populate_claude_overlay(&claude, true);
    let claude_root = std::fs::canonicalize(claude.home.join(".claude")).expect("Claude root");
    let ide = claude_root.join("ide");
    std::fs::create_dir_all(&ide).expect("Claude IDE directory");
    let implicit = confine::plan_with_path(
        &ConfineRequest {
            ..ConfineRequest::default()
        },
        Harness::Claude,
        &claude.cwd,
        &claude.host(),
        None,
    )
    .expect("implicit Claude harness config");
    assert!(implicit.writable.contains(&claude_root));
    assert!(implicit.grants.is_empty());
    assert!(confine::notice(&implicit).contains(&format!(
        "{}: harness config, writable and persistent (default for this harness).",
        claude_root.display()
    )));

    let no_default_hides = confine::plan_with_path(
        &ConfineRequest {
            no_default_hides: true,
            ..ConfineRequest::default()
        },
        Harness::Claude,
        &claude.cwd,
        &claude.host(),
        None,
    )
    .expect("implicit Claude harness config without default hides");
    assert!(no_default_hides.writable.contains(&claude_root));
    assert!(!no_default_hides.hidden_paths().contains(&ide));

    let opt_out = confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &claude.cwd,
        &claude.host(),
    )
    .expect("Claude opt-out plan");
    assert!(opt_out.writable.contains(&claude_root));

    let omp = Fixture::new();
    populate_omp_overlay(&omp);
    let omp_root = std::fs::canonicalize(omp.home.join(".omp")).expect("OMP root");
    let omp_plan = confine::plan_with_path(
        &ConfineRequest {
            ..ConfineRequest::default()
        },
        Harness::Omp,
        &omp.cwd,
        &omp.host(),
        None,
    )
    .expect("implicit OMP harness config");
    assert!(omp_plan.writable.contains(&omp_root));
    assert!(omp_plan.grants.is_empty());
    assert!(confine::notice(&omp_plan).contains(&format!(
        "{}: harness config, writable and persistent (default for this harness).",
        omp_root.display()
    )));

    let generic = Fixture::new();
    confine::plan_with_path(
        &ConfineRequest {
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &generic.cwd,
        &generic.host(),
        None,
    )
    .expect("Generic confinement plan");
}

#[test]
fn operator_harness_root_hide_or_unhide_skips_implicit_grant() {
    let fixture = Fixture::new();
    populate_claude_overlay(&fixture, true);
    let root = std::fs::canonicalize(fixture.home.join(".claude")).expect("Claude root");
    for (field, entry) in [("hide", "~/.claude"), ("unhide", "~/.claude")] {
        let request = if field == "hide" {
            ConfineRequest {
                hide: vec![entry.into()],
                ..ConfineRequest::default()
            }
        } else {
            ConfineRequest {
                unhide: vec![entry.into()],
                ..ConfineRequest::default()
            }
        };
        let plan = confine::plan(&request, Harness::Claude, &fixture.cwd, &fixture.host())
            .expect("operator root override");
        assert!(!plan.writable.contains(&root));
        assert!(plan.grants.is_empty());
        assert!(plan.unresolved.iter().any(|item| {
            item.option == "harness-config"
                && item.path == root
                && item.reason == "operator hide or unhide of the harness root"
        }));
        let notice = confine::notice(&plan);
        assert!(!notice.contains("harness config"));
        assert!(!notice.contains("unresolved"));
    }
}

#[test]
fn harness_config_grant_refuses_generic_hide_and_unhide() {
    let generic = Fixture::new();
    let error = match confine::plan(
        &ConfineRequest {
            grants: vec!["harness-config".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &generic.cwd,
        &generic.host(),
    ) {
        Ok(_) => panic!("Generic harness-config grant was accepted"),
        Err(error) => error,
    };
    assert_eq!(
        error.to_string(),
        "confinement grant \"harness-config\" needs a Claude, OMP, or Codex session"
    );

    let claude = Fixture::new();
    populate_claude_overlay(&claude, true);
    for (operation, expected) in [
        (
            ConfineRequest {
                grants: vec!["harness-config".into()],
                hide: vec!["~/.claude".into()],
                ..ConfineRequest::default()
            },
            "confinement grant \"harness-config\" conflicts with the hide of",
        ),
        (
            ConfineRequest {
                grants: vec!["harness-config".into()],
                unhide: vec!["~/.claude".into()],
                ..ConfineRequest::default()
            },
            "confinement grant \"harness-config\" conflicts with the unhide of",
        ),
    ] {
        let error = match confine::plan(&operation, Harness::Claude, &claude.cwd, &claude.host()) {
            Ok(_) => panic!("harness-config conflict was accepted"),
            Err(error) => error,
        };
        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn harness_config_grant_persists_regular_writes() -> anyhow::Result<()> {
    let root = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR"))?;
    let home = root.path().join("home");
    let state = root.path().join("state");
    let runtime = root.path().join("runtime");
    let cwd = root.path().join("workspace");
    let claude_root = home.join(".claude");
    let credentials = claude_root.join(".credentials.json");
    let settings = claude_root.join("settings.json");
    for path in [&home, &state, &runtime, &cwd, &claude_root] {
        std::fs::create_dir_all(path)?;
    }
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
        std::fs::create_dir_all(claude_root.join(name))?;
    }
    std::fs::write(&credentials, "old")?;
    std::fs::write(&settings, "settings")?;
    confine::ensure_data_dirs(Harness::Claude, &home)?;
    let empty_file = state.join("confine-empty");
    std::fs::write(&empty_file, [])?;
    let host = Host {
        home,
        state_dir: state,
        runtime_dir: runtime,
        empty_file,
    };
    let bwrap = match confine::resolve_bwrap(None) {
        Ok(path) => path,
        Err(error) if !require_bwrap() => {
            eprintln!("skipping harness-config write test: {error:#}");
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let plain = confine::plan(&ConfineRequest::default(), Harness::Claude, &cwd, &host)?;

    let granted = confine::plan(
        &ConfineRequest {
            grants: vec!["harness-config".into()],
            ..ConfineRequest::default()
        },
        Harness::Claude,
        &cwd,
        &host,
    )?;
    let script = format!(
        "set -u; settings='{}'; credentials='{}'; tmp=\"$credentials.tmp.1\"; \
         printf changed > \"$settings\"; printf renewed > \"$tmp\"; mv \"$tmp\" \"$credentials\"",
        settings.display(),
        credentials.display()
    );
    let wrapped = granted
        .with_bwrap(bwrap.clone())
        .wrap(vec!["sh".into(), "-c".into(), script])?;
    let status = Command::new(&wrapped[0])
        .args(&wrapped[1..])
        .status()
        .context("run granted harness-config write probe")?;
    assert_eq!(status.code(), Some(0));
    assert_eq!(std::fs::read_to_string(&settings)?, "changed");
    assert_eq!(std::fs::read_to_string(&credentials)?, "renewed");
    assert!(
        !credentials
            .with_file_name(".credentials.json.tmp.1")
            .exists()
    );

    std::fs::write(&settings, "settings")?;
    let script = format!(
        "if printf changed > '{}'; then exit 1; else exit 0; fi",
        settings.display()
    );
    let wrapped = plain
        .with_bwrap(bwrap)
        .wrap(vec!["sh".into(), "-c".into(), script])?;
    let status = Command::new(&wrapped[0])
        .args(&wrapped[1..])
        .status()
        .context("run ungranted harness-config write probe")?;
    assert_eq!(status.code(), Some(1));
    assert_eq!(std::fs::read_to_string(&settings)?, "changed");
    Ok(())
}

fn populate_claude_overlay(fixture: &Fixture, credential: bool) {
    let root = fixture.home.join(".claude");
    std::fs::create_dir_all(&root).expect("Claude root");
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
        "plugins",
    ] {
        std::fs::create_dir_all(root.join(name)).expect("Claude data directory");
    }
    std::fs::write(root.join("history.jsonl"), "history").expect("history");
    std::fs::write(root.join("stats-cache.json"), "stats").expect("stats");
    if credential {
        std::fs::write(root.join(".credentials.json"), "credentials").expect("credentials");
    }
    std::fs::write(root.join("settings.json"), "settings").expect("settings");
    let target = fixture._root.path().join("external-hooks");
    std::fs::create_dir_all(&target).expect("hooks target");
    symlink(&target, root.join("hooks")).expect("hooks symlink");
}

fn populate_omp_overlay(fixture: &Fixture) {
    let root = fixture.home.join(".omp");
    let agent = root.join("agent");
    std::fs::create_dir_all(agent.join("sessions")).expect("OMP sessions");
    std::fs::create_dir_all(root.join("extensions")).expect("OMP extensions");
    std::fs::write(agent.join("a.db"), "agent").expect("agent db");
    std::fs::write(root.join("stats.db"), "stats").expect("stats db");
    std::fs::write(root.join("config.yml"), "config").expect("OMP config");
}
