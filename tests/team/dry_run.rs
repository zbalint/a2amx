use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dry_run_prints_wired_commands_without_starting_or_installing() {
    let (dir, _daemon) = common::start_daemon().await;
    std::fs::write(
        dir.path().join("dry.toml"),
        r#"
team = "wired"
[[session]]
name = "claude"
command = ["claude", "--settings", "{\"x\":\"UNIQUE-SECRET\"}"]
cwd = "."
role = "developer"
attach = true
watch = ["/other"]
control_from = ["/other"]
heartbeat = "5s"
activity = { working_after = "2m", idle_after = "30s" }
[[session]]
name = "generic"
command = ["sh", "-c", "echo generic"]
[[session]]
name = "omp"
command = ["omp", "--secret"]
"#,
    )
    .unwrap();
    let output = run_binary(
        dir.path(),
        &["team", "up", "--dry-run", "--file", "dry.toml"],
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("would start wired-claude\n"));
    assert!(stdout.contains("would start wired-generic\n"));
    assert!(stdout.contains("would start wired-omp\n"));
    assert!(stdout.contains("  harness: claude\n"));
    assert!(stdout.contains("  cwd: "));
    assert!(stdout.contains("  role: developer\n"));
    assert!(stdout.contains("  attach: true\n"));
    assert!(stdout.contains("  watch: other\n"));
    assert!(stdout.contains("  control_from: other\n"));
    assert!(stdout.contains("  heartbeat: 5s\n"));
    assert!(stdout.contains("  activity: working_after 2m, idle_after 30s\n"));
    assert!(stdout.contains("  command: claude "));
    assert!(stdout.contains("--mcp-config "));
    assert!(stdout.contains("--settings "));
    assert!(stdout.contains("--append-system-prompt "));
    assert!(stdout.contains("\nomp extension install skipped (dry run)\n"));
    assert!(!stdout.contains("UNIQUE-SECRET"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("UNIQUE-SECRET"));
    assert!(!dir.path().join("omp").exists());
    let listed = run_binary(dir.path(), &["list"]).unwrap();
    assert!(listed.status.success());
    assert!(!String::from_utf8_lossy(&listed.stdout).contains("claude"));
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dry_run_expands_home_relative_confine_paths() {
    let (state, _daemon) = common::start_daemon().await;
    let project = tempfile::tempdir().unwrap();
    let user_home = tempfile::tempdir().unwrap();
    let runtime_dir = tempfile::tempdir().unwrap();
    let tool_cache = user_home.path().join(".tool-cache");
    let root_tool_cache = state.path().join(".tool-cache");
    let tool_hide = user_home.path().join(".tool-hide");
    let ssh = user_home.path().join(".ssh");
    std::fs::create_dir(&tool_cache).unwrap();
    std::fs::create_dir(&root_tool_cache).unwrap();
    std::fs::create_dir(&ssh).unwrap();
    std::fs::create_dir(&tool_hide).unwrap();
    std::fs::write(
        project.path().join("dry-tilde.toml"),
        r#"
        [confine]
        enabled = true
        [[session]]
        name = "demo"
        command = ["sh", "-c", "echo demo"]
        cwd = "."
        [session.confine]
        write = ["~/.tool-cache", "~/.missing-tool"]
        hide = ["~/.tool-hide"]
        unhide = ["~/.ssh"]
        grants = ["gh"]
        keep_env = ["GITHUB_TOKEN", "GITHUB_TOKEN"]
        "#,
    )
    .unwrap();
    let output = run_binary_in_with_home_and_runtime(
        state.path(),
        project.path(),
        user_home.path(),
        runtime_dir.path(),
        &["team", "up", "--dry-run", "--file", "dry-tilde.toml"],
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let writable = std::fs::canonicalize(&tool_cache).unwrap();
    let root_writable = std::fs::canonicalize(&root_tool_cache).unwrap();
    let hidden = std::fs::canonicalize(&tool_hide).unwrap();
    let unhidden = std::fs::canonicalize(&ssh).unwrap();
    let missing = user_home.path().join(".missing-tool");
    assert!(stdout.contains("  grants: gh\n"));
    assert!(stdout.contains("  keep_env: GITHUB_TOKEN\n"));
    assert!(!stdout.contains(&format!("  hidden: {}\n", unhidden.display())));
    assert!(stdout.contains(&format!("  writable: {}\n", writable.display())));
    assert!(stdout.contains(&format!("  hidden: {}\n", hidden.display())));
    assert!(!stdout.contains(&format!("  writable: {}\n", root_writable.display())));
    assert!(!stdout.contains("/~/"));
    assert!(!stdout.contains("~/"));
    assert!(!stdout.contains(&format!("  writable: {}\n", missing.display())));
    assert!(stdout.contains(&format!(
        "  unresolved: {} (write: not found)\n",
        missing.display()
    )));
    assert!(!stdout.contains(&format!(
        "  hidden: {} (skipped missing)\n",
        missing.display()
    )));
    let default_missing = user_home.path().join(".aws");
    assert!(stdout.contains(&format!(
        "  hidden: {} (skipped missing)\n",
        default_missing.display()
    )));
    std::fs::write(
        project.path().join("dry-plain.toml"),
        r#"
        [confine]
        enabled = true
        [[session]]
        name = "plain"
        command = ["sh", "-c", "echo plain"]
        cwd = "."
        [session.confine]
        write = ["~/.tool-cache"]
        "#,
    )
    .unwrap();
    let plain_output = run_binary_in_with_home_and_runtime(
        state.path(),
        project.path(),
        user_home.path(),
        runtime_dir.path(),
        &["team", "up", "--dry-run", "--file", "dry-plain.toml"],
    )
    .unwrap();
    assert!(
        plain_output.status.success(),
        "{}",
        String::from_utf8_lossy(&plain_output.stderr)
    );
    let plain_stdout = String::from_utf8(plain_output.stdout).unwrap();
    assert!(plain_stdout.contains("  confinement: manual\n"));
    assert!(!plain_stdout.contains("  grants: "));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dry_run_reports_auto_grants_and_skipped_tool_directories() {
    let (state, _daemon) = common::start_daemon().await;
    let project = tempfile::tempdir().unwrap();
    let user_home = tempfile::tempdir().unwrap();
    let runtime_dir = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("Cargo.toml"), "").unwrap();
    std::fs::write(
        state.path().join("config/team.toml"),
        r#"
[confine]
enabled = true
auto = true
[[session]]
name = "worker"
command = ["sh", "-c", "echo worker"]
cwd = "."
"#,
    )
    .unwrap();
    let output = run_binary_in_with_home_and_runtime(
        state.path(),
        project.path(),
        user_home.path(),
        runtime_dir.path(),
        &["team", "up", "--dry-run"],
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("  grants: cache, cargo\n"));
    assert!(stdout.contains(&format!(
        "  hidden: {} (skipped missing)\n",
        user_home.path().join(".cargo").display()
    )));
    assert!(stdout.contains(&format!(
        "  hidden: {} (skipped missing)\n",
        user_home.path().join(".cache").display()
    )));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dry_run_rejects_unknown_keep_env() {
    let (state, _daemon) = common::start_daemon().await;
    let project = tempfile::tempdir().unwrap();
    let user_home = tempfile::tempdir().unwrap();
    let runtime_dir = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("unknown-keep-env.toml"),
        r#"
[confine]
enabled = true
[[session]]
name = "worker"
command = ["sh", "-c", "echo worker"]
cwd = "."
[session.confine]
keep_env = ["NOT_A_CREDENTIAL"]
"#,
    )
    .unwrap();
    let output = run_binary_in_with_home_and_runtime(
        state.path(),
        project.path(),
        user_home.path(),
        runtime_dir.path(),
        &["team", "up", "--dry-run", "--file", "unknown-keep-env.toml"],
    )
    .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(
        "keep_env \"NOT_A_CREDENTIAL\" is not unset by default, so there is nothing to keep"
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dry_run_rejects_existing_relative_omp_prompt_file() {
    let (dir, _daemon) = common::start_daemon().await;
    std::fs::write(dir.path().join("prompt.txt"), "prompt").unwrap();
    std::fs::write(
        dir.path().join("invalid-omp.toml"),
        r#"
        [confine]
        enabled = true
        [[session]]
        name = "omp"
        command = ["omp", "--append-system-prompt=prompt.txt"]
        cwd = "."
        "#,
    )
    .unwrap();
    let output = run_binary(
        dir.path(),
        &["team", "up", "--dry-run", "--file", "invalid-omp.toml"],
    )
    .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("existing file as OMP's append-system-prompt value"));
    assert!(!stderr.contains("would start omp"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dry_run_prints_exact_private_team_report() {
    let (dir, _daemon) = common::start_daemon().await;
    std::fs::write(
        dir.path().join("dry.toml"),
        "team = \"t\"\n[[session]]\nname = \"worker\"\ncommand = [\"cat\"]\ncwd = \"/\"\n",
    )
    .unwrap();
    let output = run_binary(
        dir.path(),
        &["team", "up", "--dry-run", "--file", "dry.toml"],
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "would start t-worker\n  harness: generic\n  cwd: /\n  team: t, private, allow: -\n  confinement: off\n  command: cat\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dry_run_prints_exact_already_running_report() {
    let (dir, _daemon) = common::start_daemon().await;
    std::fs::write(
        dir.path().join("running.toml"),
        "team = \"running\"\n[[session]]\nname = \"running\"\ncommand = [\"cat\"]\n",
    )
    .unwrap();
    let started = run_binary(
        dir.path(),
        &["team", "up", "--detach", "--file", "running.toml"],
    )
    .unwrap();
    assert!(
        started.status.success(),
        "{}",
        String::from_utf8_lossy(&started.stderr)
    );
    let output = run_binary(
        dir.path(),
        &["team", "up", "--dry-run", "--file", "running.toml"],
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "already running running-running s1 (confine off)\n"
    );
}

#[test]
fn dry_run_rejects_detach_and_inline_items() {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        vec!["team", "up", "--dry-run", "--detach"],
        vec!["team", "up", "--dry-run", "worker=cat"],
    ] {
        let output = run_binary(dir.path(), &args).unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be used with"));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dry_run_redacts_flag_values_and_non_flags() {
    let (dir, _daemon) = common::start_daemon().await;
    std::fs::write(
        dir.path().join("redact.toml"),
        r#"
[[session]]
name = "values"
command = ["claude", "--model", "sonnet", "--x=ab", "pos"]
[[session]]
name = "separator"
command = ["claude", "--", "-secret"]
[[session]]
name = "short"
command = ["claude", "-token123"]
"#,
    )
    .unwrap();
    let output = run_binary(
        dir.path(),
        &["team", "up", "--dry-run", "--file", "redact.toml"],
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("command: claude --model <6 bytes> --x=<redacted, 2 bytes> <3 bytes>"));
    assert!(stdout.contains(" -- <7 bytes>"));
    assert!(stdout.contains("command: claude <9 bytes>"));
    assert!(!stdout.contains("sonnet"));
    assert!(!stdout.contains("-secret"));
    assert!(!stdout.contains("-token123"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dry_run_prints_exact_all_optional_details_in_order() {
    let (dir, _daemon) = common::start_daemon().await;
    std::fs::write(
        dir.path().join("dry-all.toml"),
        r#"
team = "t"
[[session]]
name = "worker"
command = ["cat"]
cwd = "/"
role = "developer"
attach = true
watch = ["other"]
control_from = ["other"]
heartbeat = "5s"
activity = { working_after = "2m", idle_after = "30s" }
[[session]]
name = "other"
command = ["cat"]
cwd = "/"
"#,
    )
    .unwrap();
    let output = run_binary(
        dir.path(),
        &["team", "up", "--dry-run", "--file", "dry-all.toml"],
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "would start t-worker\n  harness: generic\n  cwd: /\n  team: t, private, allow: -\n  role: developer\n  attach: true\n  watch: t-other\n  control_from: t-other\n  heartbeat: 5s\n  activity: working_after 2m, idle_after 30s\n  confinement: off\n  command: cat\nwould start t-other\n  harness: generic\n  cwd: /\n  team: t, private, allow: -\n  confinement: off\n  command: cat\n",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dry_run_prints_working_only_activity_detail() {
    let (dir, _daemon) = common::start_daemon().await;
    std::fs::write(
        dir.path().join("dry-working.toml"),
        "[[session]]\nname = 'worker'\ncommand = ['cat']\nwatch = ['other']\nactivity = { working_after = '2m' }\n[[session]]\nname = 'other'\ncommand = ['cat']",
    )
    .unwrap();
    let output = run_binary(
        dir.path(),
        &["team", "up", "--dry-run", "--file", "dry-working.toml"],
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("  activity: working_after 2m\n")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dry_run_prints_idle_only_activity_detail() {
    let (dir, _daemon) = common::start_daemon().await;
    std::fs::write(
        dir.path().join("dry-idle.toml"),
        "[[session]]\nname = 'worker'\ncommand = ['cat']\nwatch = ['other']\nactivity = { idle_after = '30s' }\n[[session]]\nname = 'other'\ncommand = ['cat']",
    )
    .unwrap();
    let output = run_binary(
        dir.path(),
        &["team", "up", "--dry-run", "--file", "dry-idle.toml"],
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("  activity: idle_after 30s\n")
    );
}
