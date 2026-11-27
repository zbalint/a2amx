use super::*;

fn plan_python_error(fixture: &Fixture, harness: Harness, no_default_hides: bool) -> String {
    confine::plan(
        &ConfineRequest {
            grants: vec!["python".into()],
            no_default_hides,
            ..ConfineRequest::default()
        },
        harness,
        &fixture.cwd,
        &fixture.host(),
    )
    .err()
    .expect("python grant plan must fail")
    .to_string()
}

#[test]
fn grant_path_inside_a2amx_home_is_rejected() {
    let fixture = Fixture::new();
    let cache = fixture.home.join(".cache");
    std::fs::create_dir_all(cache.join("pip")).expect("pip dir");
    symlink(&cache, fixture.home.join(".a2amx")).expect(".a2amx symlink");

    assert_eq!(
        plan_python_error(&fixture, Harness::Generic, false),
        "confinement grant \"python\" path ~/.cache/pip is inside ~/.a2amx"
    );
}

#[test]
fn grant_path_inside_harness_root_is_rejected() {
    let fixture = Fixture::new();
    let cache = fixture.home.join(".cache");
    std::fs::create_dir_all(cache.join("pip")).expect("pip dir");
    symlink(&cache, fixture.home.join(".gemini")).expect(".gemini symlink");

    let cache_canon = std::fs::canonicalize(&cache).expect("cache canon");
    assert_eq!(
        plan_python_error(&fixture, Harness::Agy, false),
        format!(
            "confinement grant \"python\" path ~/.cache/pip is inside the harness root {}",
            cache_canon.display()
        )
    );
}

fn setup_pip_and_docker_sock(fixture: &Fixture) -> PathBuf {
    let pip = fixture.home.join(".cache/pip");
    std::fs::create_dir_all(&pip).expect("pip dir");
    let sock_target = pip.join("docker.sock");
    std::fs::write(&sock_target, "").expect("sock target");
    let docker_run = fixture.home.join(".docker/run");
    std::fs::create_dir_all(&docker_run).expect("docker run dir");
    symlink(&sock_target, docker_run.join("docker.sock")).expect("docker sock symlink");
    sock_target
}

#[test]
fn grant_path_resolving_to_default_hide_precedes_container_hide() {
    let fixture = Fixture::new();
    setup_pip_and_docker_sock(&fixture);
    let cache = fixture.home.join(".cache");
    symlink(&cache, fixture.home.join(".kube")).expect(".kube symlink");

    let cache_canon = std::fs::canonicalize(&cache).expect("cache canon");
    assert_eq!(
        plan_python_error(&fixture, Harness::Generic, false),
        format!(
            "confinement grant \"python\" path ~/.cache/pip resolves to the hidden path {}",
            cache_canon.display()
        )
    );
}

#[test]
fn grant_path_resolving_to_container_socket_hide_is_rejected() {
    let fixture = Fixture::new();
    let sock_target = setup_pip_and_docker_sock(&fixture);

    let sock_canon = std::fs::canonicalize(&sock_target).expect("sock canon");
    assert_eq!(
        plan_python_error(&fixture, Harness::Generic, true),
        format!(
            "confinement grant \"python\" path ~/.cache/pip resolves to the hidden path {}",
            sock_canon.display()
        )
    );
}

#[test]
fn grant_path_resolving_to_mandatory_hide_is_rejected() {
    let fixture = Fixture::new();
    let pip = fixture.home.join(".cache/pip");
    std::fs::create_dir_all(&pip).expect("pip dir");
    let run_dir = a2amx::layout::Layout::new(fixture.state.clone()).run_dir();
    std::fs::remove_dir_all(&run_dir).expect("remove run dir");
    symlink(&pip, &run_dir).expect("run symlink");

    let pip_canon = std::fs::canonicalize(&pip).expect("pip canon");
    assert_eq!(
        plan_python_error(&fixture, Harness::Generic, true),
        format!(
            "confinement grant \"python\" path ~/.cache/pip resolves to the hidden path {}",
            pip_canon.display()
        )
    );
}

#[test]
fn explicit_write_root_is_rejected() {
    let fixture = Fixture::new();
    let error = confine::plan(
        &ConfineRequest {
            write: vec!["/".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .err()
    .expect("explicit write of / must fail");
    assert_eq!(error.to_string(), "writable path / is not allowed");
}

#[test]
fn explicit_write_a2amx_directory_is_rejected() {
    let fixture = Fixture::new();
    let error = confine::plan(
        &ConfineRequest {
            write: vec![fixture.state.to_string_lossy().into_owned()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .err()
    .expect("explicit write of a2amx directory must fail");
    let state_canon = std::fs::canonicalize(&fixture.state).expect("state canon");
    assert_eq!(
        error.to_string(),
        format!(
            "writable path {} overlaps the a2amx directory",
            state_canon.display()
        )
    );
}

#[test]
fn implicit_cwd_overlapping_mandatory_root_is_rejected() {
    let fixture = Fixture::new();
    let run_link = fixture._root.path().join("run-link");
    symlink("/run", &run_link).expect("run symlink");

    let error = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &run_link,
        &fixture.host(),
    )
    .err()
    .expect("cwd overlapping /run/WSL must fail");
    assert_eq!(
        error.to_string(),
        "writable path /run overlaps a mandatory hidden path"
    );
}

#[test]
fn implicit_cwd_overlapping_runtime_dir_is_rejected() {
    let fixture = Fixture::new();
    let error = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &fixture.runtime,
        &fixture.host(),
    )
    .err()
    .expect("cwd overlapping runtime dir must fail");
    let runtime_canon = std::fs::canonicalize(&fixture.runtime).expect("runtime canon");
    assert_eq!(
        error.to_string(),
        format!(
            "writable path {} overlaps the runtime directory",
            runtime_canon.display()
        )
    );
}

#[test]
fn implicit_cwd_containing_runtime_dir_is_rejected() {
    let fixture = Fixture::new();
    let tree = fixture._root.path().join("runtime-parent");
    let runtime_dir = tree.join("runtime");
    std::fs::create_dir_all(&runtime_dir).expect("runtime dir");
    let host = Host {
        home: fixture.home.clone(),
        layout: a2amx::layout::Layout::new(fixture.state.clone()),
        runtime_dir,
    };
    let error = confine::plan(&ConfineRequest::default(), Harness::Generic, &tree, &host)
        .err()
        .expect("cwd containing runtime dir must fail");
    let tree_canon = std::fs::canonicalize(&tree).expect("tree canon");
    assert_eq!(
        error.to_string(),
        format!(
            "writable path {} overlaps the runtime directory",
            tree_canon.display()
        )
    );
}

#[test]
fn harness_config_grant_allows_grant_path_inside_harness_root() {
    let fixture = Fixture::new();
    let cache = fixture.home.join(".cache");
    let pip = cache.join("pip");
    std::fs::create_dir_all(&pip).expect("pip dir");
    symlink(&cache, fixture.home.join(".codex")).expect(".codex symlink");

    let pip_canon = std::fs::canonicalize(&pip).expect("pip canon");
    let plan = confine::plan(
        &ConfineRequest {
            grants: vec!["harness-config".into(), "python".into()],
            ..ConfineRequest::default()
        },
        Harness::Codex,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("harness-config grant allows python path inside harness root");
    assert!(plan.writable.contains(&pip_canon));
}

#[test]
fn ssh_grant_removes_home_ssh_from_default_hidden() {
    let fixture = Fixture::new();
    let ssh_canon = std::fs::canonicalize(fixture.home.join(".ssh")).expect("ssh canon");
    let plan = confine::plan(
        &ConfineRequest {
            grants: vec!["ssh".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("ssh grant plan");
    assert!(!plan.default_hidden.contains(&ssh_canon));
    assert!(!plan.hidden_paths().contains(&ssh_canon));
}
