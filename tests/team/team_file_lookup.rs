use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn team_file_dry_run_uses_local_file_and_derived_team() {
    let home = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(home.path(), Some("host-a"), Default::default()).await;
    let project = home.path().join("proj-one");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("a2amx.toml"),
        "[[session]]\nname = 'worker'\ncommand = ['sh', '-c', 'sleep 30']\n",
    )
    .unwrap();
    let output = run_binary_in(home.path(), &project, &["team", "up", "--dry-run"]).unwrap();
    assert!(output.status.success(), "{:?}", output);
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains(&format!("  cwd: {}\n", project.display())),
        "{:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    let started = run_binary_in(home.path(), &project, &["team", "up", "--detach"]).unwrap();
    assert!(started.status.success(), "{:?}", started);
    let status = run_binary_in(home.path(), &project, &["team", "status"]).unwrap();
    assert!(status.status.success(), "{:?}", status);
    let status = String::from_utf8(status.stdout).unwrap();
    assert!(
        status.contains("proj-one-worker proj-one (private)"),
        "{status}"
    );
    let stopped = run_binary_in(home.path(), &project, &["team", "down"]).unwrap();
    assert!(stopped.status.success(), "{:?}", stopped);
    daemon.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn team_file_dry_run_canonicalizes_dotdot_parent_for_omitted_cwd() {
    let home = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(home.path(), Some("host-a"), Default::default()).await;
    let project = home.path().join("fixed-project");
    let child = project.join("child");
    std::fs::create_dir_all(&child).unwrap();
    std::fs::write(
        project.join("team.toml"),
        "[[session]]\nname = 'worker'\ncommand = ['sh', '-c', 'pwd']\n",
    )
    .unwrap();
    let file = child.join("../team.toml").to_string_lossy().into_owned();
    let output = run_binary_in(
        home.path(),
        &child,
        &["team", "up", "--dry-run", "--file", &file],
    )
    .unwrap();
    assert!(output.status.success(), "{:?}", output);
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains(&format!("  cwd: {}\n", project.display())),
        "{:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn team_file_cli_reports_legacy_allow_rename() {
    let home = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(home.path(), Some("host-a"), Default::default()).await;
    let file = home.path().join("legacy.toml");
    std::fs::write(
        &file,
        "[[session]]\nname = 'worker'\ncommand = ['cat']\nallow = ['x/y']\n",
    )
    .unwrap();
    let file = file.to_string_lossy().into_owned();
    let output = run_binary_in(
        home.path(),
        home.path(),
        &["team", "up", "--dry-run", "--file", &file],
    )
    .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("allow was renamed to allow_agents"),
        "{stderr}"
    );
    assert!(stderr.contains(&file), "{stderr}");
    daemon.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn team_file_explicit_team_works_in_non_derivable_directory_and_omitted_fails() {
    let home = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(home.path(), Some("host-a"), Default::default()).await;
    let dir = home.path().join("___");
    std::fs::create_dir_all(&dir).unwrap();
    let explicit = dir.join("explicit.toml");
    std::fs::write(
        &explicit,
        "team = 'explicit'\n[[session]]\nname = 'worker'\ncommand = ['cat']\n",
    )
    .unwrap();
    let explicit = explicit.to_string_lossy().into_owned();
    let output = run_binary_in(
        home.path(),
        &dir,
        &["team", "up", "--dry-run", "--file", &explicit],
    )
    .unwrap();
    assert!(output.status.success(), "{:?}", output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("would start explicit-worker"));

    let omitted = dir.join("omitted.toml");
    std::fs::write(
        &omitted,
        "[[session]]\nname = 'worker'\ncommand = ['cat']\n",
    )
    .unwrap();
    let omitted = omitted.to_string_lossy().into_owned();
    let output = run_binary_in(
        home.path(),
        &dir,
        &["team", "up", "--dry-run", "--file", &omitted],
    )
    .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("set team in the file"));
    daemon.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_team_file_is_used_with_local_precedence_and_guard() {
    let (state, _daemon) = common::start_daemon().await;
    let project_root = tempfile::tempdir().unwrap();
    let project = project_root.path().join("proj");
    std::fs::create_dir(&project).unwrap();
    let user_home = tempfile::tempdir().unwrap();
    std::fs::write(
        state.path().join("config/team.toml"),
        "[[session]]\nname = 'architect'\ncommand = ['cat']",
    )
    .unwrap();
    let global = run_binary_in(state.path(), &project, &["team", "status"]).unwrap();
    assert!(
        global.status.success(),
        "{}",
        String::from_utf8_lossy(&global.stderr)
    );
    assert!(String::from_utf8_lossy(&global.stderr).contains("using global team file"));
    assert!(String::from_utf8_lossy(&global.stdout).contains("proj-architect"));
    std::fs::write(
        project.join("a2amx.toml"),
        "[[session]]\nname = 'architect'\ncommand = ['cat']",
    )
    .unwrap();
    let local = run_binary_in(state.path(), &project, &["team", "status"]).unwrap();
    assert!(local.status.success());
    assert!(String::from_utf8_lossy(&local.stdout).contains("architect"));
    assert!(!String::from_utf8_lossy(&local.stderr).contains("using global team file"));
    let guarded = run_binary_in_with_home(
        state.path(),
        user_home.path(),
        user_home.path(),
        &["team", "status"],
    )
    .unwrap();
    assert!(!guarded.status.success());
    assert!(String::from_utf8_lossy(&guarded.stderr).contains("your home directory"));
}

#[test]
fn global_team_file_missing_error_names_both_paths() {
    let state = tempfile::tempdir().unwrap();
    let project_root = tempfile::tempdir().unwrap();
    let project = project_root.path().join("proj");
    std::fs::create_dir(&project).unwrap();
    let output = run_binary_in(state.path(), &project, &["team", "up"]).unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("a2amx.toml"));
    assert!(error.contains("config/team.toml"));
    assert!(!error.contains("start it"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_team_relative_cwd_uses_invoking_directory() {
    let (state, _daemon) = common::start_daemon().await;
    let project_root = tempfile::tempdir().unwrap();
    let project = project_root.path().join("proj");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(
        state.path().join("config/team.toml"),
        "[[session]]\nname = 'developer'\ncommand = ['sh', '-c', 'sleep 30']\ncwd = '.'",
    )
    .unwrap();
    let output = run_binary_in(state.path(), &project, &["team", "up", "--detach"]).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let listed = run_binary_in(state.path(), &project, &["list", "--details"]).unwrap();
    assert!(String::from_utf8_lossy(&listed.stdout).contains(&project.display().to_string()));
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_team_file_symlink_observes_canonical_current_dir() {
    let (state, _daemon) = common::start_daemon().await;
    let root = tempfile::tempdir().unwrap();
    let real = root.path().join("real-project");
    let link = root.path().join("project-link");
    std::fs::create_dir(&real).unwrap();
    std::fs::write(
        real.join("a2amx.toml"),
        "[[session]]\nname = 'developer'\ncommand = ['sh', '-c', 'sleep 30']\ncwd = '.'",
    )
    .unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let output = run_binary_in(state.path(), &link, &["team", "up", "--detach"]).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let listed = run_binary_in(state.path(), &link, &["list", "--details"]).unwrap();
    let details = String::from_utf8_lossy(&listed.stdout);
    // Linux current_dir() resolves a symlink before the CLI receives it.
    assert!(
        details.contains(&real.display().to_string()),
        "expected canonical project path {}, got:\n{details}",
        real.display()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_team_file_bypasses_global_fallback_and_guard() {
    let (state, _daemon) = common::start_daemon().await;
    let user_home = tempfile::tempdir().unwrap();
    std::fs::write(
        state.path().join("config/team.toml"),
        "[[session]]\nname = 'global'\ncommand = ['cat']",
    )
    .unwrap();
    let explicit = user_home.path().join("explicit.toml");
    std::fs::write(
        &explicit,
        "[[session]]\nname = 'explicit'\ncommand = ['cat']",
    )
    .unwrap();
    let explicit_path = explicit.to_string_lossy().into_owned();
    let output = run_binary_in_with_home(
        state.path(),
        user_home.path(),
        user_home.path(),
        &["team", "status", "--file", &explicit_path],
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("explicit"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("global"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("using global team file"));
    let missing = user_home.path().join("missing.toml");
    let missing_path = missing.to_string_lossy().into_owned();
    let output = run_binary_in_with_home(
        state.path(),
        user_home.path(),
        user_home.path(),
        &["team", "status", "--file", &missing_path],
    )
    .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains(&missing_path));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("using global team file"));
}

fn run_binary_in_with_home(
    home: &Path,
    cwd: &Path,
    user_home: &Path,
    args: &[&str],
) -> std::io::Result<Output> {
    Command::new(env!("CARGO_BIN_EXE_a2amx"))
        .arg("--home")
        .arg(home)
        .args(args)
        .current_dir(cwd)
        .env("HOME", user_home)
        .stdin(Stdio::null())
        .output()
}

fn run_binary_in(home: &Path, cwd: &Path, args: &[&str]) -> std::io::Result<Output> {
    Command::new(env!("CARGO_BIN_EXE_a2amx"))
        .arg("--home")
        .arg(home)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .output()
}
