use super::*;

#[test]
fn wrap_unsets_base_and_container_environment_in_stable_order() {
    let fixture = Fixture::new();
    let restricted = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("plan")
    .with_bwrap(PathBuf::from("bwrap"))
    .wrap(vec!["sh".into(), "-c".into(), "true".into()])
    .expect("wrap");
    for name in [
        "DBUS_SESSION_BUS_ADDRESS",
        "SSH_AUTH_SOCK",
        "GPG_AGENT_INFO",
        "WSL_INTEROP",
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "DOCKER_HOST",
        "DOCKER_CONTEXT",
        "CONTAINER_HOST",
        "CONTAINER_CONNECTION",
        "CONTAINERD_ADDRESS",
        "BUILDKIT_HOST",
        "LIBVIRT_DEFAULT_URI",
    ] {
        assert!(
            restricted
                .windows(2)
                .any(|window| window == ["--unsetenv", name])
        );
    }
    let allowed = confine::plan(
        &ConfineRequest {
            grants: vec!["docker".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("plan")
    .with_bwrap(PathBuf::from("bwrap"))
    .wrap(vec!["true".into()])
    .expect("wrap");
    assert!(
        !allowed
            .windows(2)
            .any(|window| window == ["--unsetenv", "DOCKER_HOST"])
    );
    assert!(
        !allowed
            .windows(2)
            .any(|window| window == ["--unsetenv", "DOCKER_CONTEXT"])
    );
    for name in [
        "CONTAINER_HOST",
        "CONTAINER_CONNECTION",
        "CONTAINERD_ADDRESS",
        "BUILDKIT_HOST",
        "LIBVIRT_DEFAULT_URI",
    ] {
        assert!(
            allowed
                .windows(2)
                .any(|window| window == ["--unsetenv", name])
        );
    }
    assert!(
        allowed
            .windows(2)
            .any(|window| window == ["--tmpfs", fixture.runtime.to_string_lossy().as_ref()])
    );
}
#[test]
fn wrap_unsets_every_credential_environment_variable_by_default() {
    let fixture = Fixture::new();
    let wrapped = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("plan")
    .with_bwrap(PathBuf::from("bwrap"))
    .wrap(vec!["true".into()])
    .expect("wrap");
    for name in [
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "GH_ENTERPRISE_TOKEN",
        "GITLAB_TOKEN",
        "GITLAB_PRIVATE_TOKEN",
        "CI_JOB_TOKEN",
        "BITBUCKET_TOKEN",
        "NPM_TOKEN",
        "NODE_AUTH_TOKEN",
        "PYPI_TOKEN",
        "TWINE_PASSWORD",
        "CARGO_REGISTRY_TOKEN",
        "CARGO_REGISTRIES_CRATES_IO_TOKEN",
        "AZURE_CLIENT_SECRET",
        "ARM_CLIENT_SECRET",
        "VAULT_TOKEN",
        "HF_TOKEN",
        "HUGGING_FACE_HUB_TOKEN",
        "SONAR_TOKEN",
        "SENTRY_AUTH_TOKEN",
        "DIGITALOCEAN_ACCESS_TOKEN",
        "HCLOUD_TOKEN",
        "CLOUDFLARE_API_TOKEN",
        "NETLIFY_AUTH_TOKEN",
        "VERCEL_TOKEN",
        "FLY_API_TOKEN",
        "HEROKU_API_KEY",
        "PGPASSWORD",
        "MYSQL_PWD",
    ] {
        assert!(
            wrapped
                .windows(2)
                .any(|window| window == ["--unsetenv", name]),
            "missing credential environment variable {name}"
        );
    }
    for name in [
        "ANTHROPIC_API_KEY",
        "A2AMX_TOKEN",
        "A2AMX_ADDR",
        "OPENAI_API_KEY",
    ] {
        assert!(
            !wrapped
                .windows(2)
                .any(|window| window == ["--unsetenv", name]),
            "harness environment variable {name} was unset"
        );
    }
}
#[test]
fn wrap_keeps_requested_credentials_and_never_unsets_harness_environment() {
    let fixture = Fixture::new();
    let kept = confine::plan(
        &ConfineRequest {
            keep_env: vec!["GITHUB_TOKEN".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("kept plan")
    .with_bwrap(PathBuf::from("bwrap"))
    .wrap(vec!["true".into()])
    .expect("kept wrap");
    for name in [
        "GH_TOKEN",
        "GH_ENTERPRISE_TOKEN",
        "GITLAB_TOKEN",
        "GITLAB_PRIVATE_TOKEN",
        "CI_JOB_TOKEN",
        "BITBUCKET_TOKEN",
        "NPM_TOKEN",
        "NODE_AUTH_TOKEN",
        "PYPI_TOKEN",
        "TWINE_PASSWORD",
        "CARGO_REGISTRY_TOKEN",
        "CARGO_REGISTRIES_CRATES_IO_TOKEN",
        "AZURE_CLIENT_SECRET",
        "ARM_CLIENT_SECRET",
        "VAULT_TOKEN",
        "HF_TOKEN",
        "HUGGING_FACE_HUB_TOKEN",
        "SONAR_TOKEN",
        "SENTRY_AUTH_TOKEN",
        "DIGITALOCEAN_ACCESS_TOKEN",
        "HCLOUD_TOKEN",
        "CLOUDFLARE_API_TOKEN",
        "NETLIFY_AUTH_TOKEN",
        "VERCEL_TOKEN",
        "FLY_API_TOKEN",
        "HEROKU_API_KEY",
        "PGPASSWORD",
        "MYSQL_PWD",
    ] {
        assert!(
            kept.windows(2).any(|window| window == ["--unsetenv", name]),
            "missing unset for non-kept credential {name}"
        );
    }
    assert!(
        !kept
            .windows(2)
            .any(|window| window == ["--unsetenv", "GITHUB_TOKEN"])
    );
    for name in [
        "ANTHROPIC_API_KEY",
        "A2AMX_TOKEN",
        "A2AMX_ADDR",
        "OPENAI_API_KEY",
    ] {
        assert!(
            !kept.windows(2).any(|window| window == ["--unsetenv", name]),
            "harness environment variable {name} was unset"
        );
    }
}
#[test]
fn plan_sorts_kept_credential_names() {
    let fixture = Fixture::new();
    let plan = confine::plan(
        &ConfineRequest {
            keep_env: vec!["NPM_TOKEN".into(), "GH_TOKEN".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("plan");
    assert_eq!(plan.keep_env, ["GH_TOKEN", "NPM_TOKEN"]);
}

#[test]
fn keep_env_validation_is_closed_and_bounded() {
    let fixture = Fixture::new();
    let error_for = |keep_env| match confine::plan(
        &ConfineRequest {
            keep_env,
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    ) {
        Ok(_) => panic!("keep_env should be rejected"),
        Err(error) => error.to_string(),
    };
    assert!(error_for(vec!["ANTHROPIC_API_KEY".into()]).contains(
        "keep_env \"ANTHROPIC_API_KEY\" is not unset by default, so there is nothing to keep"
    ));
    assert!(error_for(vec!["DOCKER_HOST".into()]).contains(
        "keep_env \"DOCKER_HOST\" is not unset by default, so there is nothing to keep; it returns with the docker grant"
    ));
    assert_eq!(
        error_for(vec!["GITHUB_TOKEN".into(), "GITHUB_TOKEN".into()]),
        "keep_env \"GITHUB_TOKEN\" is repeated"
    );
    assert_eq!(
        error_for(vec!["GITHUB_TOKEN".into(); 65]),
        "keep_env has more than 64 entries"
    );
}

#[test]
fn notice_lists_credential_environment_variables_and_kept_names() {
    let fixture = Fixture::new();
    let default_notice = confine::notice(
        &confine::plan(
            &ConfineRequest::default(),
            Harness::Generic,
            &fixture.cwd,
            &fixture.host(),
        )
        .expect("default plan"),
    );
    assert!(default_notice.contains(
        "Credential environment variables are unset: GH_TOKEN, GITHUB_TOKEN, GH_ENTERPRISE_TOKEN, GITLAB_TOKEN, GITLAB_PRIVATE_TOKEN, CI_JOB_TOKEN, BITBUCKET_TOKEN, NPM_TOKEN, NODE_AUTH_TOKEN, PYPI_TOKEN, TWINE_PASSWORD, CARGO_REGISTRY_TOKEN, CARGO_REGISTRIES_CRATES_IO_TOKEN, AZURE_CLIENT_SECRET, ARM_CLIENT_SECRET, VAULT_TOKEN, HF_TOKEN, HUGGING_FACE_HUB_TOKEN, SONAR_TOKEN, SENTRY_AUTH_TOKEN, DIGITALOCEAN_ACCESS_TOKEN, HCLOUD_TOKEN, CLOUDFLARE_API_TOKEN, NETLIFY_AUTH_TOKEN, VERCEL_TOKEN, FLY_API_TOKEN, HEROKU_API_KEY, PGPASSWORD, MYSQL_PWD."
    ));
    let kept_notice = confine::notice(
        &confine::plan(
            &ConfineRequest {
                keep_env: vec!["GITHUB_TOKEN".into()],
                ..ConfineRequest::default()
            },
            Harness::Generic,
            &fixture.cwd,
            &fixture.host(),
        )
        .expect("kept plan"),
    );
    assert!(kept_notice.contains(
        "Credential environment variables are unset: GH_TOKEN, GH_ENTERPRISE_TOKEN, GITLAB_TOKEN, GITLAB_PRIVATE_TOKEN, CI_JOB_TOKEN, BITBUCKET_TOKEN, NPM_TOKEN, NODE_AUTH_TOKEN, PYPI_TOKEN, TWINE_PASSWORD, CARGO_REGISTRY_TOKEN, CARGO_REGISTRIES_CRATES_IO_TOKEN, AZURE_CLIENT_SECRET, ARM_CLIENT_SECRET, VAULT_TOKEN, HF_TOKEN, HUGGING_FACE_HUB_TOKEN, SONAR_TOKEN, SENTRY_AUTH_TOKEN, DIGITALOCEAN_ACCESS_TOKEN, HCLOUD_TOKEN, CLOUDFLARE_API_TOKEN, NETLIFY_AUTH_TOKEN, VERCEL_TOKEN, FLY_API_TOKEN, HEROKU_API_KEY, PGPASSWORD, MYSQL_PWD; kept: GITHUB_TOKEN."
    ));
}

#[test]
fn wrap_enables_user_ipc_and_pid_namespaces() {
    let fixture = Fixture::new();
    let wrapped = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("plan")
    .with_bwrap(PathBuf::from("bwrap"))
    .wrap(vec!["true".into()])
    .expect("wrap");
    for flag in ["--unshare-user", "--unshare-ipc", "--unshare-pid"] {
        assert!(wrapped.iter().any(|argument| argument == flag), "{flag}");
    }
}

#[test]
fn remount_ro_options_follow_all_mount_operations() {
    let fixture = Fixture::new();
    let hidden = fixture.cwd.join("hidden");
    std::fs::create_dir_all(&hidden).expect("hidden directory");
    let plan = confine::plan(
        &ConfineRequest {
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
        .expect("wrap");
    let last_mount = wrapped
        .iter()
        .enumerate()
        .filter(|(_, argument)| {
            matches!(
                argument.as_str(),
                "--bind" | "--ro-bind" | "--tmpfs" | "--dev-bind" | "--proc" | "--dir"
            )
        })
        .map(|(index, _)| index)
        .max()
        .expect("mount operation");
    let remounts = wrapped
        .iter()
        .enumerate()
        .filter(|(_, argument)| argument.as_str() == "--remount-ro")
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    assert!(!remounts.is_empty());
    assert!(remounts.iter().all(|index| *index > last_mount));
}

#[test]
fn unhide_cannot_lift_container_socket_restriction() {
    let fixture = Fixture::new();
    let docker_socket = fixture.home.join(".docker/run/docker.sock");
    std::fs::create_dir_all(docker_socket.parent().expect("docker parent")).expect("docker dir");
    std::fs::write(&docker_socket, "socket placeholder").expect("docker socket fixture");
    let error = match confine::plan(
        &ConfineRequest {
            unhide: vec!["~/.docker/run/docker.sock".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    ) {
        Ok(_) => panic!("container socket unhide must be refused"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("docker grant"));
    let wrong_grant = match confine::plan(
        &ConfineRequest {
            unhide: vec!["~/.docker/run/docker.sock".into()],
            grants: vec!["podman".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    ) {
        Ok(_) => panic!("podman must not authorize a docker socket"),
        Err(error) => error,
    };
    assert!(
        wrong_grant
            .to_string()
            .contains("requires the docker grant")
    );
    let allowed = confine::plan(
        &ConfineRequest {
            unhide: vec!["~/.docker/run/docker.sock".into()],
            grants: vec!["docker".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("allow-docker switch");
    assert!(
        allowed
            .readonly
            .contains(&std::fs::canonicalize(&docker_socket).expect("socket canonical"))
    );
    let plan = confine::plan(
        &ConfineRequest {
            unhide: vec!["~/.docker".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("parent unhide");
    assert!(
        plan.hidden_paths()
            .contains(&std::fs::canonicalize(docker_socket).expect("socket canonical"))
    );
}
#[test]
fn state_omp_cannot_be_operator_unhidden() {
    let fixture = Fixture::new();
    let omp_state = fixture.host().layout.omp_dir();
    std::fs::create_dir_all(&omp_state).expect("omp state");
    for harness in [Harness::Generic, Harness::Claude] {
        let result = confine::plan(
            &ConfineRequest {
                unhide: vec![omp_state.to_string_lossy().into_owned()],
                ..ConfineRequest::default()
            },
            harness,
            &fixture.cwd,
            &fixture.host(),
        );
        assert!(result.is_err(), "{harness:?} must not unhide state/omp");
    }
    let plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Omp,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("omp plan");
    let wrapped = plan
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("wrapped omp plan");
    let state = std::fs::canonicalize(omp_state).expect("omp state canonical");
    assert!(wrapped.windows(3).any(|window| {
        window
            == [
                "--ro-bind",
                state.to_string_lossy().as_ref(),
                state.to_string_lossy().as_ref(),
            ]
    }));
}

#[test]
fn unhide_guard_canonicalizes_symlinked_state_runtime_and_container_paths() {
    let fixture = Fixture::new();
    let state_link = fixture.cwd.join("state-link");
    let runtime_link = fixture.cwd.join("runtime-link");
    symlink(&fixture.state, &state_link).expect("state symlink");
    symlink(&fixture.runtime, &runtime_link).expect("runtime symlink");
    for link in [&state_link, &runtime_link] {
        assert!(
            confine::check_unhide_allowed(link, &fixture.state, &fixture.runtime, &fixture.home)
                .is_err(),
            "symlinked protected path must be refused: {}",
            link.display()
        );
    }

    let socket = fixture.home.join(".docker/run/docker.sock");
    std::fs::create_dir_all(socket.parent().expect("socket parent")).expect("socket directory");
    std::fs::write(&socket, "socket").expect("socket fixture");
    let socket_link = fixture.cwd.join("socket-link");
    symlink(&socket, &socket_link).expect("socket symlink");
    assert!(
        confine::plan(
            &ConfineRequest {
                unhide: vec![socket_link.to_string_lossy().into_owned()],
                ..ConfineRequest::default()
            },
            Harness::Generic,
            &fixture.cwd,
            &fixture.host()
        )
        .is_err(),
        "symlinked container socket must be refused"
    );

    let secret = fixture._root.path().join("secret");
    std::fs::write(&secret, "secret").expect("secret fixture");
    let secret_link = fixture.cwd.join("secret-link");
    symlink(&secret, &secret_link).expect("secret symlink");
    let plan = confine::plan(
        &ConfineRequest {
            unhide: vec![secret_link.to_string_lossy().into_owned()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("symlinked secret unhide");
    let secret = std::fs::canonicalize(secret).expect("secret canonical");
    assert!(plan.readonly.contains(&secret));
    assert!(!plan.readonly.contains(&secret_link));
}

#[test]
fn writable_paths_override_exact_hides_but_not_container_sockets() {
    let fixture = Fixture::new();
    let ssh = std::fs::canonicalize(fixture.home.join(".ssh")).expect("ssh canonical");
    let plan = confine::plan(
        &ConfineRequest {
            write: vec!["~/.ssh".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("ssh write");
    assert!(plan.writable.contains(&ssh));
    assert!(!plan.hidden_paths().contains(&ssh));
    assert!(!plan.default_hidden.contains(&ssh));

    let socket = fixture.home.join(".docker/run/docker.sock");
    std::fs::create_dir_all(socket.parent().expect("socket parent")).expect("socket directory");
    std::fs::write(&socket, "socket placeholder").expect("socket fixture");
    let error = match confine::plan(
        &ConfineRequest {
            write: vec!["~/.docker/run/docker.sock".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    ) {
        Ok(_) => panic!("container socket write must be refused"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("docker grant"));
    let allowed = confine::plan(
        &ConfineRequest {
            write: vec!["~/.docker/run/docker.sock".into()],
            grants: vec!["docker".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("container socket write allowed");
    assert!(
        allowed
            .writable
            .contains(&std::fs::canonicalize(socket).expect("socket canonical"))
    );
}

#[test]
fn container_socket_hides_are_bound_and_can_be_disabled() {
    let fixture = Fixture::new();
    let sockets = [
        fixture.home.join(".docker/run/docker.sock"),
        fixture.home.join(".docker/desktop/docker.sock"),
    ];
    for socket in &sockets {
        std::fs::create_dir_all(socket.parent().expect("socket parent")).expect("socket dir");
        std::fs::write(socket, "socket placeholder").expect("socket fixture");
    }
    let plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("container hide plan");
    for socket in &sockets {
        let socket = std::fs::canonicalize(socket).expect("socket canonical");
        assert!(plan.hidden_paths().contains(&socket));
    }
    let wrapped = plan
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("wrapped container plan");
    for socket in &sockets {
        let socket = std::fs::canonicalize(socket).expect("socket canonical");
        assert!(wrapped.windows(3).any(|window| {
            window
                == [
                    "--ro-bind",
                    fixture.empty.to_string_lossy().as_ref(),
                    socket.to_string_lossy().as_ref(),
                ]
        }));
    }
    let allowed = confine::plan(
        &ConfineRequest {
            grants: vec!["docker".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("allow docker plan");
    for socket in &sockets {
        let socket = std::fs::canonicalize(socket).expect("socket canonical");
        assert!(!allowed.hidden_paths().contains(&socket));
    }
    let allowed_wrapped = allowed
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("wrapped allow docker plan");
    for socket in &sockets {
        let socket = std::fs::canonicalize(socket).expect("socket canonical");
        assert!(
            !allowed_wrapped
                .iter()
                .any(|argument| argument == socket.to_string_lossy().as_ref())
        );
    }
}
