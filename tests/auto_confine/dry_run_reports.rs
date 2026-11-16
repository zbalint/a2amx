use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_auto_dry_run_reports_detected_grants_once() {
    let (state, _daemon) = start_daemon().await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    global_auto_file(state.path());
    std::fs::write(
        project.path().join("Cargo.toml"),
        "[package]\nname='demo'\n",
    )
    .expect("cargo marker");
    std::fs::create_dir_all(project.path().join(".git")).expect("git directory");
    std::fs::write(
        project.path().join(".git/config"),
        "[remote \"origin\"]\nurl = git@host-a:team/demo.git\n",
    )
    .expect("git config");

    let output = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("dry run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("grants: cache, cargo, ssh"), "{stdout}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("auto confinement for session")
            && stderr.contains("grants ssh, cargo, cache"),
        "{stderr}"
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dry_run_reports_unaccepted_grants_and_respects_seeded_file() {
    let (state, _daemon) = start_daemon().await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    global_auto_file(state.path());
    std::fs::write(project.path().join("Cargo.toml"), "").expect("cargo marker");

    let output = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("dry run");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("  cargo: Cargo.toml found\n"), "{stdout}");
    assert!(
        stdout.contains("  cache: always added by auto\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("  not yet accepted: cargo, cache\n"),
        "{stdout}"
    );
    assert!(!state.path().join("state/auto-grants.toml").exists());

    std::fs::write(
        state.path().join("state/auto-grants.toml"),
        format!(
            "[projects.{:?}]\ngrants = [\"unknown\", \"cache\", \"cargo\"]\n",
            project.path().to_string_lossy()
        ),
    )
    .expect("seed accepted grants");
    let seeded = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("seeded dry run");
    assert!(seeded.status.success());
    assert!(
        !String::from_utf8_lossy(&seeded.stdout).contains("not yet accepted"),
        "{}",
        String::from_utf8_lossy(&seeded.stdout)
    );
    std::fs::write(
        state.path().join("state/auto-grants.toml"),
        "x".repeat(65_537),
    )
    .expect("oversized accepted grants");
    let oversized = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("oversized dry run");
    assert!(!oversized.status.success());
    assert!(
        String::from_utf8_lossy(&oversized.stderr).contains(&format!(
            "accepted auto-grants.toml file {} is larger than 65536 bytes",
            state.path().join("state/auto-grants.toml").display()
        ))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepted_auto_grants_file_validates_shape_size_and_fifo() {
    let (state, _daemon) = start_daemon().await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    global_auto_file(state.path());
    std::fs::write(project.path().join("Cargo.toml"), "").expect("cargo marker");
    let accepted = state.path().join("state/auto-grants.toml");
    let base = format!(
        "[projects.{:?}]\ngrants = [\"cache\"]\n",
        project.path().to_string_lossy()
    );
    let exact = format!("{base}{}", "#".repeat(65_536 - base.len()));
    assert_eq!(exact.len(), 65_536);
    std::fs::write(&accepted, exact).expect("exact-size accepted grants");
    let valid = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("exact-size dry run");
    assert!(
        valid.status.success(),
        "{}",
        String::from_utf8_lossy(&valid.stderr)
    );
    std::fs::write(&accepted, "[projects.\"/p\"]\ngrants = \"cache\"\n")
        .expect("invalid accepted grants");
    let invalid = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("invalid dry run");
    assert_eq!(invalid.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&invalid.stderr).contains(&accepted.display().to_string()));
    let invalid_normal = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up"],
    )
    .expect("invalid normal run");
    assert_eq!(invalid_normal.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&invalid_normal.stderr).contains(&accepted.display().to_string())
    );

    std::fs::remove_file(&accepted).expect("remove accepted grants");
    let fifo = AsyncCommand::new("mkfifo")
        .arg(&accepted)
        .status()
        .await
        .expect("mkfifo");
    assert!(fifo.success(), "mkfifo status: {fifo}");
    let state_path = state.path().to_owned();
    let project_path = project.path().to_owned();
    let home_path = user_home.path().to_owned();
    let runtime_path = runtime.path().to_owned();
    let fifo_result = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::task::spawn_blocking(move || {
            run_binary(
                &state_path,
                &project_path,
                &home_path,
                &runtime_path,
                &["team", "up", "--dry-run"],
            )
        }),
    )
    .await
    .expect("FIFO check hung")
    .expect("FIFO check task");
    let fifo_output = fifo_result.expect("FIFO dry run");
    assert!(!fifo_output.status.success());
    assert!(String::from_utf8_lossy(&fifo_output.stderr).contains(&accepted.display().to_string()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn non_utf8_project_directory_is_rejected_by_dry_run() {
    let (state, _daemon) = start_daemon().await;
    let root = tempfile::tempdir().expect("root");
    let bad_name = std::ffi::OsString::from_vec(vec![b'p', 0x80]);
    let bad = root.path().join(&bad_name);
    std::fs::create_dir(&bad).expect("non-UTF-8 project");
    std::fs::write(bad.join("Cargo.toml"), "").expect("cargo marker");
    std::fs::write(
        bad.join("a2amx.toml"),
        "[confine]\nenabled = true\nauto = true\n[[session]]\nname = \"worker\"\ncommand = [\"cat\"]\n",
    )
    .expect("local team file");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    let output = run_binary(
        state.path(),
        &bad,
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("non-UTF-8 dry run");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("project directory is not valid UTF-8"));
    assert!(stderr.contains(&format!("{:?}", bad.as_os_str())));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn symlinked_accepted_file_is_read_and_trailing_slash_key_does_not_match() {
    let (state, _daemon) = start_daemon().await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    global_auto_file(state.path());
    std::fs::write(project.path().join("Cargo.toml"), "").expect("cargo marker");
    let accepted = state.path().join("state/auto-grants.toml");
    let target = state.path().join("accepted-real.toml");
    std::fs::write(
        &target,
        format!(
            "[projects.{:?}]\ngrants = [\"cache\", \"cargo\"]\n",
            project.path().to_string_lossy()
        ),
    )
    .expect("real accepted file");
    std::os::unix::fs::symlink(&target, &accepted).expect("accepted symlink");
    let honored = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("symlink dry run");
    assert!(honored.status.success());
    assert!(!String::from_utf8_lossy(&honored.stdout).contains("not yet accepted"));
    std::fs::write(
        &target,
        format!(
            "[projects.{:?}]\ngrants = [\"cache\", \"cargo\"]\n",
            format!("{}/", project.path().display())
        ),
    )
    .expect("trailing slash accepted key");
    let noncanonical = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("noncanonical dry run");
    assert!(noncanonical.status.success());
    assert!(String::from_utf8_lossy(&noncanonical.stdout).contains("not yet accepted"));
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_merge_preserves_explicit_grant_and_reports_added_reasons() {
    let (state, _daemon) = start_daemon().await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    std::fs::write(
        state.path().join("config/team.toml"),
        "[confine]\nenabled = true\nauto = true\n[[session]]\nname = \"worker\"\ncommand = [\"cat\"]\n[session.confine]\ngrants = [\"cargo\"]\n",
    )
    .expect("global team file");
    std::fs::write(
        project.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\n",
    )
    .expect("cargo marker");
    std::fs::create_dir_all(project.path().join(".git")).expect("git directory");
    std::fs::write(
        project.path().join(".git/config"),
        "[remote \"origin\"]\nurl = git@host-a:team/demo.git\n",
    )
    .expect("git config");

    let output = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("dry run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("  confinement: auto\n"), "{stdout}");
    assert!(stdout.contains("  grants: cache, cargo, ssh\n"), "{stdout}");
    assert!(stdout.contains("  ssh: git remote uses ssh\n"), "{stdout}");
    assert!(
        stdout.contains("  cache: always added by auto\n"),
        "{stdout}"
    );
    assert!(!stdout.contains("  cargo: "), "{stdout}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_container_grant_reports_escape_notice_in_dry_run() {
    let (state, _daemon) = start_daemon().await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    std::fs::write(
        project.path().join("a2amx.toml"),
        "team = \"worker-test\"\n[confine]\nenabled = true\n[[session]]\nname = \"worker\"\ncommand = [\"cat\"]\n[session.confine]\ngrants = [\"docker\"]\n",
    )
    .expect("local team file");

    let output = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("dry run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("session worker-test-worker: grant docker can escape: "),
        "{stderr}"
    );

    std::fs::write(
        project.path().join("a2amx.toml"),
        "team = \"worker-test\"\n[confine]\nenabled = true\n[[session]]\nname = \"worker\"\ncommand = [\"cat\"]\n[session.confine]\ngrants = [\"cargo\"]\n",
    )
    .expect("local team file");
    let output = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("dry run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("can escape"), "{stderr}");
}
