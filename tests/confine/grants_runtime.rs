use super::*;

#[test]
fn runtime_grants_select_sockets_and_environment() {
    let fixture = Fixture::new();
    let docker_socket = fixture.home.join(".docker/run/docker.sock");
    std::fs::create_dir_all(docker_socket.parent().expect("docker parent")).expect("docker dir");
    std::fs::write(&docker_socket, "socket placeholder").expect("docker socket fixture");
    let restricted = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("restricted plan");
    let restricted_wrapped = restricted
        .wrap(vec!["true".into()])
        .expect("restricted wrapper");
    assert!(
        restricted_wrapped
            .windows(2)
            .any(|window| window == ["--unsetenv", "DOCKER_HOST"])
    );
    assert!(
        restricted_wrapped
            .windows(2)
            .any(|window| window == ["--unsetenv", "CONTAINER_HOST"])
    );
    let docker = confine::plan(
        &ConfineRequest {
            grants: vec!["docker".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("docker plan");
    let docker_socket = std::fs::canonicalize(docker_socket).expect("docker socket canonical");
    assert!(!docker.hidden_paths().contains(&docker_socket));
    let podman = PathBuf::from("/run/podman/podman.sock");
    let docker_host_socket = PathBuf::from("/run/docker.sock");
    let podman_canonical = std::fs::canonicalize(&podman).ok();
    let docker_canonical = std::fs::canonicalize(&docker_host_socket).ok();
    if let (Some(podman_target), Some(docker_target)) =
        (podman_canonical.as_ref(), docker_canonical.as_ref())
    {
        if podman_target == docker_target {
            assert!(!docker.hidden_paths().contains(podman_target));
            assert!(!docker.skipped_paths().contains(&podman));
        } else {
            assert!(
                docker.hidden_paths().contains(podman_target)
                    || docker.skipped_paths().contains(&podman),
                "podman socket must stay hidden or skipped"
            );
        }
    } else {
        let hidden_podman = podman_canonical
            .as_ref()
            .is_some_and(|target| docker.hidden_paths().contains(target));
        assert!(
            hidden_podman || docker.skipped_paths().contains(&podman),
            "podman socket must stay hidden or skipped"
        );
    }
    assert!(!docker.hidden_paths().contains(&docker_host_socket));
    assert!(!docker.skipped_paths().contains(&docker_host_socket));
    let docker_wrapped = docker.wrap(vec!["true".into()]).expect("docker wrapper");
    assert!(
        !docker_wrapped
            .windows(2)
            .any(|window| window == ["--unsetenv", "DOCKER_HOST"])
    );
    assert!(
        !docker_wrapped
            .windows(2)
            .any(|window| window == ["--unsetenv", "DOCKER_CONTEXT"])
    );
    assert!(
        docker_wrapped
            .windows(2)
            .any(|window| window == ["--unsetenv", "CONTAINER_HOST"])
    );
}

#[test]
fn credential_grants_lift_named_paths_writable() {
    let fixture = Fixture::new();
    let gh = fixture.home.join(".config/gh");
    std::fs::create_dir_all(&gh).expect("gh directory");
    std::fs::write(gh.join("hosts.yml"), "github.com:\n").expect("gh hosts");
    let gh = std::fs::canonicalize(gh).expect("gh canonical");

    let restricted = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("restricted plan");
    assert!(restricted.hidden_paths().contains(&gh));

    let granted = confine::plan(
        &ConfineRequest {
            grants: vec!["gh".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("gh plan");
    assert!(granted.writable.contains(&gh));
    assert!(!granted.readonly.contains(&gh));
    assert!(!granted.protected.contains(&gh));
    assert!(!granted.hidden_paths().contains(&gh));
    let gh_string = gh.to_string_lossy().into_owned();
    let wrapped = granted
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("gh wrapper");
    assert!(
        wrapped
            .windows(3)
            .any(|window| window == ["--bind", gh_string.as_str(), gh_string.as_str()])
    );
    assert!(
        !wrapped
            .windows(3)
            .any(|window| window == ["--ro-bind", gh_string.as_str(), gh_string.as_str()])
    );
    assert!(
        !wrapped
            .windows(2)
            .any(|window| window == ["--tmpfs", gh_string.as_str()])
    );
}

#[test]
fn credential_grant_stays_writable_inside_writable_parent() {
    let fixture = Fixture::new();
    let config = fixture.home.join(".config");
    let gh = config.join("gh");
    std::fs::create_dir_all(&gh).expect("gh directory");
    std::fs::write(gh.join("hosts.yml"), "github.com:\n").expect("gh hosts");
    let config = std::fs::canonicalize(config).expect("config canonical");
    let gh = std::fs::canonicalize(gh).expect("gh canonical");

    let granted = confine::plan(
        &ConfineRequest {
            write: vec!["~/.config".into()],
            grants: vec!["gh".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("gh writable-parent plan");
    assert!(granted.writable.contains(&config));
    assert!(granted.writable.contains(&gh));
    assert!(!granted.protected.contains(&gh));
    assert!(!granted.readonly.contains(&gh));
    let config_string = config.to_string_lossy().into_owned();
    let gh_string = gh.to_string_lossy().into_owned();
    let wrapped = granted
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("gh writable-parent wrapper");
    assert!(
        wrapped
            .windows(3)
            .any(|window| window == ["--bind", config_string.as_str(), config_string.as_str()])
    );
    assert!(
        wrapped
            .windows(3)
            .any(|window| window == ["--bind", gh_string.as_str(), gh_string.as_str()])
    );
    assert!(
        !wrapped
            .windows(3)
            .any(|window| window == ["--ro-bind", gh_string.as_str(), gh_string.as_str()])
    );
}

#[test]
fn exact_write_lifts_a_granted_credential_path() {
    let fixture = Fixture::new();
    let gh = fixture.home.join(".config/gh");
    std::fs::create_dir_all(&gh).expect("gh directory");
    std::fs::write(gh.join("hosts.yml"), "github.com:\n").expect("gh hosts");
    let gh = std::fs::canonicalize(gh).expect("gh canonical");
    let plan = confine::plan(
        &ConfineRequest {
            write: vec!["~/.config/gh".into()],
            grants: vec!["gh".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("exact-write plan");
    assert!(plan.writable.contains(&gh));
    assert!(!plan.readonly.contains(&gh));
}

#[test]
fn cargo_grant_keeps_credentials_hidden_and_path_bin_protected() {
    let fixture = Fixture::new();
    let cargo = fixture.home.join(".cargo");
    let credentials = cargo.join("credentials.toml");
    let cargo_bin = cargo.join("bin");
    std::fs::create_dir_all(&cargo_bin).expect("cargo bin");
    std::fs::write(&credentials, "credentials").expect("cargo credentials");
    std::fs::write(cargo_bin.join("cargo-install"), "binary").expect("cargo binary");
    let path_var = OsString::from(format!("{}:/usr/bin", cargo_bin.display()));
    let plan = confine::plan_with_path(
        &ConfineRequest {
            grants: vec!["cargo".into()],
            ..ConfineRequest::default()
        },
        confine::PlanTarget {
            harness: Harness::Generic,
            cwd: &fixture.cwd,
            host: &fixture.host(),
        },
        Some(path_var),
    )
    .expect("cargo grant plan");
    let cargo = std::fs::canonicalize(cargo).expect("cargo canonical");
    let credentials = std::fs::canonicalize(credentials).expect("credentials canonical");
    let cargo_bin = std::fs::canonicalize(cargo_bin).expect("cargo bin canonical");
    assert!(plan.writable.contains(&cargo));
    assert!(plan.hidden_paths().contains(&credentials));
    assert!(plan.protected.contains(&cargo_bin));
    assert!(!plan.writable.contains(&credentials));
}

#[test]
fn cargo_token_grant_lifts_only_its_credential_files() {
    let fixture = Fixture::new();
    let cargo = fixture.home.join(".cargo");
    let credentials = cargo.join("credentials");
    let credentials_toml = cargo.join("credentials.toml");
    std::fs::create_dir_all(&cargo).expect("cargo directory");
    std::fs::write(&credentials, "credentials").expect("cargo credentials");
    std::fs::write(&credentials_toml, "credentials").expect("cargo credentials toml");
    let cargo = std::fs::canonicalize(cargo).expect("cargo canonical");
    let credentials = std::fs::canonicalize(credentials).expect("credentials canonical");
    let credentials_toml =
        std::fs::canonicalize(credentials_toml).expect("credentials toml canonical");

    let cargo_only = confine::plan(
        &ConfineRequest {
            grants: vec!["cargo".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("cargo-only grant plan");
    assert!(cargo_only.writable.contains(&cargo));
    assert!(cargo_only.hidden_paths().contains(&credentials));
    assert!(cargo_only.hidden_paths().contains(&credentials_toml));

    let cargo_and_token = confine::plan(
        &ConfineRequest {
            grants: vec!["cargo".into(), "cargo-token".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("cargo and token grant plan");
    for path in [&cargo, &credentials, &credentials_toml] {
        assert!(
            cargo_and_token.writable.contains(path),
            "not writable: {path:?}"
        );
        assert!(
            !cargo_and_token.hidden_paths().contains(path),
            "still hidden: {path:?}"
        );
    }

    let token_only = confine::plan(
        &ConfineRequest {
            grants: vec!["cargo-token".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("token-only grant plan");
    assert!(!token_only.writable.contains(&cargo));
    assert!(token_only.writable.contains(&credentials));
    assert!(token_only.writable.contains(&credentials_toml));
}
#[test]
fn cache_grant_keeps_the_default_hides() {
    let fixture = Fixture::new();
    let cache = fixture.home.join(".cache");
    let token = cache.join("huggingface/token");
    std::fs::create_dir_all(token.parent().expect("token parent")).expect("cache directories");
    std::fs::write(&token, "token").expect("huggingface token");
    let cache = std::fs::canonicalize(cache).expect("cache canonical");
    let token = std::fs::canonicalize(token).expect("token canonical");

    let plan = confine::plan(
        &ConfineRequest {
            grants: vec!["cache".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("cache grant plan");
    assert!(plan.writable.contains(&cache));
    assert!(plan.hidden_paths().contains(&token));
}

#[test]
fn huggingface_token_grant_opens_only_the_token() {
    let fixture = Fixture::new();
    let cache = fixture.home.join(".cache");
    let token = cache.join("huggingface/token");
    std::fs::create_dir_all(token.parent().expect("token parent")).expect("cache directories");
    std::fs::write(&token, "token").expect("huggingface token");
    let cache = std::fs::canonicalize(cache).expect("cache canonical");
    let token = std::fs::canonicalize(token).expect("token canonical");

    let cache_and_token = confine::plan(
        &ConfineRequest {
            grants: vec!["cache".into(), "huggingface-token".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("cache and token grant plan");
    assert!(cache_and_token.writable.contains(&cache));
    assert!(!cache_and_token.hidden_paths().contains(&token));
    let wrapped = cache_and_token
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("cache and token wrapper");
    for name in ["HF_TOKEN", "HUGGING_FACE_HUB_TOKEN"] {
        assert!(
            !wrapped
                .windows(2)
                .any(|window| window == ["--unsetenv", name]),
            "{name} remains unset"
        );
    }

    let token_only = confine::plan(
        &ConfineRequest {
            grants: vec!["huggingface-token".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("token-only grant plan");
    assert!(!token_only.writable.contains(&cache));
    assert!(token_only.writable.contains(&token));
    assert!(!token_only.hidden_paths().contains(&token));
}

#[test]
fn cache_grant_with_narrower_grants() {
    let fixture = Fixture::new();
    let cache = fixture.home.join(".cache");
    for path in [
        cache.join("go-build"),
        cache.join("pip"),
        cache.join("uv"),
        cache.join("pypoetry"),
        cache.join("virtualenv"),
        fixture.home.join(".local/share/uv"),
        fixture.home.join(".local/share/pipx"),
    ] {
        std::fs::create_dir_all(path).expect("cache grant directory");
    }
    let cache = std::fs::canonicalize(cache).expect("cache canonical");
    let plan = confine::plan(
        &ConfineRequest {
            grants: vec!["cache".into(), "go".into(), "python".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("overlapping cache grants plan");
    assert!(plan.writable.contains(&cache));
}

#[test]
fn cache_grant_refuses_a_state_directory_under_it() {
    let fixture = Fixture::new();
    let cache = fixture.home.join(".cache");
    std::fs::create_dir_all(&cache).expect("cache directory");
    let state = cache.join("state");
    let layout = a2amx::layout::Layout::new(state);
    layout.ensure().unwrap();
    let _empty = confine::ensure_empty_file(&layout).expect("cache state empty file");
    let mut host = fixture.host();
    host.layout = layout;
    let request = ConfineRequest {
        grants: vec!["cache".into()],
        ..ConfineRequest::default()
    };
    let error = match confine::plan(&request, Harness::Generic, &fixture.cwd, &host) {
        Ok(_) => panic!("cache grant must reject a nested state directory"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("overlaps the a2amx directory"));

    let raw_write = confine::plan(
        &ConfineRequest {
            write: vec!["~/.cache".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &host,
    );
    assert!(raw_write.is_ok());
}

#[test]
fn cache_grant_preflight_refuses_a_state_directory_under_it() {
    let fixture = Fixture::new();
    let cache = fixture.home.join(".cache");
    std::fs::create_dir_all(&cache).expect("cache directory");
    let state = cache.join("state");
    let layout = a2amx::layout::Layout::new(state);
    layout.ensure().unwrap();
    let _empty = confine::ensure_empty_file(&layout).expect("cache state empty file");
    let mut host = fixture.host();
    host.layout = layout;
    let request = ConfineRequest {
        grants: vec!["cache".into()],
        ..ConfineRequest::default()
    };
    let preflight = confine::ensure_grant_dirs(&request, Harness::Generic, &host, true);
    let error = match confine::plan_with_preflight(
        &request,
        confine::PlanTarget {
            harness: Harness::Generic,
            cwd: &fixture.cwd,
            host: &host,
        },
        None,
        preflight,
    ) {
        Ok(_) => panic!("cache grant must reject a nested state directory"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("overlaps the a2amx directory"));
}

#[test]
fn missing_node_grant_path_is_skipped_after_preflight_without_creation() {
    let fixture = Fixture::new();
    let npm = fixture.home.join(".npm");
    let request = ConfineRequest {
        grants: vec!["node".into()],
        ..ConfineRequest::default()
    };
    let preflight = confine::ensure_grant_dirs(&request, Harness::Generic, &fixture.host(), false);
    assert!(preflight.is_empty());
    let plan = confine::plan_with_preflight(
        &request,
        confine::PlanTarget {
            harness: Harness::Generic,
            cwd: &fixture.cwd,
            host: &fixture.host(),
        },
        None,
        preflight,
    )
    .expect("missing node grant path plan");
    assert!(!npm.exists());
    assert!(plan.skipped_paths().contains(&npm));
}
#[test]
fn gh_grant_opens_only_its_paths_and_conflicts_with_exact_hides() {
    let fixture = Fixture::new();
    let config = fixture.home.join(".config");
    let gh = config.join("gh");
    std::fs::create_dir_all(&gh).expect("gh directory");
    std::fs::write(gh.join("hosts.yml"), "github.com:\n").expect("gh hosts");
    std::fs::write(
        fixture.home.join(".git-credentials"),
        "https://user:pass@example\n",
    )
    .expect("git credentials");
    let git_config = config.join("git");
    std::fs::create_dir_all(&git_config).expect("git config directory");
    std::fs::write(
        git_config.join("credentials"),
        "https://user:pass@example\n",
    )
    .expect("git config credentials");
    let gh = std::fs::canonicalize(gh).expect("gh canonical");
    let config = std::fs::canonicalize(config).expect("config canonical");
    let plan = confine::plan(
        &ConfineRequest {
            grants: vec!["gh".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("gh-only plan");
    assert!(!plan.hidden_paths().contains(&gh));
    assert!(plan.hidden_paths().contains(&fixture.home.join(".ssh")));
    assert!(!plan.writable.contains(&config));
    assert!(!plan.readonly.contains(&config));
    assert!(
        plan.hidden_paths()
            .contains(&fixture.home.join(".git-credentials"))
    );
    assert!(
        plan.hidden_paths()
            .iter()
            .any(|path| path.ends_with(".config/git/credentials"))
    );

    for request in [
        ConfineRequest {
            grants: vec!["gh".into()],
            hide: vec!["~/.config/gh".into()],
            ..ConfineRequest::default()
        },
        ConfineRequest {
            grants: vec!["gh".into()],
            hide: vec!["~/.config/gh".into()],
            write: vec!["~/.config/gh".into()],
            ..ConfineRequest::default()
        },
    ] {
        let error = match confine::plan(&request, Harness::Generic, &fixture.cwd, &fixture.host()) {
            Ok(_) => panic!("exact hide must conflict with gh grant"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("confinement grant \"gh\" conflicts with the hide of"),
            "unexpected conflict: {error}"
        );
    }
}

#[test]
fn git_credentials_grant_lifts_both_files_only() {
    let fixture = Fixture::new();
    let gh = fixture.home.join(".config/gh");
    std::fs::create_dir_all(&gh).expect("gh directory");
    std::fs::write(gh.join("hosts.yml"), "github.com:\n").expect("gh hosts");
    std::fs::write(
        fixture.home.join(".git-credentials"),
        "https://user:pass@example\n",
    )
    .expect("git credentials");
    let git_config = fixture.home.join(".config/git");
    std::fs::create_dir_all(&git_config).expect("git config directory");
    std::fs::write(
        git_config.join("credentials"),
        "https://user:pass@example\n",
    )
    .expect("git config credentials");
    let git_credentials = std::fs::canonicalize(fixture.home.join(".git-credentials"))
        .expect("git credentials canonical");
    let git_config_credentials =
        std::fs::canonicalize(git_config.join("credentials")).expect("git config credentials");
    let gh = std::fs::canonicalize(gh).expect("gh canonical");
    let plan = confine::plan(
        &ConfineRequest {
            grants: vec!["git-credentials".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("git credentials plan");
    for path in [&git_credentials, &git_config_credentials] {
        assert!(plan.writable.contains(path));
        assert!(!plan.readonly.contains(path));
        assert!(!plan.protected.contains(path));
    }
    assert!(!plan.hidden_paths().contains(&git_credentials));
    assert!(!plan.hidden_paths().contains(&git_config_credentials));
    assert!(plan.hidden_paths().contains(&gh));
    let wrapped = plan
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("git credentials wrapper");
    for path in [&git_credentials, &git_config_credentials] {
        let path = path.to_string_lossy();
        assert!(
            wrapped
                .windows(3)
                .any(|window| { window[0] == "--bind" && window[1] == path && window[2] == path })
        );
        assert!(
            !wrapped.windows(3).any(|window| {
                window[0] == "--ro-bind" && window[1] == path && window[2] == path
            })
        );
    }
}

#[test]
fn granted_paths_may_be_missing_without_becoming_unresolved() {
    let fixture = Fixture::new();
    let missing = fixture.home.join(".config/gh");
    let plan = confine::plan(
        &ConfineRequest {
            grants: vec!["gh".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("missing credential path plan");
    assert!(plan.unresolved.is_empty());
    assert!(plan.skipped_paths().contains(&missing));
}
