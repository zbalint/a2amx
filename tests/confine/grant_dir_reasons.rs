use super::*;

#[test]
fn grant_dir_protected_overlap_and_hidden_existing_reasons_are_pinned() {
    let fixture = Fixture::new();
    let cache = fixture.home.join(".cache");
    let state = cache.join("state");
    std::fs::create_dir_all(&state).expect("nested state");
    std::fs::write(state.join("confine-empty"), []).expect("empty file");
    let state_host = Host {
        home: fixture.home.clone(),
        layout: a2amx::layout::Layout::new(state),
        runtime_dir: fixture.runtime.clone(),
    };
    let cache_request = ConfineRequest {
        grants: vec!["cache".into()],
        ..ConfineRequest::default()
    };
    let unresolved =
        confine::ensure_grant_dirs(&cache_request, Harness::Generic, &state_host, true);
    assert_eq!(
        unresolved,
        vec![confine::Unresolved {
            option: "grant",
            path: std::fs::canonicalize(&cache).expect("cache canonical"),
            reason: "path overlaps protected location".into(),
        }]
    );

    let hidden_fixture = Fixture::new();
    let hidden_cache = hidden_fixture.home.join(".cache");
    std::fs::create_dir_all(&hidden_cache).expect("hidden cache");
    let hidden_request = ConfineRequest {
        grants: vec!["cache".into()],
        hide: vec!["~/.cache".into()],
        ..ConfineRequest::default()
    };
    let unresolved = confine::ensure_grant_dirs(
        &hidden_request,
        Harness::Generic,
        &hidden_fixture.host(),
        true,
    );
    assert_eq!(
        unresolved,
        vec![confine::Unresolved {
            option: "grant",
            path: std::fs::canonicalize(&hidden_cache).expect("hidden cache canonical"),
            reason: "path overlaps protected location".into(),
        }]
    );
}

#[test]
fn grant_dir_symlink_and_hidden_ancestor_reasons_are_pinned() {
    let fixture = Fixture::new();
    let npm_target = fixture._root.path().join("npm-target");
    std::fs::create_dir_all(&npm_target).expect("npm target");
    let npm = fixture.home.join(".npm");
    symlink(&npm_target, &npm).expect("npm symlink");
    let node_request = ConfineRequest {
        grants: vec!["node".into()],
        ..ConfineRequest::default()
    };
    let unresolved =
        confine::ensure_grant_dirs(&node_request, Harness::Generic, &fixture.host(), true);
    assert_eq!(
        unresolved,
        vec![confine::Unresolved {
            option: "grant",
            path: npm.clone(),
            reason: format!("symlink component: {}", npm.display()),
        }]
    );

    let missing_fixture = Fixture::new();
    let cache_target = missing_fixture._root.path().join("cache-target");
    std::fs::create_dir_all(&cache_target).expect("cache target");
    let cache = missing_fixture.home.join(".cache");
    symlink(&cache_target, &cache).expect("cache symlink");
    let c_cpp_request = ConfineRequest {
        grants: vec!["c-cpp".into()],
        ..ConfineRequest::default()
    };
    let unresolved = confine::ensure_grant_dirs(
        &c_cpp_request,
        Harness::Generic,
        &missing_fixture.host(),
        true,
    );
    assert_eq!(
        unresolved,
        vec![confine::Unresolved {
            option: "grant",
            path: cache.join("ccache"),
            reason: format!("symlink component: {}", cache.display()),
        }]
    );

    let ancestor_fixture = Fixture::new();
    std::fs::create_dir_all(ancestor_fixture.home.join(".cache")).expect("cache dir");
    let hidden_ancestor_request = ConfineRequest {
        grants: vec!["c-cpp".into()],
        hide: vec!["~/.cache".into()],
        ..ConfineRequest::default()
    };
    let unresolved = confine::ensure_grant_dirs(
        &hidden_ancestor_request,
        Harness::Generic,
        &ancestor_fixture.host(),
        true,
    );
    assert_eq!(
        unresolved,
        vec![confine::Unresolved {
            option: "grant",
            path: std::fs::canonicalize(&ancestor_fixture.home)
                .expect("home canonical")
                .join(".cache/ccache"),
            reason: "ancestor is not writable".into(),
        }]
    );
}

#[test]
fn grant_dir_create_failure_existing_file_and_order_are_pinned() {
    let fixture = Fixture::new();
    std::fs::write(fixture.home.join(".cache"), "not a directory").expect("cache file");
    std::fs::write(fixture.home.join(".npm"), "regular file").expect("npm file");
    let node_request = ConfineRequest {
        grants: vec!["node".into()],
        ..ConfineRequest::default()
    };
    let node_unresolved =
        confine::ensure_grant_dirs(&node_request, Harness::Generic, &fixture.host(), true);
    assert!(node_unresolved.is_empty());
    assert!(fixture.home.join(".npm").is_file());

    let npm_target = fixture._root.path().join("npm-target");
    std::fs::create_dir_all(&npm_target).expect("npm target");
    std::fs::remove_file(fixture.home.join(".npm")).expect("remove npm file");
    let npm = fixture.home.join(".npm");
    symlink(&npm_target, &npm).expect("npm symlink");

    let order_request = ConfineRequest {
        grants: vec!["node".into(), "python".into()],
        ..ConfineRequest::default()
    };
    let unresolved =
        confine::ensure_grant_dirs(&order_request, Harness::Generic, &fixture.host(), true);
    let canonical_home = std::fs::canonicalize(&fixture.home).expect("home canonical");
    assert_eq!(
        unresolved,
        vec![
            confine::Unresolved {
                option: "grant",
                path: npm.clone(),
                reason: format!("symlink component: {}", npm.display()),
            },
            confine::Unresolved {
                option: "grant",
                path: canonical_home.join(".cache/pip"),
                reason: "Not a directory (os error 20)".into(),
            },
            confine::Unresolved {
                option: "grant",
                path: canonical_home.join(".cache/uv"),
                reason: "Not a directory (os error 20)".into(),
            },
        ]
    );
}

#[test]
fn grant_dir_runtime_dir_overlap_reason_is_pinned() {
    let fixture = Fixture::new();
    let cache = fixture.home.join(".cache");
    std::fs::create_dir_all(&cache).expect("cache dir");
    let runtime_host = Host {
        home: fixture.home.clone(),
        layout: a2amx::layout::Layout::new(fixture.state.clone()),
        runtime_dir: cache.clone(),
    };
    let cache_request = ConfineRequest {
        grants: vec!["cache".into()],
        ..ConfineRequest::default()
    };
    let unresolved =
        confine::ensure_grant_dirs(&cache_request, Harness::Generic, &runtime_host, true);
    assert_eq!(
        unresolved,
        vec![confine::Unresolved {
            option: "grant",
            path: std::fs::canonicalize(&cache).expect("cache canonical"),
            reason: "path overlaps protected location".into(),
        }]
    );
}
