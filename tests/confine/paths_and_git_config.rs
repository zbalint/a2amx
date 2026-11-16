use super::*;

#[test]
fn canonical_paths_cover_symlinked_workspace_and_hide_targets() {
    let fixture = Fixture::new();
    let cwd_link = fixture.cwd.with_file_name("workspace-link");
    symlink(&fixture.cwd, &cwd_link).expect("cwd symlink");
    let hide_target = fixture.cwd.with_file_name("hide-target");
    std::fs::create_dir_all(&hide_target).expect("hide target");
    let hide_link = fixture.cwd.with_file_name("hide-link");
    symlink(&hide_target, &hide_link).expect("hide symlink");
    let state_link = fixture.cwd.with_file_name("state-link");
    symlink(&fixture.state, &state_link).expect("state symlink");
    assert!(
        confine::check_write_allowed(&state_link, &fixture.home, &fixture.state, false).is_err()
    );
    let runtime_link = fixture.cwd.with_file_name("runtime-link");
    symlink(&fixture.runtime, &runtime_link).expect("runtime symlink");
    assert!(
        confine::plan(
            &ConfineRequest {
                write: vec![runtime_link.to_string_lossy().into_owned()],
                ..ConfineRequest::default()
            },
            Harness::Generic,
            &fixture.cwd,
            &fixture.host()
        )
        .is_err()
    );
    let plan = confine::plan(
        &ConfineRequest {
            hide: vec![hide_link.to_string_lossy().into_owned()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &cwd_link,
        &fixture.host(),
    )
    .expect("plan");
    assert_eq!(
        plan.cwd,
        std::fs::canonicalize(&fixture.cwd).expect("cwd canonical")
    );
    assert!(
        plan.hidden_paths()
            .contains(&std::fs::canonicalize(hide_link).expect("hide canonical"))
    );
}

#[test]
fn git_common_directory_and_exact_writes_override_protection() {
    let fixture = Fixture::new();
    let repo = fixture.cwd.join("repo");
    let nested = repo.join("src");
    let git = repo.join(".git");
    let git_config = git.join("config");
    let team_file = nested.join("config/team.toml");
    std::fs::create_dir_all(&git).expect("git directory");
    std::fs::create_dir_all(&nested).expect("repo subdirectory");
    std::fs::create_dir_all(team_file.parent().expect("team parent"))
        .expect("team config directory");
    std::fs::write(&git_config, "[core]\n").expect("git config");
    std::fs::write(&team_file, "[[session]]\nname = \"worker\"\n").expect("team file");

    let plan = confine::plan(
        &ConfineRequest {
            write: vec![git_config.to_string_lossy().into_owned()],
            team_file: Some(team_file.to_string_lossy().into_owned()),
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &nested,
        &fixture.host(),
    )
    .expect("plan");
    let repo_git = std::fs::canonicalize(&git).expect("git canonical");
    let config = std::fs::canonicalize(&git_config).expect("config canonical");
    let team = std::fs::canonicalize(&team_file).expect("team canonical");
    let repo = std::fs::canonicalize(&repo).expect("repo canonical");
    assert!(plan.writable.contains(&repo_git));
    assert!(plan.writable.contains(&config));
    assert!(!plan.writable.contains(&repo));
    assert!(!plan.readonly.contains(&config));
    assert!(!plan.protected.contains(&config));
    assert!(plan.protected.contains(&team));
    let wrapped = plan
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("wrapped plan");
    let writable_index = wrapped
        .windows(3)
        .position(|window| {
            window
                == [
                    "--bind",
                    nested.to_string_lossy().as_ref(),
                    nested.to_string_lossy().as_ref(),
                ]
        })
        .expect("cwd writable bind");
    let readonly_index = wrapped
        .windows(3)
        .position(|window| {
            window
                == [
                    "--ro-bind",
                    team.to_string_lossy().as_ref(),
                    team.to_string_lossy().as_ref(),
                ]
        })
        .expect("team protected bind");
    assert!(writable_index < readonly_index);
}
#[test]
fn exact_writes_replace_default_hides_and_equal_depth_hides_precede_binds() {
    let fixture = Fixture::new();
    let hidden = fixture.home.join("operator-hidden");
    let writable = fixture.home.join("operator-writable");
    std::fs::create_dir_all(&hidden).expect("operator hidden directory");
    std::fs::create_dir_all(&writable).expect("operator writable directory");
    assert_eq!(hidden.components().count(), writable.components().count());

    let plan = confine::plan(
        &ConfineRequest {
            write: vec!["~/.aws".into(), writable.to_string_lossy().into_owned()],
            hide: vec![hidden.to_string_lossy().into_owned()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("plan");
    let wrapped = plan
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("wrapped plan");

    let aws = fixture.home.join(".aws").to_string_lossy().into_owned();
    assert_eq!(
        wrapped
            .windows(3)
            .filter(|window| *window == ["--bind", aws.as_str(), aws.as_str()])
            .count(),
        1,
        "exact writable default-hide override must leave one bind"
    );
    assert!(
        !wrapped
            .windows(2)
            .any(|window| window == ["--tmpfs", aws.as_str()]),
        "exact writable default-hide override must remove the hide"
    );

    let hidden = hidden.to_string_lossy().into_owned();
    let writable = writable.to_string_lossy().into_owned();
    let hide_index = wrapped
        .windows(2)
        .position(|window| window == ["--tmpfs", hidden.as_str()])
        .expect("operator hide mount");
    let bind_index = wrapped
        .windows(3)
        .position(|window| window == ["--bind", writable.as_str(), writable.as_str()])
        .expect("operator writable mount");
    assert!(
        hide_index < bind_index,
        "equal-depth hides must precede binds in the literal argv"
    );
}

#[test]
fn git_hooks_and_core_hooks_path_are_protected_inside_writable_repo() {
    let fixture = Fixture::new();
    let repo = fixture.cwd.join("repo");
    let git = repo.join(".git");
    let git_hooks = git.join("hooks");
    let husky = repo.join(".husky");
    std::fs::create_dir_all(&git_hooks).expect("git hooks");
    std::fs::create_dir_all(&husky).expect("husky hooks");
    std::fs::write(git.join("config"), "[core]\nhooksPath = .husky\n").expect("git config");

    let plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &repo,
        &fixture.host(),
    )
    .expect("plan");
    assert!(
        plan.writable
            .contains(&std::fs::canonicalize(&repo).expect("repo canonical"))
    );
    assert!(
        plan.protected
            .contains(&std::fs::canonicalize(git_hooks).expect("git hooks canonical"))
    );
    assert!(
        plan.protected
            .contains(&std::fs::canonicalize(git.join("config")).expect("config canonical"))
    );
    assert!(
        plan.protected
            .contains(&std::fs::canonicalize(husky).expect("husky canonical"))
    );
}

fn plan_with_timeout(repo: PathBuf, host: Host) -> anyhow::Result<confine::Plan> {
    let (sender, receiver) = std_mpsc::channel();
    std::thread::spawn(move || {
        let result = confine::plan(&ConfineRequest::default(), Harness::Generic, &repo, &host);
        let _ = sender.send(result);
    });
    receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("plan timed out")
}

#[test]
fn plan_does_not_hang_on_fifo_git_config() {
    let fixture = Fixture::new();
    let repo = fixture.cwd.join("repo");
    let git = repo.join(".git");
    let git_hooks = git.join("hooks");
    std::fs::create_dir_all(&git_hooks).expect("git hooks");
    let status = Command::new("mkfifo")
        .arg(git.join("config"))
        .status()
        .expect("mkfifo");
    assert!(status.success(), "mkfifo status: {status}");

    let plan = plan_with_timeout(repo, fixture.host()).expect("plan");
    assert!(
        plan.protected
            .contains(&std::fs::canonicalize(git.join("config")).expect("config canonical"))
    );
    assert!(
        plan.protected
            .contains(&std::fs::canonicalize(git_hooks).expect("git hooks canonical"))
    );
}

#[test]
fn plan_rejects_oversized_git_config() {
    let limit = 1024 * 1024;
    let fixture = Fixture::new();
    let repo = fixture.cwd.join("repo");
    let git = repo.join(".git");
    std::fs::create_dir_all(git.join("hooks")).expect("git hooks");
    std::fs::write(git.join("config"), vec![b'x'; limit + 1]).expect("oversized git config");
    let error = match plan_with_timeout(repo, fixture.host()) {
        Ok(_) => panic!("oversized config accepted"),
        Err(error) => error,
    };
    let text = error.to_string();
    assert!(text.contains("larger than"), "unexpected error: {text}");
    assert!(text.contains("hooksPath"), "unexpected error: {text}");

    let fixture = Fixture::new();
    let repo = fixture.cwd.join("repo");
    let git = repo.join(".git");
    let husky = repo.join(".husky");
    std::fs::create_dir_all(git.join("hooks")).expect("git hooks");
    std::fs::create_dir_all(&husky).expect("husky hooks");
    let prefix = "[user]\n\tpad = ";
    let suffix = "\n[core]\n\thooksPath = .husky\n";
    let padding = limit - prefix.len() - suffix.len();
    let config = format!("{prefix}{}{suffix}", "a".repeat(padding));
    assert_eq!(config.len(), limit);
    std::fs::write(git.join("config"), config).expect("exact-limit git config");
    let plan = plan_with_timeout(repo, fixture.host()).expect("exact-limit plan");
    assert!(
        plan.protected
            .contains(&std::fs::canonicalize(husky).expect("husky canonical"))
    );
}

#[test]
fn plan_protects_hooks_path_in_config_with_invalid_utf8() {
    let fixture = Fixture::new();
    let repo = fixture.cwd.join("repo");
    let git = repo.join(".git");
    let husky = repo.join(".husky");
    std::fs::create_dir_all(git.join("hooks")).expect("git hooks");
    std::fs::create_dir_all(&husky).expect("husky hooks");
    std::fs::write(
        git.join("config"),
        b"[user]\n\tname = \xe9\n[core]\n\thooksPath = .husky\n",
    )
    .expect("git config");

    let plan = plan_with_timeout(repo, fixture.host()).expect("plan");
    assert!(
        plan.protected
            .contains(&std::fs::canonicalize(husky).expect("husky canonical"))
    );
}

#[test]
fn plan_ignores_directory_git_config() {
    let fixture = Fixture::new();
    let repo = fixture.cwd.join("repo");
    let git = repo.join(".git");
    let git_hooks = git.join("hooks");
    std::fs::create_dir_all(&git_hooks).expect("git hooks");
    std::fs::create_dir(git.join("config")).expect("config directory");

    let plan = plan_with_timeout(repo, fixture.host()).expect("plan");
    assert!(
        plan.protected
            .contains(&std::fs::canonicalize(git.join("config")).expect("config canonical"))
    );
    assert!(
        plan.protected
            .contains(&std::fs::canonicalize(git_hooks).expect("git hooks canonical"))
    );
}

#[test]
fn plan_finds_hooks_path_after_lines_without_equals() {
    let fixture = Fixture::new();
    let repo = fixture.cwd.join("repo");
    let git = repo.join(".git");
    let husky = repo.join(".husky");
    std::fs::create_dir_all(git.join("hooks")).expect("git hooks");
    std::fs::create_dir_all(&husky).expect("husky hooks");
    std::fs::write(
        git.join("config"),
        "[core]\n\n\tbare\n# note\n\thooksPath = .husky\n",
    )
    .expect("git config");

    let plan = plan_with_timeout(repo, fixture.host()).expect("plan");
    assert!(
        plan.protected
            .contains(&std::fs::canonicalize(husky).expect("husky canonical"))
    );
}
