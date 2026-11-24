use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};

use tempfile::tempdir;

fn write(path: &std::path::Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, bytes).unwrap();
}

fn start_foreground(home: &std::path::Path, explicit: Option<&std::path::Path>) -> Child {
    let mut command = Command::new(super::BIN);
    if let Some(path) = explicit {
        command.args(["--home", path.to_str().unwrap()]);
    }
    command.args(["daemon", "start", "--foreground", "--listen", "127.0.0.1:0"]);
    command.env("HOME", home);
    command.env_remove("A2AMX_HOME");
    command.env_remove("XDG_STATE_HOME");
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn stop(child: Child) -> String {
    let pid = child.id() as i32;
    // SAFETY: this test owns the child process and sends only SIGTERM to its pid.
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
    let output = child.wait_with_output().unwrap();
    String::from_utf8(output.stderr).unwrap()
}

#[test]
fn daemon_start_keeps_old_state_and_uses_default_root() {
    let dir = tempdir().unwrap();
    let old = dir.path().join(".local/state/a2amx");
    let old_db = b"old-db";
    let old_team = b"old-team";
    write(&old.join("messages.db"), old_db);
    write(&old.join("team.toml"), old_team);

    let mut child = start_foreground(dir.path(), None);
    let mut line = String::new();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    stdout.read_line(&mut line).unwrap();
    let address = line
        .strip_prefix("listening on ")
        .and_then(|value| value.strip_suffix('\n'))
        .and_then(|value| value.parse::<std::net::SocketAddr>().ok());
    assert!(address.is_some(), "{line}");
    let stderr = stop(child);
    let mut rest = String::new();
    stdout.read_to_string(&mut rest).unwrap();
    assert_eq!(rest, "");
    assert!(!stderr.contains("moved "), "{stderr}");
    assert!(!stderr.contains("old layout"), "{stderr}");

    assert_eq!(std::fs::read(old.join("messages.db")).unwrap(), old_db);
    assert_eq!(std::fs::read(old.join("team.toml")).unwrap(), old_team);
    let new_db = std::fs::read(dir.path().join(".a2amx/state/messages.db")).unwrap();
    assert_ne!(new_db, old_db);

    let explicit = dir.path().join("explicit");
    write(&explicit.join("messages.db"), b"flat");
    let mut child = start_foreground(dir.path(), Some(&explicit));
    let mut line = String::new();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    stdout.read_line(&mut line).unwrap();
    let address = line
        .strip_prefix("listening on ")
        .and_then(|value| value.strip_suffix('\n'))
        .and_then(|value| value.parse::<std::net::SocketAddr>().ok());
    assert!(address.is_some(), "{line}");
    let stderr = stop(child);
    let mut rest = String::new();
    stdout.read_to_string(&mut rest).unwrap();
    assert_eq!(rest, "");
    assert!(!stderr.contains("old layout"), "{stderr}");
    assert_eq!(
        std::fs::read(explicit.join("messages.db")).unwrap(),
        b"flat"
    );
}
