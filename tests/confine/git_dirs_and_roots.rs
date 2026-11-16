use super::*;

#[test]
fn git_worktree_and_submodule_common_dirs_are_bound() {
    let fixture = Fixture::new();

    let worktree = fixture.cwd.join("worktree");
    let metadata = fixture.cwd.join("meta/worktree");
    let common = fixture.cwd.join("common.git");
    std::fs::create_dir_all(&worktree).expect("worktree");
    std::fs::create_dir_all(&metadata).expect("worktree metadata");
    std::fs::create_dir_all(&common).expect("common git");
    std::fs::write(common.join("HEAD"), "ref: refs/heads/main\n").expect("common HEAD");
    std::fs::create_dir(common.join("objects")).expect("common objects");
    std::fs::write(worktree.join(".git"), "gitdir: ../meta/worktree\n").expect("gitdir marker");
    std::fs::write(metadata.join("commondir"), "../../common.git\n").expect("commondir");
    let worktree_plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &worktree,
        &fixture.host(),
    )
    .expect("worktree plan");
    assert!(
        worktree_plan
            .writable
            .contains(&std::fs::canonicalize(&common).expect("common canonical"))
    );

    let submodule = fixture.cwd.join("submodule");
    let submodule_git = fixture.cwd.join("submodule-meta");
    std::fs::create_dir_all(&submodule).expect("submodule");
    std::fs::create_dir_all(&submodule_git).expect("submodule git");
    std::fs::write(submodule_git.join("HEAD"), "ref: refs/heads/main\n").expect("submodule HEAD");
    std::fs::create_dir(submodule_git.join("objects")).expect("submodule objects");
    std::fs::write(submodule.join(".git"), "gitdir: ../submodule-meta\n")
        .expect("submodule marker");
    let submodule_plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &submodule,
        &fixture.host(),
    )
    .expect("submodule plan");
    assert!(
        submodule_plan
            .writable
            .contains(&std::fs::canonicalize(&submodule_git).expect("submodule canonical"))
    );
}
#[test]
fn implicit_git_writes_inside_default_hides_are_refused() {
    let fixture = Fixture::new();
    let hidden_git = fixture.home.join(".ssh");
    std::fs::write(hidden_git.join("HEAD"), "ref: refs/heads/main\n").expect("git HEAD");
    std::fs::create_dir(hidden_git.join("objects")).expect("git objects");

    let direct = fixture.cwd.join("direct");
    std::fs::create_dir_all(&direct).expect("direct repository");
    std::fs::write(
        direct.join(".git"),
        format!("gitdir: {}\n", hidden_git.display()),
    )
    .expect("direct git pointer");
    let error = match confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &direct,
        &fixture.host(),
    ) {
        Ok(_) => panic!("git directory inside a default hide was writable"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("git directory"));
    assert!(error.to_string().contains("resolves to the hidden path"));

    let metadata = fixture.cwd.join("metadata");
    let commondir_repo = fixture.cwd.join("commondir-repo");
    std::fs::create_dir_all(&metadata).expect("metadata directory");
    std::fs::create_dir_all(&commondir_repo).expect("commondir repository");
    std::fs::write(
        commondir_repo.join(".git"),
        format!("gitdir: {}\n", metadata.display()),
    )
    .expect("commondir git pointer");
    std::fs::write(
        metadata.join("commondir"),
        format!("{}\n", hidden_git.display()),
    )
    .expect("commondir pointer");
    let error = match confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &commondir_repo,
        &fixture.host(),
    ) {
        Ok(_) => panic!("commondir inside a default hide was writable"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("git directory"));
    assert!(error.to_string().contains("resolves to the hidden path"));

    let explicit = confine::plan(
        &ConfineRequest {
            write: vec![hidden_git.to_string_lossy().into_owned()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &direct,
        &fixture.host(),
    )
    .expect("explicit git write");
    assert!(
        explicit
            .writable
            .contains(&hidden_git.canonicalize().expect("hidden git"))
    );

    let no_default_hides = confine::plan(
        &ConfineRequest {
            no_default_hides: true,
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &direct,
        &fixture.host(),
    )
    .expect("git write with default hides disabled");
    assert!(
        no_default_hides
            .writable
            .contains(&hidden_git.canonicalize().expect("hidden git"))
    );
}

#[test]
fn harness_data_links_stay_inside_expected_locations() {
    let hidden_dir = Fixture::new();
    let hidden_root = hidden_dir.home.join(".claude");
    std::fs::create_dir_all(&hidden_root).expect("Claude root");
    symlink(hidden_dir.home.join(".ssh"), hidden_root.join("todos")).expect("hidden todos symlink");
    let error = match confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &hidden_dir.cwd,
        &hidden_dir.host(),
    ) {
        Ok(_) => panic!("harness data symlink escaped into a hidden directory"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("harness data path"));
    assert!(error.to_string().contains("outside"));

    let hidden_file = Fixture::new();
    let hidden_file_root = hidden_file.home.join(".claude");
    std::fs::create_dir_all(&hidden_file_root).expect("Claude root");
    std::fs::write(hidden_file.home.join(".netrc"), "credentials").expect("netrc");
    symlink(
        hidden_file.home.join(".netrc"),
        hidden_file_root.join("history.jsonl"),
    )
    .expect("hidden history symlink");
    let error = match confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &hidden_file.cwd,
        &hidden_file.host(),
    ) {
        Ok(_) => panic!("harness data file escaped into a hidden file"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("harness data path"));
    assert!(error.to_string().contains("outside"));

    let outside = Fixture::new();
    let outside_root = outside.home.join(".claude");
    std::fs::create_dir_all(&outside_root).expect("Claude root");
    std::fs::create_dir_all(outside.home.join(".config")).expect("config");
    symlink(outside.home.join(".config"), outside_root.join("todos"))
        .expect("outside todos symlink");
    let error = match confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &outside.cwd,
        &outside.host(),
    ) {
        Ok(_) => panic!("harness data symlink escaped its root"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("harness data path"));
    assert!(error.to_string().contains("outside"));

    let explicit_config = Fixture::new();
    let explicit_config_root = explicit_config.home.join(".claude");
    std::fs::create_dir_all(&explicit_config_root).expect("Claude root");
    std::fs::create_dir_all(explicit_config.home.join(".config")).expect("config");
    symlink(
        explicit_config.home.join(".config"),
        explicit_config_root.join("todos"),
    )
    .expect("explicit config symlink");
    let explicit_config_plan = confine::plan(
        &ConfineRequest {
            write: vec![
                explicit_config
                    .home
                    .join(".config")
                    .to_string_lossy()
                    .into_owned(),
            ],
            ..ConfineRequest::default()
        },
        Harness::Claude,
        &explicit_config.cwd,
        &explicit_config.host(),
    )
    .expect("explicit harness config target");
    assert!(
        explicit_config_plan.writable.contains(
            &explicit_config
                .home
                .join(".config")
                .canonicalize()
                .expect("config canonical")
        )
    );

    let explicit = Fixture::new();
    let symlinked_root = Fixture::new();
    let dotfiles_root = symlinked_root.home.join("dotfiles/claude");
    std::fs::create_dir_all(dotfiles_root.join("todos")).expect("dotfiles Claude root");
    symlink(&dotfiles_root, symlinked_root.home.join(".claude")).expect("dotfiles Claude symlink");
    let symlinked_plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &symlinked_root.cwd,
        &symlinked_root.host(),
    )
    .expect("symlinked Claude root");
    assert!(
        symlinked_plan
            .writable
            .contains(&dotfiles_root.canonicalize().expect("dotfiles canonical"))
    );

    let explicit_root = explicit.home.join(".claude");
    std::fs::create_dir_all(&explicit_root).expect("Claude root");
    symlink(explicit.home.join(".ssh"), explicit_root.join("todos"))
        .expect("explicit todos symlink");
    let explicit_plan = confine::plan(
        &ConfineRequest {
            write: vec![explicit.home.join(".ssh").to_string_lossy().into_owned()],
            ..ConfineRequest::default()
        },
        Harness::Claude,
        &explicit.cwd,
        &explicit.host(),
    )
    .expect("explicit harness data target");
    assert!(
        explicit_plan.writable.contains(
            &explicit
                .home
                .join(".ssh")
                .canonicalize()
                .expect("SSH canonical")
        )
    );

    let in_root = Fixture::new();
    let in_root_claude = in_root.home.join(".claude");
    let ide = in_root_claude.join("ide");
    std::fs::create_dir_all(&ide).expect("Claude IDE directory");
    symlink(&ide, in_root_claude.join("todos")).expect("in-root todos symlink");
    let in_root_plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &in_root.cwd,
        &in_root.host(),
    )
    .expect("in-root harness data target");
    let ide = ide.canonicalize().expect("IDE canonical");
    assert!(in_root_plan.hidden_paths().contains(&ide));
    assert!(!in_root_plan.writable.contains(&ide));

    let home_file = Fixture::new();
    std::fs::write(home_file.home.join(".claude.json"), "{}").expect("Claude home file");
    confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &home_file.cwd,
        &home_file.host(),
    )
    .expect("Claude home data file");
}

#[test]
fn symlinked_home_with_claude_home_file_is_accepted() {
    let fixture = Fixture::new();
    let real_home = fixture._root.path().join("real-home");
    std::fs::rename(&fixture.home, &real_home).expect("real home");
    symlink(&real_home, &fixture.home).expect("home symlink");
    std::fs::write(real_home.join(".claude.json"), "{}").expect("Claude home file");

    confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("symlinked home with Claude home file");
}

#[test]
fn implicit_workspace_writes_inside_default_hides_are_refused() {
    let fixture = Fixture::new();
    let cwd = fixture.home.join(".password-store/sub");
    std::fs::create_dir_all(&cwd).expect("password store cwd");
    let error = match confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &cwd,
        &fixture.host(),
    ) {
        Ok(_) => panic!("workspace inside a default hide was writable"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("working directory"));
    assert!(error.to_string().contains("resolves to the hidden path"));

    let combined = Fixture::new();
    let combined_cwd = combined.home.join(".password-store/sub");
    let combined_git = combined_cwd.join(".git");
    std::fs::create_dir_all(&combined_git).expect("password store git directory");
    std::fs::write(combined_git.join("config"), "[core]\n").expect("git config");
    let error = match confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &combined_cwd,
        &combined.host(),
    ) {
        Ok(_) => panic!("workspace and git directory inside a default hide were writable"),
        Err(error) => error,
    };
    let text = error.to_string();
    assert!(text.contains("working directory"));
    assert!(text.contains("git directory"));

    confine::plan(
        &ConfineRequest {
            write: vec![
                combined_cwd.to_string_lossy().into_owned(),
                combined_git.to_string_lossy().into_owned(),
            ],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &combined_cwd,
        &combined.host(),
    )
    .expect("explicit workspace and git writes");
}

#[test]
fn explicit_writes_inside_a2amx_root_are_rejected() {
    let fixture = Fixture::new();
    for path in [
        fixture.host().layout.log_dir(),
        fixture.host().layout.config_dir(),
    ] {
        let error = match confine::plan(
            &ConfineRequest {
                write: vec![path.to_string_lossy().into_owned()],
                ..ConfineRequest::default()
            },
            Harness::Generic,
            &fixture.cwd,
            &fixture.host(),
        ) {
            Ok(_) => panic!("a2amx root write accepted"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("overlaps the a2amx directory"));
    }
}

#[test]
fn plan_layout_uses_canonical_root() {
    let fixture = Fixture::new();
    let outer = tempfile::tempdir().expect("outer");
    let actual = outer.path().join("actual");
    std::fs::create_dir_all(&actual).expect("actual");
    let alias = outer.path().join("alias");
    symlink(&actual, &alias).expect("alias");
    let layout = a2amx::layout::Layout::new(alias);
    layout.ensure().expect("layout");
    let host = Host {
        home: fixture.home.clone(),
        layout,
        runtime_dir: fixture.runtime.clone(),
    };
    let plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &fixture.cwd,
        &host,
    )
    .expect("plan");
    assert_eq!(plan.layout().root(), actual);
}

#[test]
fn writable_ancestor_of_a2amx_root_protects_root() {
    let fixture = Fixture::new();
    let outer = tempfile::tempdir().expect("outer");
    let writable = outer.path().join("work");
    let root = writable.join("a2amx");
    std::fs::create_dir_all(&root).expect("root");
    let layout = a2amx::layout::Layout::new(root);
    layout.ensure().expect("layout");
    let host = Host {
        home: fixture.home.clone(),
        layout,
        runtime_dir: fixture.runtime.clone(),
    };
    let plan = confine::plan(
        &ConfineRequest {
            write: vec![writable.to_string_lossy().into_owned()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &host,
    )
    .expect("writable ancestor accepted with protected root");
    assert!(plan.writable.contains(&writable));
    assert!(
        plan.readonly
            .contains(&host.layout.root().canonicalize().expect("canonical root"))
    );
}

#[test]
fn git_common_dir_pointer_requires_git_shape() {
    let fixture = Fixture::new();
    let metadata = fixture.cwd.join("metadata");
    let worktree = fixture.cwd.join("worktree");
    std::fs::create_dir_all(&metadata).expect("metadata");
    std::fs::create_dir_all(&worktree).expect("worktree");
    std::fs::write(worktree.join(".git"), "gitdir: ../metadata\n").expect("git marker");
    let home = fixture.home.canonicalize().expect("home canonical");
    assert!(
        confine::git_common_dir(&worktree.canonicalize().expect("worktree canonical"), &home)
            .is_none()
    );

    std::fs::create_dir(metadata.join("objects")).expect("objects");
    assert!(
        confine::git_common_dir(&worktree.canonicalize().expect("worktree canonical"), &home)
            .is_none()
    );

    std::fs::remove_dir(metadata.join("objects")).expect("remove objects");
    std::fs::write(metadata.join("HEAD"), "ref: refs/heads/main\n").expect("HEAD");
    assert!(
        confine::git_common_dir(&worktree.canonicalize().expect("worktree canonical"), &home)
            .is_none()
    );

    std::fs::write(metadata.join("objects"), "not a directory").expect("objects file");
    assert!(
        confine::git_common_dir(&worktree.canonicalize().expect("worktree canonical"), &home)
            .is_none()
    );
    std::fs::remove_file(metadata.join("objects")).expect("remove objects file");

    std::fs::create_dir(metadata.join("objects")).expect("objects");
    assert!(
        confine::git_common_dir(&worktree.canonicalize().expect("worktree canonical"), &home)
            .is_some()
    );

    let config_worktree = fixture.cwd.join("config-worktree");
    std::fs::create_dir_all(&config_worktree).expect("config worktree");
    std::fs::write(
        config_worktree.join(".git"),
        format!("gitdir: {}\n", fixture.home.join(".config").display()),
    )
    .expect("config pointer");
    std::fs::create_dir_all(fixture.home.join(".config")).expect("config directory");
    assert!(
        confine::git_common_dir(
            &config_worktree
                .canonicalize()
                .expect("config worktree canonical"),
            &home
        )
        .is_none()
    );

    let plain = fixture.cwd.join("plain");
    std::fs::create_dir_all(&plain).expect("plain worktree");
    std::fs::write(
        plain.join(".git"),
        format!("gitdir: {}\n", fixture.home.join(".ssh").display()),
    )
    .expect("plain pointer");
    assert!(
        confine::git_common_dir(&plain.canonicalize().expect("plain canonical"), &home).is_none()
    );
    let plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &plain,
        &fixture.host(),
    )
    .expect("plain pointer plan");
    let ssh = fixture
        .home
        .join(".ssh")
        .canonicalize()
        .expect("SSH canonical");
    assert!(plan.hidden_paths().contains(&ssh));
    assert!(!plan.writable.contains(&ssh));
}

#[test]
fn git_common_dir_canonicalizes_commondir_targets() {
    let fixture = Fixture::new();
    let metadata = fixture.cwd.join("commondir-meta/nested");
    let worktree = fixture.cwd.join("commondir-worktree");
    let common = fixture.cwd.join("common.git");
    std::fs::create_dir_all(&metadata).expect("commondir metadata");
    std::fs::create_dir_all(&worktree).expect("commondir worktree");
    std::fs::create_dir_all(&common).expect("common git");
    std::fs::write(common.join("HEAD"), "ref: refs/heads/main\n").expect("common HEAD");
    std::fs::create_dir(common.join("objects")).expect("common objects");
    std::fs::write(
        worktree.join(".git"),
        format!("gitdir: {}\n", metadata.display()),
    )
    .expect("commondir git pointer");
    std::fs::write(metadata.join("commondir"), "../../common.git\n").expect("relative commondir");
    let home = fixture.home.canonicalize().expect("home canonical");
    let worktree = worktree.canonicalize().expect("worktree canonical");
    let expected = common.canonicalize().expect("common canonical");
    let (actual, _) = confine::git_common_dir(&worktree, &home).expect("relative common dir");
    assert_eq!(actual, expected);

    std::fs::write(
        metadata.join("commondir"),
        format!(
            "{}/commondir-meta/nested/../../common.git\n",
            fixture.cwd.display()
        ),
    )
    .expect("absolute commondir");
    let (actual, _) = confine::git_common_dir(&worktree, &home).expect("absolute common dir");
    assert_eq!(actual, expected);

    let home_metadata = fixture.cwd.join("home-meta/nested");
    let home_worktree = fixture.cwd.join("home-worktree");
    std::fs::create_dir_all(&home_metadata).expect("home metadata");
    std::fs::create_dir_all(&home_worktree).expect("home worktree");
    std::fs::write(
        home_worktree.join(".git"),
        format!("gitdir: {}\n", home_metadata.display()),
    )
    .expect("home git pointer");
    std::fs::write(
        home_metadata.join("commondir"),
        format!("{}\n", fixture.home.join("../home").display()),
    )
    .expect("home commondir");
    std::fs::write(fixture.home.join("HEAD"), "ref: refs/heads/main\n").expect("home HEAD");
    std::fs::create_dir(fixture.home.join("objects")).expect("home objects");
    assert!(
        confine::git_common_dir(
            &home_worktree
                .canonicalize()
                .expect("home worktree canonical"),
            &home
        )
        .is_none()
    );
}

#[test]
fn git_common_dir_rejects_nonregular_head_without_hanging() {
    let fixture = Fixture::new();
    let metadata = fixture.cwd.join("metadata");
    let worktree = fixture.cwd.join("worktree");
    std::fs::create_dir_all(&metadata).expect("metadata");
    std::fs::create_dir_all(&worktree).expect("worktree");
    std::fs::create_dir(metadata.join("HEAD")).expect("HEAD directory");
    std::fs::create_dir(metadata.join("objects")).expect("objects");
    std::fs::write(worktree.join(".git"), "gitdir: ../metadata\n").expect("git marker");
    let result = git_common_dir_with_timeout(
        worktree.canonicalize().expect("worktree canonical"),
        fixture.home.canonicalize().expect("home canonical"),
    );
    assert!(result.is_none());

    std::fs::remove_dir(metadata.join("HEAD")).expect("remove HEAD directory");
    let status = Command::new("mkfifo")
        .arg(metadata.join("HEAD"))
        .status()
        .expect("mkfifo");
    assert!(status.success(), "mkfifo status: {status}");
    let result = git_common_dir_with_timeout(
        worktree.canonicalize().expect("worktree canonical"),
        fixture.home.canonicalize().expect("home canonical"),
    );
    assert!(result.is_none());
}

fn git_common_dir_with_timeout(cwd: PathBuf, home: PathBuf) -> Option<(PathBuf, PathBuf)> {
    let (sender, receiver) = std_mpsc::channel();
    std::thread::spawn(move || {
        let result = confine::git_common_dir(&cwd, &home);
        let _ = sender.send(result);
    });
    receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("git common directory lookup timed out")
}

#[test]
fn git_common_dir_rejects_fifo_commondir_without_hanging() {
    let fixture = Fixture::new();
    let worktree = fixture.cwd.join("worktree");
    let metadata = fixture.cwd.join("meta/worktree");
    std::fs::create_dir_all(&worktree).expect("worktree");
    std::fs::create_dir_all(&metadata).expect("worktree metadata");
    std::fs::write(worktree.join(".git"), "gitdir: ../meta/worktree\n").expect("gitdir marker");
    let fifo = metadata.join("commondir");
    let status = Command::new("mkfifo").arg(&fifo).status().expect("mkfifo");
    assert!(status.success(), "mkfifo status: {status}");
    let result = git_common_dir_with_timeout(
        std::fs::canonicalize(&worktree).expect("canonical worktree"),
        std::fs::canonicalize(&fixture.home).expect("canonical home"),
    );
    assert!(result.is_none());
}

#[test]
fn git_common_dir_rejects_oversized_git_marker() {
    let fixture = Fixture::new();
    let worktree = fixture.cwd.join("worktree");
    let metadata = fixture.cwd.join("metadata");
    std::fs::create_dir_all(&worktree).expect("worktree");
    std::fs::create_dir_all(&metadata).expect("gitdir");
    std::fs::write(
        worktree.join(".git"),
        format!("gitdir: ../metadata\n{}", "x".repeat(4096)),
    )
    .expect("oversized git marker");
    let result = git_common_dir_with_timeout(
        std::fs::canonicalize(&worktree).expect("canonical worktree"),
        std::fs::canonicalize(&fixture.home).expect("canonical home"),
    );
    assert!(result.is_none());
}

#[test]
fn git_common_dir_rejects_directory_commondir() {
    let fixture = Fixture::new();
    let worktree = fixture.cwd.join("worktree");
    let metadata = fixture.cwd.join("meta/worktree");
    std::fs::create_dir_all(&worktree).expect("worktree");
    std::fs::create_dir_all(metadata.join("commondir")).expect("commondir directory");
    std::fs::write(worktree.join(".git"), "gitdir: ../meta/worktree\n").expect("gitdir marker");
    let result = git_common_dir_with_timeout(
        std::fs::canonicalize(&worktree).expect("canonical worktree"),
        std::fs::canonicalize(&fixture.home).expect("canonical home"),
    );
    assert!(result.is_none());
}

#[test]
fn git_common_dir_rejects_oversized_commondir() {
    let fixture = Fixture::new();
    let worktree = fixture.cwd.join("worktree");
    let metadata = fixture.cwd.join("meta/worktree");
    let common = fixture.cwd.join("common.git");
    std::fs::create_dir_all(&worktree).expect("worktree");
    std::fs::create_dir_all(&metadata).expect("worktree metadata");
    std::fs::create_dir_all(&common).expect("common git");
    std::fs::write(common.join("HEAD"), "ref: refs/heads/main\n").expect("common HEAD");
    std::fs::create_dir(common.join("objects")).expect("common objects");
    std::fs::write(worktree.join(".git"), "gitdir: ../meta/worktree\n").expect("gitdir marker");
    std::fs::write(
        metadata.join("commondir"),
        format!("../../common.git\n{}", " ".repeat(4096)),
    )
    .expect("oversized commondir");
    let result = git_common_dir_with_timeout(
        worktree.canonicalize().expect("worktree canonical"),
        fixture.home.canonicalize().expect("home canonical"),
    );
    assert!(result.is_none());
}

#[test]
fn home_repository_is_not_bound_and_outside_team_file_stays_hidden() {
    let fixture = Fixture::new();
    std::fs::create_dir_all(fixture.home.join(".git")).expect("home git");
    let child = fixture.home.join("project");
    std::fs::create_dir_all(&child).expect("home project");
    let plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &child,
        &fixture.host(),
    )
    .expect("home child plan");
    assert!(
        plan.writable
            .iter()
            .all(|path| !path.starts_with(fixture.home.join(".git")))
    );
    let team_file = fixture.state.join("config/team.toml");
    std::fs::write(&team_file, "team = \"private\"\n").expect("state team file");
    let plan = confine::plan(
        &ConfineRequest {
            team_file: Some(team_file.to_string_lossy().into_owned()),
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("outside team plan");
    assert!(!plan.readonly.contains(&team_file));
    assert!(!plan.protected.contains(&team_file));
    assert!(
        plan.hidden_paths()
            .contains(&fixture.host().layout.state_dir())
    );
}
