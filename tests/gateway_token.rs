#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::io::Write;
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_a2amx");

#[test]
fn gateway_token_with_an_empty_line_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let mut command = Command::new(BIN);
    command
        .args(["--home", home.to_str().unwrap(), "gateway", "token"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    child.stdin.as_mut().unwrap().write_all(b"\n").unwrap();
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "a2amx: gateway token is empty\n"
    );
    assert!(!home.join("config/secrets/discord.token").exists());
}
