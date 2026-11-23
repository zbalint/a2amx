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

#[test]
fn team_file_lookup_characterizes_exact_error_texts_and_global_notice() {
    let state = tempfile::tempdir().unwrap();
    let user_home = tempfile::tempdir().unwrap();
    let missing_file = user_home.path().join("missing.toml");
    let missing_arg = missing_file.to_str().unwrap();

    // `team up --file MISSING` fails at canonicalize before `read_team`'s `cannot read team file`.
    let up_missing = run_binary_in_with_home(
        state.path(),
        user_home.path(),
        user_home.path(),
        &["team", "up", "--file", missing_arg],
    )
    .unwrap();
    assert!(!up_missing.status.success());
    let up_err = String::from_utf8(up_missing.stderr).unwrap();
    assert_eq!(up_err, "a2amx: No such file or directory (os error 2)\n");

    // `team down --file MISSING` reaches `read_team`'s `cannot read team file`.
    let down_missing = run_binary_in_with_home(
        state.path(),
        user_home.path(),
        user_home.path(),
        &["team", "down", "--file", missing_arg],
    )
    .unwrap();
    assert!(!down_missing.status.success());
    assert_eq!(
        String::from_utf8(down_missing.stderr).unwrap(),
        format!(
            "a2amx: cannot read team file {}: No such file or directory (os error 2)\n",
            missing_file.display()
        )
    );

    // Missing parent directory fails at `cannot canonicalize team file directory`.
    let bad_parent = user_home.path().join("no-such-dir");
    let bad_file = bad_parent.join("a2amx.toml");
    let bad_dir_out = run_binary_in_with_home(
        state.path(),
        user_home.path(),
        user_home.path(),
        &["team", "up", "--file", bad_file.to_str().unwrap()],
    )
    .unwrap();
    assert!(!bad_dir_out.status.success());
    assert_eq!(
        String::from_utf8(bad_dir_out.stderr).unwrap(),
        format!(
            "a2amx: cannot canonicalize team file directory {}: No such file or directory (os error 2)\n",
            bad_parent.display()
        )
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn team_file_lookup_characterizes_global_notice_parse_and_eloop() {
    let state = tempfile::tempdir().unwrap();
    let _daemon = common::start_daemon_in(state.path(), Some("host-a"), Default::default()).await;
    let user_home = tempfile::tempdir().unwrap();
    let canonical_state = std::fs::canonicalize(state.path()).unwrap();
    let global_file = canonical_state.join("config/team.toml");
    std::fs::create_dir_all(global_file.parent().unwrap()).unwrap();

    // Global notice exact text when local a2amx.toml is absent and global team.toml is valid.
    std::fs::write(
        &global_file,
        "[[session]]\nname = 'worker'\ncommand = ['cat']\n",
    )
    .unwrap();
    let project = user_home.path().join("my-proj");
    std::fs::create_dir_all(&project).unwrap();
    let global_ok = run_binary_in_with_home(
        state.path(),
        &project,
        user_home.path(),
        &["team", "up", "--dry-run"],
    )
    .unwrap();
    assert!(global_ok.status.success(), "{global_ok:?}");
    assert_eq!(
        String::from_utf8(global_ok.stderr).unwrap(),
        format!(
            "using global team file {} as team my-proj\n",
            global_file.display()
        )
    );

    // Non-derivable cwd triggers team_name_for remedy text inside the global notice.
    let bad_cwd = user_home.path().join("---");
    std::fs::create_dir_all(&bad_cwd).unwrap();
    let bad_cwd_out = run_binary_in_with_home(
        state.path(),
        &bad_cwd,
        user_home.path(),
        &["team", "up", "--dry-run"],
    )
    .unwrap();
    assert!(!bad_cwd_out.status.success());
    assert_eq!(
        String::from_utf8(bad_cwd_out.stderr).unwrap(),
        format!(
            "a2amx: cannot derive a valid team name \"\" from directory {}: invalid name \"\": expected [a-z0-9][a-z0-9-]{{0,62}}; create an a2amx.toml here, or pass --file\n",
            bad_cwd.display()
        )
    );

    // team::parse rejects a global file that sets `team =`.
    std::fs::write(
        &global_file,
        "team = 'forbidden'\n[[session]]\nname = 'worker'\ncommand = ['cat']\n",
    )
    .unwrap();
    let global_with_team = run_binary_in_with_home(
        state.path(),
        &project,
        user_home.path(),
        &["team", "up", "--dry-run"],
    )
    .unwrap();
    assert!(!global_with_team.status.success());
    assert_eq!(
        String::from_utf8(global_with_team.stderr).unwrap(),
        format!(
            "using global team file {} as team my-proj\na2amx: team file {}: the global team file must not set team; the team is named after the directory\n",
            global_file.display(),
            global_file.display()
        )
    );

    // read_team's `team file PATH: ...` parse context on explicit --file.
    let bad_toml = user_home.path().join("bad.toml");
    std::fs::write(&bad_toml, "team = 'ok'\n").unwrap();
    let bad_parse = run_binary_in_with_home(
        state.path(),
        &project,
        user_home.path(),
        &["team", "up", "--file", bad_toml.to_str().unwrap()],
    )
    .unwrap();
    assert!(!bad_parse.status.success());
    assert_eq!(
        String::from_utf8(bad_parse.stderr).unwrap(),
        format!(
            "a2amx: team file {}: team must contain at least one session\n",
            bad_toml.display()
        )
    );

    // Self-referential symlink on global team.toml fails with ELOOP ("cannot inspect global team file").
    std::fs::remove_file(&global_file).unwrap();
    std::os::unix::fs::symlink("team.toml", &global_file).unwrap();
    let global_eloop = run_binary_in_with_home(
        state.path(),
        &project,
        user_home.path(),
        &["team", "up", "--dry-run"],
    )
    .unwrap();
    assert!(!global_eloop.status.success());
    assert_eq!(
        String::from_utf8(global_eloop.stderr).unwrap(),
        format!(
            "a2amx: cannot inspect global team file {}: Too many levels of symbolic links (os error 40)\n",
            global_file.display()
        )
    );

    // Self-referential symlink on cwd/a2amx.toml fails with ELOOP ("cannot inspect team file").
    let local_link = project.join("a2amx.toml");
    std::os::unix::fs::symlink("a2amx.toml", &local_link).unwrap();
    let local_eloop = run_binary_in_with_home(
        state.path(),
        &project,
        user_home.path(),
        &["team", "up", "--dry-run"],
    )
    .unwrap();
    assert!(!local_eloop.status.success());
    assert_eq!(
        String::from_utf8(local_eloop.stderr).unwrap(),
        format!(
            "a2amx: cannot inspect team file {}: Too many levels of symbolic links (os error 40)\n",
            local_link.display()
        )
    );
}
