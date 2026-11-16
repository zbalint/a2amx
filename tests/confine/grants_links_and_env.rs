use super::*;

#[test]
fn grant_alias_to_hidden_or_unhidden_path_is_rejected() {
    let fixture = Fixture::new();
    let config = fixture.home.join(".config");
    std::fs::create_dir_all(&config).expect("config directory");
    let hidden_alias = config.join("gh");
    symlink(fixture.home.join(".ssh"), &hidden_alias).expect("gh hidden alias");
    let error = match confine::plan(
        &ConfineRequest {
            grants: vec!["gh".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    ) {
        Ok(_) => panic!("gh alias to ssh must fail"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("confinement grant \"gh\" path ~/.config/gh passes through the symlink")
    );
    assert!(
        error
            .to_string()
            .contains(&hidden_alias.display().to_string())
    );

    let plain = Fixture::new();
    let plain_config = plain.home.join(".config");
    let target = plain.home.join("unhidden-target");
    std::fs::create_dir_all(&plain_config).expect("plain config directory");
    std::fs::create_dir_all(&target).expect("plain target");
    let plain_alias = plain_config.join("gh");
    symlink(&target, &plain_alias).expect("gh plain alias");
    let error = match confine::plan(
        &ConfineRequest {
            grants: vec!["gh".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &plain.cwd,
        &plain.host(),
    ) {
        Ok(_) => panic!("gh alias to a plain path must fail"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("confinement grant \"gh\" path ~/.config/gh passes through the symlink")
    );
    assert!(
        error
            .to_string()
            .contains(&plain_alias.display().to_string())
    );
}

#[test]
fn grant_symlink_paths_are_rejected_for_toolchain_and_token_grants() {
    for (grant_name, entry, target_is_file) in [
        ("python", "~/.cache/pip", false),
        ("cargo", "~/.cargo", false),
        ("npm-token", "~/.npmrc", true),
    ] {
        let fixture = Fixture::new();
        let source = fixture
            .home
            .join(entry.strip_prefix("~/").expect("home entry"));
        let target = fixture._root.path().join("plain-target");
        if target_is_file {
            std::fs::write(&target, "target").expect("plain target file");
        } else {
            std::fs::create_dir_all(&target).expect("plain target directory");
        }
        std::fs::create_dir_all(source.parent().expect("symlink parent"))
            .expect("symlink parent directory");
        symlink(&target, &source).expect("grant symlink");
        let error = match confine::plan(
            &ConfineRequest {
                grants: vec![grant_name.into()],
                ..ConfineRequest::default()
            },
            Harness::Generic,
            &fixture.cwd,
            &fixture.host(),
        ) {
            Ok(_) => panic!("{grant_name} symlink must fail"),
            Err(error) => error,
        };
        let text = error.to_string();
        assert!(
            text.contains(&format!(
                "confinement grant \"{grant_name}\" path {entry} passes through the symlink"
            )),
            "unexpected error: {text}"
        );
        assert!(
            text.contains(&source.display().to_string()),
            "unexpected error: {text}"
        );
    }
}

#[test]
fn grant_symlinked_home_ancestor_is_accepted_after_home_canonicalization() {
    let fixture = Fixture::new();
    let config = fixture.home.join(".config/gh");
    std::fs::create_dir_all(&config).expect("gh directory");
    let home_link = fixture._root.path().join("home-link");
    symlink(&fixture.home, &home_link).expect("home link");
    let mut host = fixture.host();
    host.home = home_link;
    let plan = confine::plan(
        &ConfineRequest {
            grants: vec!["gh".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &host,
    )
    .expect("canonical home plan");
    assert!(
        plan.writable
            .contains(&std::fs::canonicalize(config).expect("gh canonical"))
    );
}

#[test]
fn grant_alias_to_home_is_rejected_as_a_symlink() {
    let fixture = Fixture::new();
    let config = fixture.home.join(".config");
    std::fs::create_dir_all(&config).expect("config directory");
    let alias = config.join("gh");
    symlink(&fixture.home, &alias).expect("home alias");
    let error = match confine::plan(
        &ConfineRequest {
            grants: vec!["gh".into()],
            no_default_hides: true,
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    ) {
        Ok(_) => panic!("grant alias to home must be rejected"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("passes through the symlink"));
    assert!(error.to_string().contains(&alias.display().to_string()));
}

#[test]
fn grant_alias_into_state_is_rejected_as_a_symlink() {
    let fixture = Fixture::new();
    let target = fixture.state.join("inside");
    std::fs::create_dir_all(&target).expect("state target");
    let config = fixture.home.join(".config");
    std::fs::create_dir_all(&config).expect("config directory");
    let alias = config.join("gh");
    symlink(&target, &alias).expect("state alias");
    let error = match confine::plan(
        &ConfineRequest {
            grants: vec!["gh".into()],
            no_default_hides: true,
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    ) {
        Ok(_) => panic!("grant alias into state must be rejected"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("passes through the symlink"));
    assert!(error.to_string().contains(&alias.display().to_string()));
}

#[test]
fn grant_alias_into_install_or_harness_is_rejected_as_a_symlink() {
    let install = Fixture::new();
    let install_target = install.home.join(".a2amx/bin");
    std::fs::create_dir_all(&install_target).expect("install target");
    let install_config = install.home.join(".config");
    std::fs::create_dir_all(&install_config).expect("config directory");
    let install_alias = install_config.join("gh");
    symlink(&install_target, &install_alias).expect("install alias");
    let error = match confine::plan(
        &ConfineRequest {
            grants: vec!["gh".into()],
            no_default_hides: true,
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &install.cwd,
        &install.host(),
    ) {
        Ok(_) => panic!("grant alias into .a2amx must be rejected"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("passes through the symlink"));
    assert!(
        error
            .to_string()
            .contains(&install_alias.display().to_string())
    );

    let harness = Fixture::new();
    let harness_root = harness.home.join(".claude");
    std::fs::create_dir_all(&harness_root).expect("Claude root");
    let harness_config = harness.home.join(".config");
    std::fs::create_dir_all(&harness_config).expect("config directory");
    let harness_alias = harness_config.join("gh");
    symlink(&harness_root, &harness_alias).expect("harness alias");
    let error = match confine::plan(
        &ConfineRequest {
            grants: vec!["gh".into()],
            no_default_hides: true,
            ..ConfineRequest::default()
        },
        Harness::Claude,
        &harness.cwd,
        &harness.host(),
    ) {
        Ok(_) => panic!("grant alias into harness root must be rejected"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("passes through the symlink"));
    assert!(
        error
            .to_string()
            .contains(&harness_alias.display().to_string())
    );
}

#[test]
fn grant_alias_to_path_directory_is_rejected_as_a_symlink() {
    let fixture = Fixture::new();
    let path_dir = fixture.home.join(".local/bin");
    std::fs::create_dir_all(&path_dir).expect("PATH directory");
    let config = fixture.home.join(".config");
    std::fs::create_dir_all(&config).expect("config directory");
    let alias = config.join("gh");
    symlink(&path_dir, &alias).expect("PATH alias");
    let path_var = OsString::from(format!("{}:/usr/bin", path_dir.display()));
    let error = match confine::plan_with_path(
        &ConfineRequest {
            grants: vec!["gh".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
        Some(path_var),
    ) {
        Ok(_) => panic!("grant alias into PATH must be rejected"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("passes through the symlink"));
    assert!(error.to_string().contains(&alias.display().to_string()));
}

#[test]
fn new_grants_open_only_existing_paths_and_release_only_their_environment() {
    const CREDENTIAL_ENV: [&str; 29] = [
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
    ];
    let cases: [(&str, &[&str]); 19] = [
        ("cargo", &["~/.cargo", "~/.rustup"]),
        ("go", &["~/go", "~/.cache/go-build"]),
        (
            "python",
            &[
                "~/.cache/pip",
                "~/.cache/uv",
                "~/.local/share/uv",
                "~/.cache/pypoetry",
                "~/.cache/virtualenv",
                "~/.local/share/pipx",
            ],
        ),
        (
            "node",
            &[
                "~/.npm",
                "~/.yarn/berry",
                "~/.local/share/pnpm",
                "~/.bun/install/cache",
            ],
        ),
        ("java", &["~/.m2/repository", "~/.gradle"]),
        (
            "c-cpp",
            &["~/.cache/ccache", "~/.ccache", "~/.conan2", "~/.cmake"],
        ),
        ("semgrep", &["~/.semgrep"]),
        ("sonar", &["~/.sonar"]),
        ("aws", &["~/.aws"]),
        ("gcloud", &["~/.config/gcloud"]),
        ("azure", &["~/.azure"]),
        ("kube", &["~/.kube"]),
        ("terraform", &["~/.terraform.d"]),
        ("npm-token", &["~/.npmrc"]),
        ("pypi-token", &["~/.pypirc"]),
        (
            "cargo-token",
            &["~/.cargo/credentials", "~/.cargo/credentials.toml"],
        ),
        ("netrc", &["~/.netrc"]),
        ("cache", &["~/.cache"]),
        ("huggingface-token", &["~/.cache/huggingface/token"]),
    ];

    for (grant_name, entries) in cases {
        let fixture = Fixture::new();
        let mut expected = Vec::new();
        for &entry in entries {
            let path = fixture
                .home
                .join(entry.strip_prefix("~/").expect("home-relative grant path"));
            let is_file = matches!(
                entry,
                "~/.npmrc"
                    | "~/.pypirc"
                    | "~/.netrc"
                    | "~/.cargo/credentials"
                    | "~/.cargo/credentials.toml"
                    | "~/.cache/huggingface/token"
            );
            if is_file {
                std::fs::create_dir_all(path.parent().expect("grant file parent"))
                    .expect("grant file parent");
                std::fs::write(&path, "credential").expect("grant file");
            } else {
                std::fs::create_dir_all(&path).expect("grant directory");
            }
            expected.push(std::fs::canonicalize(path).expect("grant path canonical"));
        }
        let baseline = confine::plan(
            &ConfineRequest {
                ..ConfineRequest::default()
            },
            Harness::Generic,
            &fixture.cwd,
            &fixture.host(),
        )
        .expect("baseline grant matrix plan");
        let granted = confine::plan(
            &ConfineRequest {
                grants: vec![grant_name.into()],
                ..ConfineRequest::default()
            },
            Harness::Generic,
            &fixture.cwd,
            &fixture.host(),
        )
        .expect("grant matrix plan");
        let mut added = granted
            .writable
            .iter()
            .filter(|path| !baseline.writable.contains(path))
            .cloned()
            .collect::<Vec<_>>();
        added.sort();
        expected.sort();
        assert_eq!(
            added, expected,
            "unexpected writable paths for {grant_name}"
        );

        let ssh = std::fs::canonicalize(fixture.home.join(".ssh")).expect("ssh canonical");
        assert!(
            granted.hidden_paths().contains(&ssh),
            "unrelated ssh hide lifted by {grant_name}"
        );
        let aws = std::fs::canonicalize(fixture.home.join(".aws")).expect("aws canonical");
        if grant_name == "aws" {
            assert!(!granted.hidden_paths().contains(&aws));
        } else {
            assert!(
                granted.hidden_paths().contains(&aws),
                "unrelated aws hide lifted by {grant_name}"
            );
        }

        let wrapped = granted
            .with_bwrap(PathBuf::from("bwrap"))
            .wrap(vec!["true".into()])
            .expect("grant matrix wrapper");
        let row = grant::lookup(grant_name).expect("grant row");
        for name in CREDENTIAL_ENV {
            let unset = wrapped
                .windows(2)
                .any(|window| window == ["--unsetenv", name]);
            assert_eq!(
                unset,
                !row.env.contains(&name),
                "unexpected environment release for {grant_name}: {name}"
            );
        }
    }
}

#[test]
fn team_file_cargo_token_grant_releases_environment() {
    let sessions = team::parse(
        r#"
    [confine]
    enabled = true
    grants = ["cargo-token"]
    [[session]]
    name = "worker"
    command = ["cat"]
    "#,
        std::path::Path::new("/work/demo"),
        false,
    )
    .expect("team cargo-token grant");
    let request = sessions[0]
        .confine
        .clone()
        .expect("team confinement request");
    assert_eq!(request.grants, vec!["cargo-token"]);
    let fixture = Fixture::new();
    let plan = confine::plan(&request, Harness::Generic, &fixture.cwd, &fixture.host())
        .expect("team cargo-token plan");
    let wrapped = plan
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("team cargo-token wrapper");
    assert!(
        wrapped
            .windows(2)
            .any(|window| window == ["--unsetenv", "NPM_TOKEN"])
    );
    assert!(
        !wrapped
            .windows(2)
            .any(|window| window == ["--unsetenv", "CARGO_REGISTRY_TOKEN"])
    );
    assert!(
        !wrapped
            .windows(2)
            .any(|window| window == ["--unsetenv", "CARGO_REGISTRIES_CRATES_IO_TOKEN"])
    );
}

#[test]
fn exact_unhide_of_grant_path_stays_writable() {
    let fixture = Fixture::new();
    let gh = fixture.home.join(".config/gh");
    std::fs::create_dir_all(&gh).expect("gh directory");
    let gh = std::fs::canonicalize(gh).expect("gh canonical");
    let plan = confine::plan(
        &ConfineRequest {
            grants: vec!["gh".into()],
            unhide: vec!["~/.config/gh".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("exact grant unhide plan");
    assert!(plan.writable.contains(&gh));
    assert!(!plan.readonly.contains(&gh));
}

#[test]
fn nested_unhide_inside_grant_stays_readonly_and_protected() {
    let fixture = Fixture::new();
    let ssh = fixture.home.join(".ssh");
    let config = std::fs::canonicalize(ssh.join("config")).expect("ssh config canonical");
    let plan = confine::plan(
        &ConfineRequest {
            grants: vec!["ssh".into()],
            unhide: vec!["~/.ssh/config".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("nested grant unhide plan");
    assert!(plan.readonly.contains(&config));
    assert!(plan.protected.contains(&config));
    let config = config.to_string_lossy();
    let wrapped = plan
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("nested grant unhide wrapper");
    assert!(
        wrapped.windows(3).any(|window| {
            window[0] == "--ro-bind" && window[1] == config && window[2] == config
        })
    );
}

#[test]
fn gh_grant_releases_its_environment_and_notice_names() {
    let fixture = Fixture::new();
    let plan = confine::plan(
        &ConfineRequest {
            grants: vec!["gh".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("gh environment plan");
    let notice = confine::notice(&plan);
    let wrapped = plan
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("gh environment wrapper");
    for name in ["GH_TOKEN", "GITHUB_TOKEN", "GH_ENTERPRISE_TOKEN"] {
        assert!(
            !wrapped
                .windows(2)
                .any(|window| window == ["--unsetenv", name]),
            "granted environment variable {name} was unset"
        );
    }
    for name in [
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
            "unrelated credential environment variable {name} was not unset"
        );
    }
    let line = notice
        .lines()
        .find(|line| line.starts_with("Credential environment variables are unset:"))
        .expect("credential notice line");
    let names = line
        .strip_prefix("Credential environment variables are unset: ")
        .expect("credential notice prefix")
        .strip_suffix('.')
        .expect("credential notice suffix");
    assert_eq!(names.split(", ").count(), 26);
    assert!(!names.contains("GH_TOKEN"));
    assert!(!names.contains("GITHUB_TOKEN"));
    assert!(!names.contains("GH_ENTERPRISE_TOKEN"));
}

#[test]
fn ssh_grant_unhides_home_ssh_and_stands_in_host_config() {
    let fixture = Fixture::new();
    let ssh_dir = std::fs::canonicalize(fixture.home.join(".ssh")).expect("ssh directory");
    let plain = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("plain plan");
    assert!(plain.hidden_paths().contains(&ssh_dir));

    let config_source = Path::new("/etc/ssh/ssh_config");
    let config = std::fs::canonicalize(config_source).ok();
    assert!(
        !plain.skipped_paths().contains(&config_source.to_owned()),
        "plain plans must not skip the SSH stand-in"
    );
    if let Some(config) = &config {
        assert!(!plain.hidden_paths().contains(config));
    }

    for no_default_hides in [false, true] {
        let plan = confine::plan(
            &ConfineRequest {
                grants: vec!["ssh".into()],
                no_default_hides,
                ..ConfineRequest::default()
            },
            Harness::Generic,
            &fixture.cwd,
            &fixture.host(),
        )
        .expect("ssh grant plan");
        assert!(plan.writable.contains(&ssh_dir));
        assert!(!plan.readonly.contains(&ssh_dir));
        assert!(!plan.protected.contains(&ssh_dir));
        assert!(!plan.hidden_paths().contains(&ssh_dir));
        let config_hidden = config
            .as_ref()
            .is_some_and(|config| plan.hidden_paths().contains(config));
        let config_skipped = plan.skipped_paths().contains(&config_source.to_owned());
        let wrapped = plan
            .with_bwrap(PathBuf::from("bwrap"))
            .wrap(vec!["true".into()])
            .expect("ssh grant wrapper");
        let ssh_dir = ssh_dir.to_string_lossy();
        assert!(wrapped.windows(3).any(|window| {
            window[0] == "--bind" && window[1] == ssh_dir && window[2] == ssh_dir
        }));
        assert!(!wrapped.windows(3).any(|window| {
            window[0] == "--ro-bind" && window[1] == ssh_dir && window[2] == ssh_dir
        }));
        match &config {
            Some(config) => {
                assert!(config_hidden);
                let empty = fixture.empty.to_string_lossy();
                let config = config.to_string_lossy();
                assert!(wrapped.windows(3).any(|window| {
                    window[0] == "--ro-bind" && window[1] == empty && window[2] == config
                }));
            }
            None => assert!(
                config_skipped,
                "missing SSH config must be reported as skipped"
            ),
        }
    }
}

#[test]
fn ssh_grant_operator_visibility_wins_and_key_hides_remain_supported() {
    let fixture = Fixture::new();
    let config_source = Path::new("/etc/ssh/ssh_config");
    let Some(config) = std::fs::canonicalize(config_source).ok() else {
        eprintln!("skipping SSH operator override assertions: host config is absent");
        return;
    };

    let unhidden = confine::plan(
        &ConfineRequest {
            grants: vec!["ssh".into()],
            unhide: vec![config_source.display().to_string()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("SSH unhide override");
    assert!(!unhidden.hidden_paths().contains(&config));

    let writable = confine::plan(
        &ConfineRequest {
            grants: vec!["ssh".into()],
            write: vec![config_source.display().to_string()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("SSH write override");
    assert!(!writable.hidden_paths().contains(&config));
    let wrapped = writable
        .with_bwrap(PathBuf::from("bwrap"))
        .wrap(vec!["true".into()])
        .expect("SSH write wrapper");
    let config = config.to_string_lossy();
    assert!(
        wrapped
            .windows(3)
            .any(|window| { window[0] == "--bind" && window[1] == config && window[2] == config })
    );

    let whole_directory = match confine::plan(
        &ConfineRequest {
            grants: vec!["ssh".into()],
            hide: vec!["~/.ssh".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    ) {
        Ok(_) => panic!("whole SSH directory hide must conflict"),
        Err(error) => error,
    };
    assert!(
        whole_directory
            .to_string()
            .contains("conflicts with the hide")
    );

    let key = fixture.home.join(".ssh/id_work");
    std::fs::write(&key, "private key").expect("SSH key fixture");
    let key = std::fs::canonicalize(key).expect("SSH key path");
    let per_key = confine::plan(
        &ConfineRequest {
            grants: vec!["ssh".into()],
            hide: vec![key.display().to_string()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("per-key SSH hide");
    assert!(per_key.hidden_paths().contains(&key));
}
