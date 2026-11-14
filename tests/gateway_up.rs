#![allow(clippy::expect_used, clippy::unwrap_used)]
mod common;

use tokio::io::{AsyncBufReadExt, BufReader as AsyncBufReader};

use std::fs;
use std::os::unix::fs::{FileTypeExt, PermissionsExt, symlink};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use a2amx::gateway_launch::{LaunchPlan, sandbox_argv};

#[test]
fn sandbox_argv_matches_worked_vector_with_external_resolv_target() {
    let layout = LaunchPlan {
        bwrap: PathBuf::from("/usr/bin/bwrap"),
        layout: a2amx::layout::Layout::new(PathBuf::from("/home/me/state")),
        exe: PathBuf::from("/opt/a2amx"),
        rust_log: Some("debug".into()),
        resolv_target: Some(PathBuf::from("/mnt/wsl/resolv.conf")),
        die_with_parent: true,
    };
    let argv = sandbox_argv(&layout, &[]);
    let joined = argv.join("\n");
    for path in [
        "run/link",
        "config/gateway.toml",
        "config/secrets/discord.token",
        "state/gateway",
    ] {
        assert!(joined.contains(path), "missing bind {path}");
    }
    assert!(joined.contains("--chdir"));
    assert!(joined.contains("--remount-ro"));
}

#[test]
fn sandbox_argv_matches_worked_vector_without_external_resolv_target() {
    let layout = LaunchPlan {
        bwrap: PathBuf::from("/usr/bin/bwrap"),
        layout: a2amx::layout::Layout::new(PathBuf::from("/home/me/state")),
        exe: PathBuf::from("/opt/a2amx"),
        rust_log: None,
        resolv_target: None,
        die_with_parent: true,
    };
    let argv = sandbox_argv(&layout, &[]);
    let bind_paths: Vec<_> = argv
        .windows(4)
        .filter(|w| w[0] == "--ro-bind" || w[0] == "--bind")
        .collect();
    let layout_bind_paths: Vec<_> = bind_paths
        .iter()
        .filter(|w| {
            w[1].contains("/run/link")
                || w[1].contains("/config/")
                || w[1].contains("/state/gateway")
        })
        .collect();
    assert_eq!(layout_bind_paths.len(), 4);
    assert!(
        bind_paths
            .iter()
            .any(|w| w[1].ends_with("run/link") && w[2].ends_with("run/link"))
    );
    assert!(bind_paths.iter().any(|w| w[1].ends_with("config/gateway.toml") && w[2].ends_with("config/gateway.toml")));
    assert!(
        bind_paths
            .iter()
            .any(|w| w[1].ends_with("config/secrets/discord.token")
                && w[2].ends_with("config/secrets/discord.token"))
    );
    assert!(
        bind_paths
            .iter()
            .any(|w| w[1].ends_with("state/gateway") && w[2].ends_with("state/gateway"))
    );
    for path in [
        "run/link",
        "config/gateway.toml",
        "config/secrets/discord.token",
    ] {
        assert!(
            bind_paths
                .iter()
                .any(|w| w[0] == "--ro-bind" && w[1].ends_with(path) && w[2].ends_with(path)),
            "expected read-only bind for {path}"
        );
    }
    assert!(bind_paths.iter().any(|w| w[0] == "--bind"
        && w[1].ends_with("state/gateway")
        && w[2].ends_with("state/gateway")));
    for path in [
        "/home/me/state",
        "/home/me/state/config",
        "/home/me/state/state",
        "/home/me/state/log",
    ] {
        assert!(
            !bind_paths.iter().any(|w| w[1] == path || w[2] == path),
            "unexpected broad bind {path}"
        );
    }
    assert!(argv.iter().any(|arg| arg == "--remount-ro"));
    assert_eq!(
        argv.windows(2).find(|w| w[0] == "--chdir").unwrap()[1],
        "/home/me/state/state/gateway"
    );
}

#[test]
fn sandbox_argv_background_omits_die_with_parent() {
    let layout = LaunchPlan {
        bwrap: PathBuf::from("/usr/bin/bwrap"),
        layout: a2amx::layout::Layout::new(PathBuf::from("/home/me/state")),
        exe: PathBuf::from("/opt/a2amx"),
        rust_log: None,
        resolv_target: None,
        die_with_parent: false,
    };
    let argv = sandbox_argv(
        &layout,
        &[
            "/opt/a2amx".into(),
            "--home".into(),
            "/home/me/state".into(),
            "gateway".into(),
            "run".into(),
        ],
    );
    assert_eq!(
        &argv[0..5],
        [
            "/usr/bin/bwrap",
            "--unshare-user",
            "--unshare-ipc",
            "--unshare-pid",
            "--tmpfs",
        ]
    );
}
fn bwrap_for_test() -> Option<PathBuf> {
    let bwrap = match a2amx::confine::resolve_bwrap(None) {
        Ok(path) => path,
        Err(error) => {
            if std::env::var_os("A2AMX_REQUIRE_BWRAP").is_some() {
                panic!("bubblewrap is required for this test: {error}");
            }
            println!("skipping real-bubblewrap test: {error}");
            return None;
        }
    };
    a2amx::confine::preflight(&bwrap).expect("bubblewrap preflight");
    Some(bwrap)
}

fn prepare_gateway_files(home: &Path) {
    let layout = a2amx::layout::Layout::new(home.to_owned());
    layout.ensure().unwrap();
    for (path, contents) in [
        (layout.discord_token(), "bot-token\n"),
        (layout.gateway_config(), "guild_id = \"123\"\n"),
        (layout.pairing(), r#"{"guild_id":"123","user_id":"456"}"#),
    ] {
        fs::write(&path, contents).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    }
}

fn prepare_state(home: &Path) {
    let layout = a2amx::layout::Layout::new(home.to_owned());
    layout.ensure().unwrap();
    prepare_gateway_files(home);
    for (path, contents) in [
        (layout.admin_token(), "admin-secret\n"),
        (layout.link_addr(), "127.0.0.1:1\n"),
        (layout.link_gateway_token(), "gateway-secret\n"),
    ] {
        fs::write(&path, contents).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    }
}

fn shell_layout(home: &Path, bwrap: &Path, rust_log: Option<&str>) -> (LaunchPlan, String) {
    let exe = fs::canonicalize("/bin/sh").unwrap();
    (
        LaunchPlan {
            bwrap: bwrap.to_owned(),
            layout: a2amx::layout::Layout::new(fs::canonicalize(home).unwrap()),
            exe,
            rust_log: rust_log.map(str::to_owned),
            resolv_target: None,
            die_with_parent: true,
        },
        fs::canonicalize("/bin/sh")
            .unwrap()
            .to_string_lossy()
            .into_owned(),
    )
}

fn run_sandbox(layout: &LaunchPlan, inner: &[String], env: &[(&str, &str)]) -> Output {
    let argv = sandbox_argv(layout, inner);
    let mut command = Command::new(&argv[0]);
    command.args(&argv[1..]);
    for (name, value) in env {
        command.env(name, value);
    }
    command.output().unwrap()
}

fn run_gateway_up(home: &Path, pair: bool, path: &str) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_a2amx"));
    command.args(["--home"]).arg(home).args(["gateway", "up"]);
    if pair {
        command.arg("--pair");
    }
    command
        .env("PATH", path)
        .env("HOME", "/tmp/fake-home")
        .env("A2AMX_TOKEN", "fake-token")
        .output()
        .unwrap()
}
fn assert_error_contains(output: Output, needle: &str) {
    assert!(
        !output.status.success(),
        "command unexpectedly succeeded: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(needle),
        "stderr did not contain {needle:?}: stdout={} stderr={stderr}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn sandbox_hides_state_host_and_limits_writes_to_gateway() {
    let Some(bwrap) = bwrap_for_test() else {
        return;
    };
    let parent = std::env::var_os("HOME")
        .map(PathBuf::from)
        .expect("HOME is set for the real-bubblewrap state probe");
    let home = tempfile::tempdir_in(parent).unwrap();
    prepare_state(home.path());
    let (layout, shell) = shell_layout(home.path(), &bwrap, None);
    let state = layout.layout.root().to_string_lossy().into_owned();
    let script = r#"set -eu
test -r "$1/config/gateway.toml"
test -r "$1/config/secrets/discord.token"
set +e
(printf x > "$1/run/link/x") 2>/dev/null
link_status=$?
set -e
test "$link_status" -ne 0
printf x > "$1/state/gateway/x"
test "$(cat "$1/state/gateway/x")" = x
"#;
    let inner = vec![shell, "-c".into(), script.into(), "probe".into(), state];
    let output = run_sandbox(&layout, &inner, &[]);
    assert!(
        output.status.success(),
        "sandbox probe failed status={:?}: {}{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(home.path().join("state/gateway/x").is_file());
    assert!(!home.path().join("run/link/x").exists());
    assert!(!home.path().join("marker").exists());
}

#[test]
fn sandbox_blocks_same_user_unix_sockets_and_host_runtime_bus() {
    let Some(bwrap) = bwrap_for_test() else {
        return;
    };
    let python = if Path::new("/usr/bin/python3").exists() {
        fs::canonicalize("/usr/bin/python3").unwrap()
    } else if Path::new("/usr/bin/perl").exists() {
        fs::canonicalize("/usr/bin/perl").unwrap()
    } else {
        println!("skipping Unix socket test: neither python3 nor perl exists");
        return;
    };
    let home = tempfile::tempdir().unwrap();
    prepare_state(home.path());
    let outside = home.path().join("outside.sock");
    let listener = UnixListener::bind(&outside).unwrap();
    let mut sockets = vec![outside];
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        let bus = PathBuf::from(runtime).join("bus");
        if fs::metadata(&bus)
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)
        {
            sockets.push(bus);
        }
    }
    let script = if python.ends_with("perl") {
        r#"use IO::Socket::UNIX;
for my $path (@ARGV) {
  my $socket = IO::Socket::UNIX->new(Peer => $path);
  exit 1 if $socket;
}
exit 0
"#
    } else {
        r#"import socket
import sys
for path in sys.argv[1:]:
    try:
        socket.socket(socket.AF_UNIX).connect(path)
    except OSError:
        pass
    else:
        raise SystemExit(1)
"#
    };
    let state = fs::canonicalize(home.path()).unwrap();
    let layout = LaunchPlan {
        bwrap: bwrap.clone(),
        layout: a2amx::layout::Layout::new(state.clone()),
        exe: python.clone(),
        rust_log: None,
        resolv_target: None,
        die_with_parent: true,
    };
    let mut inner = vec![
        python.to_string_lossy().into_owned(),
        "-c".into(),
        script.into(),
    ];
    inner.extend(
        sockets
            .iter()
            .map(|path| path.to_string_lossy().into_owned()),
    );
    let output = run_sandbox(&layout, &inner, &[]);
    drop(listener);
    assert!(
        output.status.success(),
        "socket probe unexpectedly connected: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn sandbox_clears_environment_except_path_pwd_and_rust_log() {
    let Some(bwrap) = bwrap_for_test() else {
        return;
    };
    let home = tempfile::tempdir().unwrap();
    prepare_state(home.path());
    let env_exe = fs::canonicalize("/usr/bin/env").unwrap();
    let state = fs::canonicalize(home.path()).unwrap();
    let layout = LaunchPlan {
        bwrap,
        layout: a2amx::layout::Layout::new(state.clone()),
        exe: env_exe.clone(),
        rust_log: Some("trace".into()),
        resolv_target: None,
        die_with_parent: true,
    };
    let inner = vec![env_exe.to_string_lossy().into_owned()];
    let output = run_sandbox(
        &layout,
        &inner,
        &[
            ("HOME", "/tmp/fake-home"),
            ("A2AMX_TOKEN", "fake-token"),
            ("A2AMX_BIN", "fake-bin"),
        ],
    );
    assert!(output.status.success(), "env failed: {:?}", output);
    let mut lines = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    lines.sort();
    let mut expected = vec![
        "PATH=/usr/local/bin:/usr/bin:/bin".to_owned(),
        format!("PWD={}", state.join("state/gateway").display()),
        "RUST_LOG=trace".to_owned(),
    ];
    expected.sort();
    assert_eq!(lines, expected);
}

#[test]
fn sandbox_keeps_inner_arguments_after_separator() {
    let layout = LaunchPlan {
        bwrap: PathBuf::from("/usr/bin/bwrap"),
        layout: a2amx::layout::Layout::new(PathBuf::from("/tmp/- state 'quoted'")),
        exe: PathBuf::from("/opt/a2amx"),
        rust_log: None,
        resolv_target: None,
        die_with_parent: true,
    };
    let inner = vec![
        "/opt/a2amx".into(),
        "--home".into(),
        "/tmp/- state 'quoted'".into(),
        "gateway".into(),
        "run".into(),
        "--ro-bind".into(),
        "value with spaces".into(),
    ];
    let argv = sandbox_argv(&layout, &inner);
    let separator = argv.iter().position(|argument| argument == "--").unwrap();
    assert_eq!(&argv[separator + 1..], inner);
}

#[test]
fn gateway_token_and_up_create_gateway_template_once() {
    let home = tempfile::tempdir().unwrap();
    let layout = a2amx::layout::Layout::new(home.path().to_path_buf());
    layout.ensure().unwrap();
    let mut token = Command::new(env!("CARGO_BIN_EXE_a2amx"))
        .args(["--home"])
        .arg(home.path())
        .args(["gateway", "token"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write as _;
    token
        .stdin
        .take()
        .unwrap()
        .write_all(b"bot-token\n")
        .unwrap();
    let output = token.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let template = fs::read_to_string(layout.gateway_config()).unwrap();
    assert!(template.contains("# guild_id ="));
    assert_eq!(
        fs::metadata(layout.gateway_config())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let original = template.clone();
    let output = run_gateway_up(home.path(), false, "");
    assert_error_contains(output.clone(), "guild_id");
    assert_error_contains(output, "not set");
    assert_eq!(
        fs::read_to_string(layout.gateway_config()).unwrap(),
        original
    );
}
#[test]
fn empty_gateway_token_does_not_create_template() {
    let home = tempfile::tempdir().unwrap();
    let layout = a2amx::layout::Layout::new(home.path().to_path_buf());
    layout.ensure().unwrap();
    let mut token = Command::new(env!("CARGO_BIN_EXE_a2amx"))
        .args(["--home"])
        .arg(home.path())
        .args(["gateway", "token"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write as _;
    token.stdin.take().unwrap().write_all(b"\n").unwrap();
    let output = token.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(!layout.gateway_config().exists());
}
#[test]
fn failed_gateway_token_write_does_not_create_template() {
    let home = tempfile::tempdir().unwrap();
    let layout = a2amx::layout::Layout::new(home.path().to_path_buf());
    layout.ensure().unwrap();
    fs::create_dir(layout.discord_token()).unwrap();
    let mut token = Command::new(env!("CARGO_BIN_EXE_a2amx"))
        .args(["--home"])
        .arg(home.path())
        .args(["gateway", "token"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write as _;
    token
        .stdin
        .take()
        .unwrap()
        .write_all(b"bot-token\n")
        .unwrap();
    let output = token.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(!layout.gateway_config().exists());
}

#[test]
fn gateway_up_on_token_only_root_writes_template_then_reports_missing_guild() {
    let home = tempfile::tempdir().unwrap();
    let layout = a2amx::layout::Layout::new(home.path().to_path_buf());
    layout.ensure().unwrap();
    fs::write(layout.discord_token(), "bot-token\n").unwrap();
    fs::set_permissions(layout.discord_token(), fs::Permissions::from_mode(0o600)).unwrap();
    let output = run_gateway_up(home.path(), false, "");
    assert_error_contains(output.clone(), "guild_id");
    assert_error_contains(output, "not set");
    assert!(
        fs::read_to_string(layout.gateway_config())
            .unwrap()
            .contains("# guild_id =")
    );
}

#[test]
fn ensure_gateway_file_preserves_dangling_symlink() {
    let home = tempfile::tempdir().unwrap();
    let layout = a2amx::layout::Layout::new(home.path().to_path_buf());
    layout.ensure().unwrap();
    let target = home.path().join("missing-target");
    std::os::unix::fs::symlink(&target, layout.gateway_config()).unwrap();
    a2amx::config::ensure_gateway_file(&layout).unwrap();
    assert!(layout.gateway_config().read_link().is_ok());
    assert!(!target.exists());
}

#[test]
fn gateway_up_refuses_missing_inputs_and_symlinked_link() {
    let home = tempfile::tempdir().unwrap();
    prepare_state(home.path());
    fs::remove_file(a2amx::layout::Layout::new(home.path().to_path_buf()).gateway_config())
        .unwrap();
    let output = run_gateway_up(home.path(), false, "");
    assert_error_contains(output, "gateway.toml");

    let home = tempfile::tempdir().unwrap();
    prepare_state(home.path());
    fs::remove_file(a2amx::layout::Layout::new(home.path().to_path_buf()).discord_token()).unwrap();
    let output = run_gateway_up(home.path(), false, "");
    assert_error_contains(output, "discord.token");

    let home = tempfile::tempdir().unwrap();
    prepare_state(home.path());
    fs::remove_dir_all(a2amx::layout::Layout::new(home.path().to_path_buf()).link_dir()).unwrap();
    let output = run_gateway_up(home.path(), false, "");
    assert_error_contains(
        output,
        "the a2amx daemon is not running in this a2amx directory; start it first",
    );

    let home = tempfile::tempdir().unwrap();
    prepare_state(home.path());
    let target = home.path().join("link-target");
    fs::create_dir(&target).unwrap();
    fs::remove_dir_all(a2amx::layout::Layout::new(home.path().to_path_buf()).link_dir()).unwrap();
    symlink(
        &target,
        a2amx::layout::Layout::new(home.path().to_path_buf()).link_dir(),
    )
    .unwrap();
    let output = run_gateway_up(home.path(), false, "");
    assert_error_contains(
        output,
        "the a2amx daemon is not running in this a2amx directory; start it first",
    );
}
#[test]
fn gateway_up_refuses_each_missing_link_file() {
    for missing in ["addr", "gateway.token"] {
        let home = tempfile::tempdir().unwrap();
        prepare_state(home.path());
        fs::remove_file(
            a2amx::layout::Layout::new(home.path().to_path_buf())
                .link_dir()
                .join(missing),
        )
        .unwrap();
        let output = run_gateway_up(home.path(), false, "");
        assert_error_contains(
            output,
            "the a2amx daemon is not running in this a2amx directory; start it first",
        );
    }
}

#[test]
fn gateway_up_requires_pairing_without_fallback() {
    let home = tempfile::tempdir().unwrap();
    prepare_state(home.path());
    fs::remove_file(a2amx::layout::Layout::new(home.path().to_path_buf()).pairing()).unwrap();
    let output = run_gateway_up(home.path(), false, "");
    assert_error_contains(output, "not paired; run with --pair");
}

#[tokio::test]
async fn gateway_up_requires_bubblewrap_without_fallback_with_running_daemon() {
    let (home, daemon) = common::start_daemon().await;
    prepare_gateway_files(home.path());
    let output = run_gateway_up(home.path(), false, "");
    assert_error_contains(
        output,
        "bubblewrap is required to run the gateway; it never runs unconfined",
    );
    daemon.shutdown().await.unwrap();
}
#[test]
fn gateway_up_refuses_a_bubblewrap_that_fails_preflight() {
    let home = tempfile::tempdir().unwrap();
    prepare_state(home.path());
    let path = tempfile::tempdir().unwrap();
    symlink("/bin/false", path.path().join("bwrap")).unwrap();
    let path = path.path().to_string_lossy().into_owned();
    let output = run_gateway_up(home.path(), false, &path);
    assert_error_contains(
        output,
        "bubblewrap is required to run the gateway; it never runs unconfined",
    );
}

#[tokio::test]
async fn sandbox_link_mount_sees_new_address_after_daemon_restart() {
    let Some(bwrap) = bwrap_for_test() else {
        return;
    };
    let (home, daemon) = common::start_daemon().await;
    let old_address = daemon.addrs()[0].to_string();
    let root_layout = a2amx::layout::Layout::new(home.path().to_path_buf());
    root_layout.ensure().unwrap();
    prepare_gateway_files(home.path());
    fs::write(root_layout.link_addr(), format!("{old_address}\n")).unwrap();
    let (layout, shell) = shell_layout(home.path(), &bwrap, None);
    let state = layout.layout.root().to_string_lossy().into_owned();
    let script = r#"set -eu
cat "$1/run/link/addr"
while [ ! -e "$1/state/gateway/release" ]; do sleep 0.01; done
cat "$1/run/link/addr"
"#;
    let inner = vec![shell, "-c".into(), script.into(), "probe".into(), state];
    let argv = sandbox_argv(&layout, &inner);
    let mut child = tokio::process::Command::new(&argv[0])
        .args(&argv[1..])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = AsyncBufReader::new(stdout);
    let mut first = String::new();
    match tokio::time::timeout(
        std::time::Duration::from_secs(10),
        reader.read_line(&mut first),
    )
    .await
    {
        Ok(result) => result.unwrap(),
        Err(error) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            panic!("timed out waiting for initial link address: {error}");
        }
    };
    if first.trim() != old_address {
        let _ = child.kill().await;
        let _ = child.wait().await;
        panic!(
            "initial link address was {:?}, expected {old_address}",
            first.trim()
        );
    }

    daemon.shutdown().await.unwrap();
    let restarted = common::start_daemon_in(
        home.path(),
        Some("host-a"),
        a2amx::messaging::Limits::default(),
    )
    .await;
    let new_address = restarted.addrs()[0].to_string();
    fs::write(home.path().join("state/gateway/release"), b"ready\n").unwrap();
    let mut second = String::new();
    match tokio::time::timeout(
        std::time::Duration::from_secs(10),
        reader.read_line(&mut second),
    )
    .await
    {
        Ok(result) => result.unwrap(),
        Err(error) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            panic!("timed out waiting for restarted link address: {error}");
        }
    };
    if second.trim() != new_address {
        let _ = child.kill().await;
        let _ = child.wait().await;
        panic!(
            "restarted link address was {:?}, expected {new_address}",
            second.trim()
        );
    }
    let status = match tokio::time::timeout(std::time::Duration::from_secs(10), child.wait()).await
    {
        Ok(result) => result.unwrap(),
        Err(error) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            panic!("timed out waiting for sandbox child exit: {error}");
        }
    };
    assert_eq!(status.code(), Some(0));
    restarted.shutdown().await.unwrap();
}
