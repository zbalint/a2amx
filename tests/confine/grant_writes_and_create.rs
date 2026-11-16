use super::*;

#[test]
fn grants_are_validated_and_reported_by_the_plan() {
    let fixture = Fixture::new();
    let plan = confine::plan(
        &ConfineRequest {
            grants: vec!["docker".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("docker grant");
    assert_eq!(plan.grants, ["docker"]);
    assert!(confine::notice(&plan).contains("Granted: docker."));

    let error = match confine::plan(
        &ConfineRequest {
            grants: vec!["container".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    ) {
        Ok(_) => panic!("unknown grant"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("closest match: containerd"));
}

#[test]
fn grant_count_limit_precedes_grant_lookup() {
    let fixture = Fixture::new();
    let error = match confine::plan(
        &ConfineRequest {
            grants: (0..65).map(|_| "docker".to_owned()).collect(),
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    ) {
        Ok(_) => panic!("grant count limit was not enforced"),
        Err(error) => error,
    };
    assert_eq!(error.to_string(), "grants has more than 64 entries");
}
#[test]
fn write_home_is_allowed_and_keeps_hides_on_top() {
    let fixture = Fixture::new();
    let root = std::fs::canonicalize(fixture._root.path()).expect("fixture root");
    let netrc = fixture.home.join(".netrc");
    std::fs::write(&netrc, []).expect("netrc");
    let plan = confine::plan(
        &ConfineRequest {
            write: vec![root.to_string_lossy().into_owned()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("ancestor write plan");
    let root = root.to_string_lossy().into_owned();
    let empty = std::fs::canonicalize(&fixture.empty)
        .expect("empty canonical")
        .to_string_lossy()
        .into_owned();
    let hidden = plan.hidden_paths().to_vec();
    let wrapped = plan
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("wrapped ancestor plan");
    let writable_index = wrapped
        .windows(3)
        .position(|window| window == ["--bind", root.as_str(), root.as_str()])
        .expect("ancestor writable bind");
    for path in [
        fixture.host().layout.state_dir(),
        fixture.runtime.clone(),
        fixture.home.join(".ssh"),
        netrc,
    ] {
        let path = std::fs::canonicalize(path).expect("hidden path");
        assert!(hidden.contains(&path), "missing hidden path {path:?}");
        let path = path.to_string_lossy().into_owned();
        let hide_index = wrapped
            .windows(2)
            .position(|window| window == ["--tmpfs", path.as_str()])
            .or_else(|| {
                wrapped
                    .windows(3)
                    .position(|window| window == ["--ro-bind", empty.as_str(), path.as_str()])
            })
            .expect("hide mount");
        assert!(
            hide_index > writable_index,
            "hide mount must follow ancestor writable mount"
        );
    }

    let plan = confine::plan(
        &ConfineRequest {
            write: vec!["~".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("home write plan");
    assert!(plan.writable.contains(&fixture.home));
    for path in [fixture.home.join(".ssh"), fixture.home.join(".netrc")] {
        assert!(
            plan.hidden_paths()
                .contains(&std::fs::canonicalize(path).expect("home hide"))
        );
    }
}

#[test]
fn write_home_keeps_the_launch_inputs_read_only() {
    let fixture = Fixture::new();
    let install = fixture.home.join(".a2amx");
    let path_dir = fixture.home.join(".local/bin");
    let relative = fixture.home.join("relative");
    std::fs::create_dir_all(&install).expect("install directory");
    std::fs::create_dir_all(&path_dir).expect("PATH directory");
    std::fs::create_dir_all(&relative).expect("relative PATH directory");
    let path_var = OsString::from(format!("{}:/usr/bin", path_dir.display()));
    let plan = |write: Vec<String>, path_var: OsString| {
        confine::plan_with_path(
            &ConfineRequest {
                write,
                ..ConfineRequest::default()
            },
            Harness::Generic,
            &fixture.cwd,
            &fixture.host(),
            Some(path_var),
        )
        .expect("launch-input plan")
    };

    let base = plan(vec!["~".into()], path_var.clone());
    let install = std::fs::canonicalize(&install).expect("install canonical");
    let path_dir = std::fs::canonicalize(&path_dir).expect("PATH canonical");
    let relative = std::fs::canonicalize(&relative).expect("relative canonical");
    assert!(base.protected.contains(&install));
    assert!(base.protected.contains(&path_dir));
    assert!(
        !base
            .readonly
            .contains(&std::fs::canonicalize("/usr/bin").expect("usr bin"))
    );
    assert!(
        !base
            .protected
            .contains(&std::fs::canonicalize("/usr/bin").expect("usr bin"))
    );

    let install_open = plan(vec!["~".into(), "~/.a2amx".into()], path_var.clone());
    assert!(!install_open.protected.contains(&install));
    assert!(install_open.protected.contains(&path_dir));

    let path_open = plan(vec!["~".into(), "~/.local/bin".into()], path_var.clone());
    assert!(path_open.protected.contains(&install));
    assert!(!path_open.protected.contains(&path_dir));

    let relative_path = plan(
        vec!["~".into()],
        OsString::from(format!("relative::{}", path_dir.display())),
    );
    assert!(!relative_path.readonly.contains(&relative));
    assert!(!relative_path.protected.contains(&relative));
}
#[test]
fn grant_create_paths_are_preflighted_and_writable() {
    let fixture = Fixture::new();
    let host = fixture.host();
    let request = ConfineRequest {
        grants: vec!["cargo".into(), "cache".into()],
        ..ConfineRequest::default()
    };
    let preflight = confine::ensure_grant_dirs(&request, Harness::Generic, &host, true);
    assert!(preflight.is_empty());
    let plan = confine::plan_with_preflight(
        &request,
        Harness::Generic,
        &fixture.cwd,
        &host,
        None,
        preflight,
    )
    .expect("grant plan");
    let cargo = std::fs::canonicalize(fixture.home.join(".cargo")).expect("created cargo");
    let cache = std::fs::canonicalize(fixture.home.join(".cache")).expect("created cache");
    assert!(plan.writable.contains(&cargo));
    assert!(plan.writable.contains(&cache));
}

#[test]
fn grant_create_path_inside_state_is_unresolved() {
    let fixture = Fixture::new();
    let cache = fixture.home.join(".cache");
    let state = cache.join("state");
    std::fs::create_dir_all(&state).expect("nested state");
    let empty_file = state.join("confine-empty");
    std::fs::write(&empty_file, []).expect("empty file");
    let host = Host {
        home: fixture.home.clone(),
        layout: a2amx::layout::Layout::new(state.clone()),
        runtime_dir: fixture.runtime.clone(),
    };
    let request = ConfineRequest {
        grants: vec!["cache".into()],
        ..ConfineRequest::default()
    };
    let preflight = confine::ensure_grant_dirs(&request, Harness::Generic, &host, true);
    let expected = std::fs::canonicalize(cache).expect("cache path");
    assert!(preflight.iter().any(|item| {
        item.option == "grant" && item.path == expected && item.reason.contains("protected")
    }));
}

#[test]
fn grant_create_matrix_is_safe_and_explicit() {
    let node = Fixture::new();
    let node_request = ConfineRequest {
        grants: vec!["node".into()],
        ..ConfineRequest::default()
    };
    let unresolved =
        confine::ensure_grant_dirs(&node_request, Harness::Generic, &node.host(), true);
    assert!(unresolved.is_empty());
    assert!(node.home.join(".npm").is_dir());

    let aws = Fixture::new();
    let aws_request = ConfineRequest {
        grants: vec!["aws".into()],
        ..ConfineRequest::default()
    };
    std::fs::remove_dir_all(aws.home.join(".aws")).expect("remove fixture aws directory");
    confine::ensure_grant_dirs(&aws_request, Harness::Generic, &aws.host(), true);
    assert!(!aws.home.join(".aws").exists());

    let dry = Fixture::new();
    confine::ensure_grant_dirs(&node_request, Harness::Generic, &dry.host(), false);
    assert!(!dry.home.join(".npm").exists());

    let symlinked = Fixture::new();
    let target = symlinked._root.path().join("npm-target");
    std::fs::create_dir_all(&target).expect("npm target");
    symlink(&target, symlinked.home.join(".npm")).expect("npm symlink");
    let unresolved =
        confine::ensure_grant_dirs(&node_request, Harness::Generic, &symlinked.host(), true);
    assert!(
        unresolved
            .iter()
            .any(|item| { item.option == "grant" && item.path == symlinked.home.join(".npm") })
    );
    assert!(!target.join("created").exists());
}

#[test]
fn grant_paths_keep_nested_credentials_hidden() {
    let fixture = Fixture::new();
    let cargo = fixture.home.join(".cargo");
    let cache = fixture.home.join(".cache");
    let credentials = cargo.join("credentials.toml");
    let token = cache.join("huggingface/token");
    std::fs::create_dir_all(&cargo).expect("cargo directory");
    std::fs::create_dir_all(token.parent().expect("token parent")).expect("cache directories");
    std::fs::write(&credentials, "credentials").expect("cargo credentials");
    std::fs::write(&token, "token").expect("huggingface token");
    let request = ConfineRequest {
        grants: vec!["cargo".into(), "cache".into()],
        ..ConfineRequest::default()
    };
    let plan = confine::plan(&request, Harness::Generic, &fixture.cwd, &fixture.host())
        .expect("grant path plan");
    let cargo = std::fs::canonicalize(cargo).expect("cargo path");
    let cache = std::fs::canonicalize(cache).expect("cache path");
    let credentials = std::fs::canonicalize(credentials).expect("credentials path");
    let token = std::fs::canonicalize(token).expect("token path");
    assert!(plan.writable.contains(&cargo));
    assert!(plan.writable.contains(&cache));
    assert!(plan.hidden_paths().contains(&credentials));
    assert!(plan.hidden_paths().contains(&token));
}

#[test]
fn grant_create_ancestor_does_not_release_hidden_path() {
    let fixture = Fixture::new();
    std::fs::create_dir_all(fixture.home.join(".cargo")).expect("cargo directory");
    symlink(
        fixture.home.join(".ssh"),
        fixture.home.join(".cargo/registry"),
    )
    .expect("registry symlink");
    let host = fixture.host();
    let request = ConfineRequest {
        grants: vec!["cargo".into()],
        ..ConfineRequest::default()
    };
    let preflight = confine::ensure_grant_dirs(&request, Harness::Generic, &host, true);
    assert!(preflight.is_empty());
    let plan = confine::plan_with_preflight(
        &request,
        Harness::Generic,
        &fixture.cwd,
        &host,
        None,
        preflight,
    )
    .expect("grant path plan");
    let ssh = std::fs::canonicalize(fixture.home.join(".ssh")).expect("ssh path");
    assert!(!plan.writable.contains(&ssh));
    assert!(plan.hidden_paths().contains(&ssh));
}

#[test]
fn grant_create_symlink_ancestor_is_not_created() {
    let fixture = Fixture::new();
    let target = fixture._root.path().join("outside");
    std::fs::create_dir_all(&target).expect("outside directory");
    symlink(&target, fixture.home.join(".cargo")).expect("cargo symlink");
    let host = fixture.host();
    let request = ConfineRequest {
        grants: vec!["cargo".into()],
        ..ConfineRequest::default()
    };
    let preflight = confine::ensure_grant_dirs(&request, Harness::Generic, &host, true);
    let error = match confine::plan_with_preflight(
        &request,
        Harness::Generic,
        &fixture.cwd,
        &host,
        None,
        preflight,
    ) {
        Ok(_) => panic!("symlinked cargo grant must fail"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("passes through the symlink"));
}

#[test]
fn write_slash_and_inside_state_dir_stay_refused() {
    let fixture = Fixture::new();
    let nested = fixture.state.join("nested");
    std::fs::create_dir_all(&nested).expect("nested state path");
    for path in [
        "/",
        fixture.state.to_str().expect("state"),
        nested.to_str().expect("nested"),
        fixture.runtime.to_str().expect("runtime"),
    ] {
        assert!(
            confine::plan(
                &ConfineRequest {
                    write: vec![path.into()],
                    ..ConfineRequest::default()
                },
                Harness::Generic,
                &fixture.cwd,
                &fixture.host(),
            )
            .is_err(),
            "write must be refused: {path}"
        );
    }
}

#[test]
fn workspace_in_home_is_still_refused() {
    let fixture = Fixture::new();
    let error = match confine::plan(
        &ConfineRequest {
            write: vec!["~".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.home,
        &fixture.host(),
    ) {
        Ok(_) => panic!("home cwd must remain refused"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("writable path $HOME is not allowed")
    );

    let outer = tempfile::tempdir().expect("outer root");
    let home = outer.path().join("home");
    std::fs::create_dir_all(&home).expect("outer home");
    let protected = tempfile::tempdir().expect("protected root");
    let state = protected.path().join("state");
    let runtime = protected.path().join("runtime");
    std::fs::create_dir_all(&state).expect("outer state");
    std::fs::create_dir_all(&runtime).expect("outer runtime");
    let layout = a2amx::layout::Layout::new(state.clone());
    layout.ensure().expect("outer layout");
    let empty = layout.confine_empty();
    std::fs::write(&empty, []).expect("outer empty");
    let host = Host {
        home: home.clone(),
        layout,
        runtime_dir: runtime,
    };
    let error = match confine::plan(
        &ConfineRequest {
            write: vec!["~".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        outer.path(),
        &host,
    ) {
        Ok(_) => panic!("ancestor cwd must remain refused"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("writable path $HOME is not allowed")
    );
}
