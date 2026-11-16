use super::*;

#[test]
fn git_config_size_limit_accepts_65536_and_rejects_65537_bytes() {
    let state = tempfile::tempdir().expect("state");
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    std::fs::create_dir(project.path().join(".git")).expect("git directory");
    let remote = "[remote \"origin\"]\nurl = git@host-a:team/demo.git\n";
    let config_for_size = |size: usize| {
        let mut config = format!("{}{}", "# padding\n".repeat(32), remote);
        while config.len() + 2 <= size {
            config.push_str("#\n");
        }
        while config.len() < size {
            config.push('#');
        }
        assert_eq!(config.len(), size);
        config
    };

    for (size, expected) in [
        (
            65_536,
            "# Suggested grants:\ngrants = [\"ssh\", \"cache\"]\n",
        ),
        (65_537, "# Suggested grants:\ngrants = [\"cache\"]\n"),
    ] {
        std::fs::write(project.path().join(".git/config"), config_for_size(size))
            .expect("git config");
        assert_eq!(
            std::fs::metadata(project.path().join(".git/config"))
                .expect("git config metadata")
                .len(),
            size as u64
        );
        let output = run_binary(
            state.path(),
            project.path(),
            user_home.path(),
            runtime.path(),
            &[
                "confine",
                "suggest",
                project.path().to_str().expect("project path"),
            ],
        )
        .expect("suggest");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, expected.as_bytes());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_auto_rejects_missing_worktree_git_target() {
    let (state, _daemon) = start_daemon().await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    global_auto_file(state.path());
    std::fs::write(project.path().join(".git"), "gitdir: missing-meta\n")
        .expect("missing worktree git file");
    std::fs::write(
        project.path().join("config"),
        "[remote \"origin\"]\nurl = git@host-a:team/demo.git\n",
    )
    .expect("fallback git config bait");

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
    assert!(stdout.contains("  grants: cache\n"), "{stdout}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_auto_rejects_oversized_worktree_git_config() {
    let (state, _daemon) = start_daemon().await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    global_auto_file(state.path());
    let target = project.path().join("meta/worktree");
    std::fs::create_dir_all(&target).expect("worktree metadata");
    std::fs::write(target.join("HEAD"), "ref: refs/heads/main\n").expect("worktree HEAD");
    std::fs::create_dir(target.join("objects")).expect("worktree objects");
    std::fs::write(project.path().join(".git"), "gitdir: meta/worktree\n")
        .expect("worktree git file");
    std::fs::write(
        target.join("config"),
        format!(
            "[remote \"origin\"]\nurl = git@host-a:team/demo.git\n{}",
            "x".repeat(65_500)
        ),
    )
    .expect("oversized git config");

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
    assert!(stdout.contains("  grants: cache\n"), "{stdout}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_auto_rejects_worktree_git_config_directory() {
    let (state, _daemon) = start_daemon().await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    global_auto_file(state.path());
    let target = project.path().join("meta/worktree");
    std::fs::create_dir_all(target.join("config")).expect("config directory");
    std::fs::write(target.join("HEAD"), "ref: refs/heads/main\n").expect("worktree HEAD");
    std::fs::create_dir(target.join("objects")).expect("worktree objects");
    std::fs::write(project.path().join(".git"), "gitdir: meta/worktree\n")
        .expect("worktree git file");
    std::fs::write(
        project.path().join("meta/config"),
        "[remote \"origin\"]\nurl = git@host-a:team/demo.git\n",
    )
    .expect("sibling git config");

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
    assert!(stdout.contains("  grants: cache\n"), "{stdout}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_auto_rejects_worktree_git_config_symlink_to_directory() {
    let (state, _daemon) = start_daemon().await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    global_auto_file(state.path());
    let target = project.path().join("meta/worktree");
    std::fs::create_dir_all(target.join("config-directory")).expect("config directory");
    std::fs::write(target.join("HEAD"), "ref: refs/heads/main\n").expect("worktree HEAD");
    std::fs::create_dir(target.join("objects")).expect("worktree objects");
    std::fs::write(project.path().join(".git"), "gitdir: meta/worktree\n")
        .expect("worktree git file");
    std::os::unix::fs::symlink("config-directory", target.join("config"))
        .expect("config directory symlink");
    std::fs::write(
        project.path().join("meta/config"),
        "[remote \"origin\"]\nurl = git@host-a:team/demo.git\n",
    )
    .expect("sibling git config");

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
    assert!(stdout.contains("  grants: cache\n"), "{stdout}");
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_auto_rejects_worktree_git_config_fifo_without_hanging() {
    let (state, _daemon) = start_daemon().await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    global_auto_file(state.path());
    let target = project.path().join("meta/worktree");
    std::fs::create_dir_all(&target).expect("worktree metadata");
    std::fs::write(target.join("HEAD"), "ref: refs/heads/main\n").expect("worktree HEAD");
    std::fs::create_dir(target.join("objects")).expect("worktree objects");
    std::fs::write(project.path().join(".git"), "gitdir: meta/worktree\n")
        .expect("worktree git file");
    let status = AsyncCommand::new("mkfifo")
        .arg(target.join("config"))
        .status()
        .await
        .expect("mkfifo");
    assert!(status.success(), "mkfifo status: {status}");
    std::fs::write(
        project.path().join("meta/config"),
        "[remote \"origin\"]\nurl = git@host-a:team/demo.git\n",
    )
    .expect("sibling git config");

    let state_path = state.path().to_owned();
    let project_path = project.path().to_owned();
    let user_home_path = user_home.path().to_owned();
    let runtime_path = runtime.path().to_owned();
    let (sender, receiver) = std_mpsc::channel();
    std::thread::spawn(move || {
        let result = run_binary(
            &state_path,
            &project_path,
            &user_home_path,
            &runtime_path,
            &[
                "confine",
                "suggest",
                project_path.to_str().expect("project path"),
            ],
        );
        let _ = sender.send(result);
    });
    let output = receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("auto grant detection timed out")
        .expect("suggest");
    assert!(output.status.success());
    assert_eq!(
        output.stdout,
        b"# Suggested grants:\ngrants = [\"cache\"]\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_auto_rejects_gitdir_at_home() {
    let (state, _daemon) = start_daemon().await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    global_auto_file(state.path());
    std::fs::write(
        project.path().join(".git"),
        format!("gitdir: {}\n", user_home.path().display()),
    )
    .expect("home git file");
    std::fs::write(
        user_home.path().join("config"),
        "[remote \"origin\"]\nurl = git@host-a:team/demo.git\n",
    )
    .expect("home git config");

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
    assert!(stdout.contains("  grants: cache\n"), "{stdout}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_auto_ignores_worktree_git_files_and_oversized_configs() {
    let (state, _daemon) = start_daemon().await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    global_auto_file(state.path());
    let external = tempfile::tempdir().expect("external repository");
    let external_git = external.path().join(".git");
    std::fs::create_dir_all(&external_git).expect("external git directory");
    std::fs::write(external_git.join("HEAD"), "ref: refs/heads/main\n").expect("external HEAD");
    std::fs::create_dir(external_git.join("objects")).expect("external objects");
    std::fs::write(
        external_git.join("config"),
        format!(
            "[remote \"origin\"]\nurl = git@host-a:repo.git\n{}",
            "x".repeat(65_500)
        ),
    )
    .expect("external oversized config");
    std::fs::write(
        project.path().join(".git"),
        format!("gitdir: {}\n", external_git.display()),
    )
    .expect("worktree git file");

    let worktree = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("worktree dry run");
    assert!(worktree.status.success());
    let worktree_stdout = String::from_utf8_lossy(&worktree.stdout);
    let grants = worktree_stdout
        .lines()
        .find(|line| line.starts_with("  grants:"))
        .expect("grants line");
    assert_eq!(grants, "  grants: cache");

    std::fs::remove_file(project.path().join(".git")).expect("remove worktree marker");
    std::fs::create_dir_all(project.path().join(".git")).expect("git directory");
    std::fs::write(
        project.path().join(".git/config"),
        format!(
            "[remote \"origin\"]\nurl = git@host-a:repo.git\n{}",
            "x".repeat(65_500)
        ),
    )
    .expect("oversized config");
    let oversized = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("oversized config dry run");
    assert!(oversized.status.success());
    let oversized_stdout = String::from_utf8_lossy(&oversized.stdout);
    let grants = oversized_stdout
        .lines()
        .find(|line| line.starts_with("  grants:"))
        .expect("grants line");
    assert_eq!(grants, "  grants: cache");
}
