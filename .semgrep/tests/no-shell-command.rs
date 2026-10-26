fn commands(program: &str) {
    // ruleid: no-shell-command
    Command::new("sh");
    // ruleid: no-shell-command
    std::process::Command::new("/bin/bash");
    // ruleid: no-shell-command
    tokio::process::Command::new("cmd.exe");
    // ok: no-shell-command
    Command::new("git");
    // ok: no-shell-command
    Command::new("shell-tool");
    // ok: no-shell-command
    Command::new(program);
}
