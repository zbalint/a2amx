use super::*;

fn run_binary_without_home(
    state: &Path,
    cwd: &Path,
    runtime: &Path,
    args: &[&str],
) -> std::io::Result<Output> {
    Command::new(env!("CARGO_BIN_EXE_a2amx"))
        .arg("--home")
        .arg(state)
        .args(args)
        .current_dir(cwd)
        .env_remove("HOME")
        .env("XDG_RUNTIME_DIR", runtime)
        .stdin(Stdio::null())
        .output()
}

fn write_worktree_git_file(project: &Path, config: &str) {
    std::fs::create_dir_all(project.join("meta/worktree")).expect("worktree metadata");
    std::fs::create_dir_all(project.join("common.git")).expect("common git directory");
    std::fs::write(project.join("common.git/HEAD"), "ref: refs/heads/main\n").expect("common HEAD");
    std::fs::create_dir(project.join("common.git/objects")).expect("common objects");
    std::fs::write(project.join(".git"), "gitdir: meta/worktree\n").expect("worktree git file");
    std::fs::write(
        project.join("meta/worktree/commondir"),
        "../../common.git\n",
    )
    .expect("commondir");
    std::fs::write(project.join("common.git/config"), config).expect("git config");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_auto_follows_worktree_git_file_for_ssh() {
    let (state, _daemon) = start_daemon().await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    global_auto_file(state.path());
    write_worktree_git_file(
        project.path(),
        "[remote \"origin\"]\nurl = git@host-a:team/demo.git\n",
    );

    let dry_run = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("dry run");
    assert!(
        dry_run.status.success(),
        "{}",
        String::from_utf8_lossy(&dry_run.stderr)
    );
    let dry_stdout = String::from_utf8_lossy(&dry_run.stdout);
    assert!(
        dry_stdout.contains("  grants: cache, ssh\n"),
        "{dry_stdout}"
    );

    let suggest = run_binary(
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
    assert!(suggest.status.success());
    assert_eq!(
        suggest.stdout,
        b"# Suggested grants:\ngrants = [\"ssh\", \"cache\"]\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_auto_follows_submodule_git_file_for_ssh() {
    let (state, _daemon) = start_daemon().await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    global_auto_file(state.path());
    std::fs::create_dir_all(project.path().join("submodule-meta")).expect("submodule git");
    std::fs::write(
        project.path().join("submodule-meta/HEAD"),
        "ref: refs/heads/main\n",
    )
    .expect("submodule HEAD");
    std::fs::create_dir(project.path().join("submodule-meta/objects")).expect("submodule objects");
    std::fs::write(project.path().join(".git"), "gitdir: submodule-meta\n")
        .expect("submodule git file");
    std::fs::write(
        project.path().join("submodule-meta/config"),
        "[remote \"origin\"]\nurl = git@host-a:team/demo.git\n",
    )
    .expect("git config");

    let dry_run = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("dry run");
    assert!(
        dry_run.status.success(),
        "{}",
        String::from_utf8_lossy(&dry_run.stderr)
    );
    let dry_stdout = String::from_utf8_lossy(&dry_run.stdout);
    assert!(
        dry_stdout.contains("  grants: cache, ssh\n"),
        "{dry_stdout}"
    );

    let suggest = run_binary(
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
    assert!(suggest.status.success());
    assert_eq!(
        suggest.stdout,
        b"# Suggested grants:\ngrants = [\"ssh\", \"cache\"]\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_auto_follows_repository_from_subdirectory() {
    let (state, _daemon) = start_daemon().await;
    let root = tempfile::tempdir().expect("repository");
    let subdirectory = root.path().join("sub");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    global_auto_file(state.path());
    std::fs::create_dir_all(root.path().join(".git")).expect("git directory");
    std::fs::create_dir(&subdirectory).expect("subdirectory");
    std::fs::write(
        root.path().join(".git/config"),
        "[remote \"origin\"]\nurl = git@host-a:team/demo.git\n",
    )
    .expect("git config");

    let dry_run = run_binary(
        state.path(),
        &subdirectory,
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("dry run");
    assert!(
        dry_run.status.success(),
        "{}",
        String::from_utf8_lossy(&dry_run.stderr)
    );
    let dry_stdout = String::from_utf8_lossy(&dry_run.stdout);
    assert!(
        dry_stdout.contains("  grants: cache, ssh\n"),
        "{dry_stdout}"
    );

    let suggest = run_binary(
        state.path(),
        &subdirectory,
        user_home.path(),
        runtime.path(),
        &[
            "confine",
            "suggest",
            subdirectory.to_str().expect("subdirectory path"),
        ],
    )
    .expect("suggest");
    assert!(suggest.status.success());
    assert_eq!(
        suggest.stdout,
        b"# Suggested grants:\ngrants = [\"ssh\", \"cache\"]\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_auto_follows_worktree_git_file_for_https_gh() {
    let (state, _daemon) = start_daemon().await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    global_auto_file(state.path());
    std::fs::create_dir_all(user_home.path().join(".config/gh")).expect("gh config");
    write_worktree_git_file(
        project.path(),
        "[remote \"origin\"]\nurl = https://host-a/team/demo.git\n",
    );

    let dry_run = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("dry run");
    assert!(
        dry_run.status.success(),
        "{}",
        String::from_utf8_lossy(&dry_run.stderr)
    );
    let dry_stdout = String::from_utf8_lossy(&dry_run.stdout);
    assert!(dry_stdout.contains("  grants: cache, gh\n"), "{dry_stdout}");

    let suggest = run_binary(
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
    assert!(suggest.status.success());
    assert_eq!(
        suggest.stdout,
        b"# Suggested grants:\ngrants = [\"gh\", \"cache\"]\n"
    );
}

#[test]
fn suggest_does_not_probe_cwd_for_host_paths_when_home_is_unset() {
    let state = tempfile::tempdir().expect("state");
    let project = tempfile::tempdir().expect("project");
    let runtime = tempfile::tempdir().expect("runtime");
    std::fs::create_dir_all(project.path().join(".config/gh")).expect("project gh marker");
    std::fs::create_dir_all(project.path().join(".git")).expect("git directory");
    std::fs::write(
        project.path().join(".git/config"),
        "[remote \"origin\"]\nurl = https://example.invalid/team/demo.git\n",
    )
    .expect("git config");

    let output = run_binary_without_home(
        state.path(),
        project.path(),
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
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("grants = [\"cache\"]"), "{stdout}");
    assert!(!stdout.contains("\"gh\""), "{stdout}");
}

#[test]
fn suggest_symlinked_home_repository_is_not_detected() {
    let state = tempfile::tempdir().expect("state");
    let home_parent = tempfile::tempdir().expect("home parent");
    let real_home = home_parent.path().join("real-home");
    let symlinked_home = home_parent.path().join("home-link");
    let project = symlinked_home.join("project");
    let runtime = tempfile::tempdir().expect("runtime");
    std::fs::create_dir_all(real_home.join(".git")).expect("home git directory");
    std::os::unix::fs::symlink(&real_home, &symlinked_home).expect("home symlink");
    std::fs::create_dir_all(&project).expect("project");
    std::fs::write(
        real_home.join(".git/config"),
        "[remote \"origin\"]\nurl = git@host-a:team/demo.git\n",
    )
    .expect("home git config");

    let output = run_binary(
        state.path(),
        &project,
        &symlinked_home,
        runtime.path(),
        &[
            "confine",
            "suggest",
            project.to_str().expect("project path"),
        ],
    )
    .expect("suggest");
    assert!(output.status.success());
    assert_eq!(
        output.stdout,
        b"# Suggested grants:\ngrants = [\"cache\"]\n"
    );
}

#[test]
fn suggest_follows_relative_gitdir_from_parent_repository() {
    let state = tempfile::tempdir().expect("state");
    let root = tempfile::tempdir().expect("repository");
    let subdirectory = root.path().join("sub");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    std::fs::create_dir_all(root.path().join("meta")).expect("git metadata");
    std::fs::write(root.path().join("meta/HEAD"), "ref: refs/heads/main\n").expect("git HEAD");
    std::fs::create_dir(root.path().join("meta/objects")).expect("git objects");
    std::fs::create_dir(&subdirectory).expect("subdirectory");
    std::fs::write(root.path().join(".git"), "gitdir: meta\n").expect("gitdir marker");
    std::fs::write(
        root.path().join("meta/config"),
        "[remote \"origin\"]\nurl = git@host-a:team/demo.git\n",
    )
    .expect("git config");

    let output = run_binary(
        state.path(),
        &subdirectory,
        user_home.path(),
        runtime.path(),
        &[
            "confine",
            "suggest",
            subdirectory.to_str().expect("subdirectory path"),
        ],
    )
    .expect("suggest");
    assert!(output.status.success());
    assert_eq!(
        output.stdout,
        b"# Suggested grants:\ngrants = [\"ssh\", \"cache\"]\n"
    );
}

#[test]
fn suggest_canonicalizes_relative_directory_before_git_walk() {
    let state = tempfile::tempdir().expect("state");
    let root = tempfile::tempdir().expect("repository parent");
    let repository = root.path().join("repository");
    let sibling = root.path().join("sibling");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    std::fs::create_dir_all(repository.join(".git")).expect("git directory");
    std::fs::create_dir(&sibling).expect("sibling");
    std::fs::write(
        repository.join(".git/config"),
        "[remote \"origin\"]\nurl = git@host-a:team/demo.git\n",
    )
    .expect("git config");

    let output = run_binary(
        state.path(),
        &repository,
        user_home.path(),
        runtime.path(),
        &["confine", "suggest", "../sibling"],
    )
    .expect("suggest");
    assert!(output.status.success());
    assert_eq!(
        output.stdout,
        b"# Suggested grants:\ngrants = [\"cache\"]\n"
    );
}
