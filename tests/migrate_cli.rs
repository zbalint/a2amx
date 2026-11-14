#![allow(clippy::expect_used, clippy::unwrap_used)]
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use rusqlite::Connection;
use tempfile::tempdir;

const BIN: &str = env!("CARGO_BIN_EXE_a2amx");

fn write(path: &std::path::Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, bytes).unwrap();
}

fn start_foreground(
    home: &std::path::Path,
    explicit: Option<&std::path::Path>,
) -> std::process::Child {
    let mut command = Command::new(BIN);
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

fn stop(child: std::process::Child) -> String {
    let pid = child.id() as i32;
    // SAFETY: this test owns the child process and sends only SIGTERM to its pid.
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
    let output = child.wait_with_output().unwrap();
    String::from_utf8(output.stderr).unwrap()
}

#[test]
fn test_14_daemon_start_migrates_and_prints_delete_hint() {
    let dir = tempdir().unwrap();
    let old = dir.path().join(".local/state/a2amx");
    std::fs::create_dir_all(&old).unwrap();
    let connection = Connection::open(old.join("messages.db")).unwrap();
    connection.execute_batch(
        "CREATE TABLE messages (seq INTEGER PRIMARY KEY AUTOINCREMENT, boot TEXT NOT NULL, sender_session TEXT NOT NULL, sender_address TEXT NOT NULL, recipient_session TEXT NOT NULL, recipient_address TEXT NOT NULL, subject TEXT NOT NULL, body TEXT NOT NULL, state TEXT NOT NULL, detail TEXT, accepted_at INTEGER NOT NULL, updated_at INTEGER NOT NULL); CREATE INDEX messages_recipient ON messages (boot, recipient_session, state, seq); CREATE TABLE attempts (id INTEGER PRIMARY KEY AUTOINCREMENT, message_seq INTEGER NOT NULL REFERENCES messages(seq), started_at INTEGER NOT NULL, outcome TEXT NOT NULL, detail TEXT, finished_at INTEGER, receipt_at INTEGER); PRAGMA user_version = 2; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;",
    ).unwrap();
    drop(connection);
    let expected = std::fs::read(old.join("messages.db")).unwrap();
    let mut child = start_foreground(dir.path(), None);
    let mut line = String::new();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    stdout.read_line(&mut line).unwrap();
    assert!(line.contains("listening on"), "{line}");
    let stderr = stop(child);
    assert!(
        stderr.contains("moved") && stderr.contains("rm -r"),
        "{stderr}"
    );
    assert_eq!(
        std::fs::read(dir.path().join(".a2amx/state/messages.db")).unwrap(),
        expected
    );
}

#[test]
fn test_14_gateway_token_migrates_and_explicit_home_is_ignored() {
    let dir = tempdir().unwrap();
    let old = dir.path().join(".local/state/a2amx");
    write(&old.join("messages.db"), b"old-db");
    let mut command = Command::new(BIN);
    command
        .args(["gateway", "token"])
        .env("HOME", dir.path())
        .env_remove("A2AMX_HOME")
        .env_remove("XDG_STATE_HOME")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"gateway-token\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read(dir.path().join(".a2amx/state/messages.db")).unwrap(),
        b"old-db"
    );

    write(
        &dir.path().join(".local/state/a2amx/daemon.log"),
        b"must-not-move",
    );
    let explicit = dir.path().join("explicit");
    write(&explicit.join("messages.db"), b"flat");
    write(&explicit.join("gateway/config.toml"), b"flat");
    let mut child = start_foreground(dir.path(), Some(&explicit));
    let mut line = String::new();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    stdout.read_line(&mut line).unwrap();
    assert!(line.contains("listening on"), "{line}");
    let stderr = stop(child);
    assert!(stderr.contains("old layout"), "{stderr}");
    assert!(!stderr.contains("moved"), "{stderr}");
    assert_eq!(
        std::fs::read(dir.path().join(".local/state/a2amx/daemon.log")).unwrap(),
        b"must-not-move"
    );
    assert!(explicit.join("messages.db").exists());
}
