#![allow(clippy::expect_used, clippy::unwrap_used)]
mod common;

use tokio::io::{AsyncBufReadExt, BufReader as AsyncBufReader};

use std::fs;
use std::os::unix::fs::{FileTypeExt, PermissionsExt, symlink};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use a2amx::gateway_launch::{Layout, sandbox_argv};

#[test]
fn sandbox_argv_matches_worked_vector_with_external_resolv_target() {
    let layout = Layout {
        bwrap: PathBuf::from("/usr/bin/bwrap"),
        state_dir: PathBuf::from("/home/me/state"),
        exe: PathBuf::from("/opt/a2amx"),
        rust_log: Some("debug".into()),
        resolv_target: Some(PathBuf::from("/mnt/wsl/resolv.conf")),
    };
    let inner = vec![
        "/opt/a2amx".into(),
        "--home".into(),
        "/home/me/state".into(),
        "gateway".into(),
        "run".into(),
        "--pair".into(),
    ];
    assert_eq!(
        sandbox_argv(&layout, &inner),
        vec![
            "/usr/bin/bwrap",
            "--die-with-parent",
            "--unshare-user",
            "--unshare-ipc",
            "--unshare-pid",
            "--tmpfs",
            "/",
            "--ro-bind",
            "/usr",
            "/usr",
            "--ro-bind-try",
            "/bin",
            "/bin",
            "--ro-bind-try",
            "/sbin",
            "/sbin",
            "--ro-bind-try",
            "/lib",
            "/lib",
            "--ro-bind-try",
            "/lib64",
            "/lib64",
            "--ro-bind",
            "/etc",
            "/etc",
            "--ro-bind",
            "/mnt/wsl/resolv.conf",
            "/mnt/wsl/resolv.conf",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--size",
            "1073741824",
            "--tmpfs",
            "/tmp",
            "--ro-bind",
            "/opt/a2amx",
            "/opt/a2amx",
            "--ro-bind",
            "/home/me/state/link",
            "/home/me/state/link",
            "--bind",
            "/home/me/state/gateway",
            "/home/me/state/gateway",
            "--remount-ro",
            "/",
            "--clearenv",
            "--setenv",
            "PATH",
            "/usr/local/bin:/usr/bin:/bin",
            "--setenv",
            "RUST_LOG",
            "debug",
            "--chdir",
            "/home/me/state/gateway",
            "--",
            "/opt/a2amx",
            "--home",
            "/home/me/state",
            "gateway",
            "run",
            "--pair",
        ]
    );
}

#[test]
fn sandbox_argv_matches_worked_vector_without_external_resolv_target() {
    let layout = Layout {
        bwrap: PathBuf::from("/usr/bin/bwrap"),
        state_dir: PathBuf::from("/home/me/state"),
        exe: PathBuf::from("/opt/a2amx"),
        rust_log: None,
        resolv_target: None,
    };
    let inner = vec![
        "/opt/a2amx".into(),
        "--home".into(),
        "/home/me/state".into(),
        "gateway".into(),
        "run".into(),
    ];
    assert_eq!(
        sandbox_argv(&layout, &inner),
        vec![
            "/usr/bin/bwrap",
            "--die-with-parent",
            "--unshare-user",
            "--unshare-ipc",
            "--unshare-pid",
            "--tmpfs",
            "/",
            "--ro-bind",
            "/usr",
            "/usr",
            "--ro-bind-try",
            "/bin",
            "/bin",
            "--ro-bind-try",
            "/sbin",
            "/sbin",
            "--ro-bind-try",
            "/lib",
            "/lib",
            "--ro-bind-try",
            "/lib64",
            "/lib64",
            "--ro-bind",
            "/etc",
            "/etc",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--size",
            "1073741824",
            "--tmpfs",
            "/tmp",
            "--ro-bind",
            "/opt/a2amx",
            "/opt/a2amx",
            "--ro-bind",
            "/home/me/state/link",
            "/home/me/state/link",
            "--bind",
            "/home/me/state/gateway",
            "/home/me/state/gateway",
            "--remount-ro",
            "/",
            "--clearenv",
            "--setenv",
            "PATH",
            "/usr/local/bin:/usr/bin:/bin",
            "--chdir",
            "/home/me/state/gateway",
            "--",
            "/opt/a2amx",
            "--home",
            "/home/me/state",
            "gateway",
            "run",
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
    fs::create_dir_all(home.join("gateway")).unwrap();
    fs::set_permissions(home.join("gateway"), fs::Permissions::from_mode(0o700)).unwrap();
    for (path, contents) in [
        ("gateway/discord.token", "bot-token\n"),
        ("gateway/config.toml", "guild_id = \"123\"\n"),
        (
            "gateway/pairing.json",
            r#"{"guild_id":"123","user_id":"456"}"#,
        ),
    ] {
        fs::write(home.join(path), contents).unwrap();
        fs::set_permissions(home.join(path), fs::Permissions::from_mode(0o600)).unwrap();
    }
}

fn prepare_state(home: &Path) {
    prepare_gateway_files(home);
    fs::create_dir_all(home.join("link")).unwrap();
    fs::set_permissions(home.join("link"), fs::Permissions::from_mode(0o700)).unwrap();
    for (path, contents) in [
        ("admin.token", "admin-secret\n"),
        ("link/addr", "127.0.0.1:1\n"),
        ("link/gateway.token", "gateway-secret\n"),
    ] {
        fs::write(home.join(path), contents).unwrap();
        fs::set_permissions(home.join(path), fs::Permissions::from_mode(0o600)).unwrap();
    }
}

fn shell_layout(home: &Path, bwrap: &Path, rust_log: Option<&str>) -> (Layout, String) {
    let exe = fs::canonicalize("/bin/sh").unwrap();
    (
        Layout {
            bwrap: bwrap.to_owned(),
            state_dir: fs::canonicalize(home).unwrap(),
            exe,
            rust_log: rust_log.map(str::to_owned),
            resolv_target: None,
        },
        fs::canonicalize("/bin/sh")
            .unwrap()
            .to_string_lossy()
            .into_owned(),
    )
}

fn run_sandbox(layout: &Layout, inner: &[String], env: &[(&str, &str)]) -> Output {
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
    let state = layout.state_dir.to_string_lossy().into_owned();
    let script = r#"set -eu
test ! -e "$1/admin.token"
if cat "$1/admin.token" 2>/dev/null; then exit 1; fi
entries=$(find "$1" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort)
test "$entries" = "gateway
link"
test ! -e /root
test ! -e /var
test ! -e /opt
test ! -e /srv
test ! -e /run
root_entries=$(find / -mindepth 1 -maxdepth 1 -printf '%f\n' | sort)
test "$root_entries" = "bin
dev
etc
home
lib
lib64
proc
sbin
tmp
usr"
set +e
(printf x > "$1/link/x") 2>/dev/null
link_status=$?
(printf x > "$1/marker") 2>/dev/null
marker_status=$?
set -e
test "$link_status" -ne 0
test "$marker_status" -ne 0
printf x > "$1/gateway/x"
test "$(cat "$1/gateway/x")" = x
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
    assert!(home.path().join("gateway/x").is_file());
    assert!(!home.path().join("link/x").exists());
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
    let layout = Layout {
        bwrap: bwrap.clone(),
        state_dir: state,
        exe: python.clone(),
        rust_log: None,
        resolv_target: None,
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
    let layout = Layout {
        bwrap,
        state_dir: state.clone(),
        exe: env_exe.clone(),
        rust_log: Some("trace".into()),
        resolv_target: None,
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
        format!("PWD={}", state.join("gateway").display()),
        "RUST_LOG=trace".to_owned(),
    ];
    expected.sort();
    assert_eq!(lines, expected);
}

#[test]
fn sandbox_keeps_inner_arguments_after_separator() {
    let layout = Layout {
        bwrap: PathBuf::from("/usr/bin/bwrap"),
        state_dir: PathBuf::from("/tmp/- state 'quoted'"),
        exe: PathBuf::from("/opt/a2amx"),
        rust_log: None,
        resolv_target: None,
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
fn gateway_up_refuses_missing_inputs_and_symlinked_link() {
    let home = tempfile::tempdir().unwrap();
    prepare_state(home.path());
    fs::remove_file(home.path().join("gateway/config.toml")).unwrap();
    let output = run_gateway_up(home.path(), false, "");
    assert_error_contains(output, "config.toml");

    let home = tempfile::tempdir().unwrap();
    prepare_state(home.path());
    fs::remove_file(home.path().join("gateway/discord.token")).unwrap();
    let output = run_gateway_up(home.path(), false, "");
    assert_error_contains(output, "discord.token");

    let home = tempfile::tempdir().unwrap();
    prepare_state(home.path());
    fs::remove_dir_all(home.path().join("link")).unwrap();
    let output = run_gateway_up(home.path(), false, "");
    assert_error_contains(
        output,
        "the a2amx daemon is not running in this state directory; start it first",
    );

    let home = tempfile::tempdir().unwrap();
    prepare_state(home.path());
    let target = home.path().join("link-target");
    fs::create_dir(&target).unwrap();
    fs::remove_dir_all(home.path().join("link")).unwrap();
    symlink(&target, home.path().join("link")).unwrap();
    let output = run_gateway_up(home.path(), false, "");
    assert_error_contains(
        output,
        "the a2amx daemon is not running in this state directory; start it first",
    );
}
#[test]
fn gateway_up_refuses_each_missing_link_file() {
    for missing in ["addr", "gateway.token"] {
        let home = tempfile::tempdir().unwrap();
        prepare_state(home.path());
        fs::remove_file(home.path().join("link").join(missing)).unwrap();
        let output = run_gateway_up(home.path(), false, "");
        assert_error_contains(
            output,
            "the a2amx daemon is not running in this state directory; start it first",
        );
    }
}

#[test]
fn gateway_up_requires_pairing_without_fallback() {
    let home = tempfile::tempdir().unwrap();
    prepare_state(home.path());
    fs::remove_file(home.path().join("gateway/pairing.json")).unwrap();
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
    fs::create_dir_all(home.path().join("gateway")).unwrap();
    fs::set_permissions(
        home.path().join("gateway"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let old_address = daemon.addrs()[0].to_string();
    let (layout, shell) = shell_layout(home.path(), &bwrap, None);
    let state = layout.state_dir.to_string_lossy().into_owned();
    let script = r#"set -eu
cat "$1/link/addr"
while [ ! -e "$1/gateway/release" ]; do sleep 0.01; done
cat "$1/link/addr"
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
    fs::write(home.path().join("gateway/release"), b"ready\n").unwrap();
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
