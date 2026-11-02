use std::process::Command;

use a2amx::{grant, team};

#[test]
fn grant_table_has_expected_rows() {
    let expected = [
        (
            "docker",
            vec![
                "/run/docker.sock",
                "~/.docker/desktop/docker.sock",
                "~/.docker/run/docker.sock",
            ],
            vec!["DOCKER_HOST", "DOCKER_CONTEXT"],
            vec![],
            vec![],
            false,
            false,
        ),
        (
            "podman",
            vec!["/run/podman/podman.sock"],
            vec!["CONTAINER_HOST", "CONTAINER_CONNECTION"],
            vec![],
            vec![],
            false,
            false,
        ),
        (
            "containerd",
            vec![
                "/run/containerd/containerd.sock",
                "/run/containerd/containerd.sock.ttrpc",
                "/run/buildkit/buildkitd.sock",
                "/run/k3s/containerd/containerd.sock",
            ],
            vec!["CONTAINERD_ADDRESS", "BUILDKIT_HOST"],
            vec![],
            vec![],
            false,
            false,
        ),
        (
            "crio",
            vec!["/run/crio/crio.sock"],
            vec![],
            vec![],
            vec![],
            false,
            false,
        ),
        (
            "libvirt",
            vec!["/run/libvirt/libvirt-sock"],
            vec!["LIBVIRT_DEFAULT_URI"],
            vec![],
            vec![],
            false,
            false,
        ),
        (
            "lxd",
            vec![
                "/var/snap/lxd/common/lxd/unix.socket",
                "/var/lib/lxd/unix.socket",
                "/var/lib/incus/unix.socket",
            ],
            vec![],
            vec![],
            vec![],
            false,
            false,
        ),
        (
            "snap",
            vec!["/run/snapd.socket", "/run/snapd-snap.socket"],
            vec![],
            vec![],
            vec![],
            false,
            false,
        ),
        (
            "harness-config",
            vec![],
            vec![],
            vec![],
            vec![],
            false,
            true,
        ),
        (
            "gh",
            vec![],
            vec!["GH_TOKEN", "GITHUB_TOKEN", "GH_ENTERPRISE_TOKEN"],
            vec!["~/.config/gh"],
            vec![],
            true,
            false,
        ),
        (
            "git-credentials",
            vec![],
            vec![],
            vec!["~/.git-credentials", "~/.config/git/credentials"],
            vec![],
            true,
            false,
        ),
        (
            "ssh",
            vec![],
            vec![],
            vec!["~/.ssh"],
            vec!["/etc/ssh/ssh_config"],
            true,
            false,
        ),
        (
            "cargo",
            vec![],
            vec![],
            vec!["~/.cargo", "~/.rustup"],
            vec![],
            true,
            false,
        ),
        (
            "go",
            vec![],
            vec![],
            vec!["~/go", "~/.cache/go-build"],
            vec![],
            true,
            false,
        ),
        (
            "python",
            vec![],
            vec![],
            vec![
                "~/.cache/pip",
                "~/.cache/uv",
                "~/.local/share/uv",
                "~/.cache/pypoetry",
                "~/.cache/virtualenv",
                "~/.local/share/pipx",
            ],
            vec![],
            true,
            false,
        ),
        (
            "node",
            vec![],
            vec![],
            vec![
                "~/.npm",
                "~/.yarn/berry",
                "~/.local/share/pnpm",
                "~/.bun/install/cache",
            ],
            vec![],
            true,
            false,
        ),
        (
            "java",
            vec![],
            vec![],
            vec!["~/.m2/repository", "~/.gradle"],
            vec![],
            true,
            false,
        ),
        (
            "c-cpp",
            vec![],
            vec![],
            vec!["~/.cache/ccache", "~/.ccache", "~/.conan2", "~/.cmake"],
            vec![],
            true,
            false,
        ),
        (
            "semgrep",
            vec![],
            vec![],
            vec!["~/.semgrep"],
            vec![],
            true,
            false,
        ),
        (
            "sonar",
            vec![],
            vec!["SONAR_TOKEN"],
            vec!["~/.sonar"],
            vec![],
            true,
            false,
        ),
        ("aws", vec![], vec![], vec!["~/.aws"], vec![], true, false),
        (
            "gcloud",
            vec![],
            vec![],
            vec!["~/.config/gcloud"],
            vec![],
            true,
            false,
        ),
        (
            "azure",
            vec![],
            vec!["AZURE_CLIENT_SECRET", "ARM_CLIENT_SECRET"],
            vec!["~/.azure"],
            vec![],
            true,
            false,
        ),
        ("kube", vec![], vec![], vec!["~/.kube"], vec![], true, false),
        (
            "terraform",
            vec![],
            vec![],
            vec!["~/.terraform.d"],
            vec![],
            true,
            false,
        ),
        (
            "npm-token",
            vec![],
            vec!["NPM_TOKEN", "NODE_AUTH_TOKEN"],
            vec!["~/.npmrc"],
            vec![],
            true,
            false,
        ),
        (
            "pypi-token",
            vec![],
            vec!["PYPI_TOKEN", "TWINE_PASSWORD"],
            vec!["~/.pypirc"],
            vec![],
            true,
            false,
        ),
        (
            "cargo-token",
            vec![],
            vec!["CARGO_REGISTRY_TOKEN", "CARGO_REGISTRIES_CRATES_IO_TOKEN"],
            vec!["~/.cargo/credentials", "~/.cargo/credentials.toml"],
            vec![],
            true,
            false,
        ),
        (
            "netrc",
            vec![],
            vec![],
            vec!["~/.netrc"],
            vec![],
            true,
            false,
        ),
    ];
    let all = grant::all();
    assert_eq!(all.len(), expected.len());
    for ((name, sockets, env, unhides, stand_ins, team_default_ok, own_config), row) in
        expected.iter().zip(all)
    {
        assert_eq!(row.name, *name);
        assert!(!row.exposes.is_empty());
        assert_eq!(row.team_default_ok, *team_default_ok);
        assert_eq!(row.sockets, sockets.as_slice());
        assert_eq!(row.env, env.as_slice());
        assert_eq!(row.unhides, unhides.as_slice());
        assert_eq!(row.stand_ins, stand_ins.as_slice());
        assert_eq!(row.own_config, *own_config);
    }
}

#[test]
fn grant_rows_have_exact_exposure_text() {
    let expected = [
        (
            "gh",
            "your GitHub CLI login (~/.config/gh, read-write) and its tokens in the environment: the agent can push, open and merge pull requests, read private repositories as you, and change or delete the login",
        ),
        (
            "git-credentials",
            "the plain-text git credential files (read-write in place): every host and password stored there, which the agent can also overwrite",
        ),
        (
            "ssh",
            "your ssh keys and config (~/.ssh, read-write): the agent can log in to every host those keys open as you, push with them, and change or delete them",
        ),
        (
            "cargo",
            "the Rust toolchain, read-write: ~/.cargo (registry and git caches, installed crates; its bin directory stays read-only when it is on PATH) and ~/.rustup (installed toolchains), whose programs and settings (config.toml, env) your own builds outside the session run; your registry login stays hidden while it exists unless you also give cargo-token",
        ),
        (
            "go",
            "the Go toolchain, read-write: ~/go (module cache, downloaded toolchains and installed binaries, which your own builds outside the session may run) and ~/.cache/go-build; ~/.netrc stays hidden unless you also give netrc",
        ),
        (
            "python",
            "the Python tool caches, read-write: pip, uv, poetry, virtualenv and pipx data under ~/.cache and ~/.local/share, including tool environments whose programs your own shell may run; ~/.pypirc stays hidden unless you also give pypi-token",
        ),
        (
            "node",
            "the JavaScript package caches, read-write: ~/.npm, ~/.yarn/berry, ~/.local/share/pnpm and ~/.bun/install/cache; ~/.npmrc stays hidden unless you also give npm-token",
        ),
        (
            "java",
            "the Java build caches, read-write: ~/.m2/repository and the whole ~/.gradle, where a gradle.properties you wrote may hold repository passwords the agent can then read and an init.d script is run by every gradle build you start outside the session",
        ),
        (
            "c-cpp",
            "the C and C++ build caches, read-write: ccache, conan and cmake data in ~/.cache/ccache, ~/.ccache, ~/.conan2 and ~/.cmake, which your own builds outside the session use",
        ),
        (
            "semgrep",
            "semgrep's data directory ~/.semgrep, read-write: its settings file holds the login token if you ran semgrep login",
        ),
        (
            "sonar",
            "sonar-scanner's data directory ~/.sonar, read-write, and the SONAR_TOKEN environment variable: the agent can run analyses and publish results to your SonarQube or SonarCloud project as you",
        ),
        (
            "aws",
            "your AWS CLI configuration and credentials (~/.aws, read-write, including cached SSO tokens): the agent acts in your AWS accounts as you and can change or delete the files",
        ),
        (
            "gcloud",
            "your Google Cloud CLI login (~/.config/gcloud, read-write): the agent acts in your Google Cloud projects as you and can change or delete the login",
        ),
        (
            "azure",
            "your Azure CLI login (~/.azure, read-write) and the AZURE_CLIENT_SECRET and ARM_CLIENT_SECRET environment variables: the agent acts in your Azure subscriptions as you and can change or delete the login",
        ),
        (
            "kube",
            "your kubectl configuration (~/.kube, read-write): the cluster credentials in it let the agent act in your clusters as you, and it can change or delete them",
        ),
        (
            "terraform",
            "terraform's user directory (~/.terraform.d, read-write): the saved Terraform Cloud or registry tokens in it let the agent act as you, and it can change or delete them",
        ),
        (
            "npm-token",
            "your ~/.npmrc (read-write in place) and the NPM_TOKEN and NODE_AUTH_TOKEN environment variables: the agent can publish packages as you and change the file",
        ),
        (
            "pypi-token",
            "your ~/.pypirc (read-write in place) and the PYPI_TOKEN and TWINE_PASSWORD environment variables: the agent can upload packages as you and change the file",
        ),
        (
            "cargo-token",
            "your cargo registry login (~/.cargo/credentials and ~/.cargo/credentials.toml, read-write in place) and the CARGO_REGISTRY_TOKEN and CARGO_REGISTRIES_CRATES_IO_TOKEN environment variables: the agent can publish crates as you and change the file",
        ),
        (
            "netrc",
            "your ~/.netrc (read-write in place): every host login and password stored there, which the agent can also overwrite",
        ),
    ];
    for (name, exposes) in expected {
        assert_eq!(grant::lookup(name).expect("grant row").exposes, exposes);
    }
}

#[test]
fn new_grant_environment_names_are_in_the_closed_credential_list() {
    let credential_env = [
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
    for row in grant::all().iter().skip(11) {
        assert!(
            row.env.iter().all(|name| credential_env.contains(name)),
            "{} has an environment name outside CREDENTIAL_ENV",
            row.name
        );
    }
}

#[test]
fn grant_validation_rejects_repeats_and_suggests_close_names() {
    let repeated =
        grant::validate(&["docker".into(), "docker".into()]).expect_err("repeated grant");
    assert!(repeated.to_string().contains("repeated"));

    for (name, closest) in [
        ("container", "containerd"),
        ("containrs", "containerd"),
        ("contaner", "containerd"),
        ("lx", "lxd"),
        ("gitcredentials", "git-credentials"),
        ("ghh", "gh"),
    ] {
        let error = grant::lookup(name).expect_err("close typo");
        assert!(
            error
                .to_string()
                .contains(&format!("closest match: {closest}"))
        );
    }
    for name in ["gpg", "git", "g", "gx", "gj"] {
        let error = grant::lookup(name).expect_err("ambiguous close typo");
        assert!(
            !error.to_string().contains("closest match"),
            "{name} unexpectedly received a suggestion: {error}"
        );
    }
    let distant = grant::lookup("zzz").expect_err("unknown grant");
    assert!(distant.to_string().contains("docker"));
    assert!(!distant.to_string().contains("closest"));
}

#[test]
fn old_containers_name_reports_the_split_grants() {
    let error = grant::lookup("containers").expect_err("old grant name");
    assert_eq!(
        error.to_string(),
        "confinement grant \"containers\" was split into one grant per runtime: docker, podman, containerd, crio, libvirt, lxd, snap; run a2amx confine list"
    );
}
#[test]
fn team_table_accepts_new_grants_and_rejects_the_rest() {
    for grant in [
        "gh",
        "ssh",
        "git-credentials",
        "cargo",
        "go",
        "python",
        "node",
        "java",
        "c-cpp",
        "semgrep",
        "sonar",
        "aws",
        "gcloud",
        "azure",
        "kube",
        "terraform",
        "npm-token",
        "pypi-token",
        "cargo-token",
        "netrc",
    ] {
        team::parse(&format!(
            "[confine]\ngrants = ['{grant}']\n[[session]]\nname = 'worker'\ncommand = ['cat']"
        ))
        .expect("team-default grant is allowed team-wide");
    }
    for grant in ["docker", "harness-config"] {
        let error = team::parse(&format!(
            "[confine]\ngrants = ['{grant}']\n[[session]]\nname = 'worker'\ncommand = ['cat']"
        ))
        .expect_err("non-team grant is session-only");
        assert!(error.to_string().contains("grant it in the session"));
    }
}

#[test]
fn render_list_has_every_grant_and_instruction() {
    let rendered = grant::render_list();
    let lines = rendered.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), grant::all().len() + 1);
    for (line, row) in lines.iter().zip(grant::all()) {
        let after_name = line
            .strip_prefix(row.name)
            .expect("each rendered row starts with its grant name");
        assert_eq!(
            after_name.chars().next(),
            Some(' '),
            "{line:?} has no separating space after {}",
            row.name
        );
        let scope = if row.team_default_ok {
            "team or session"
        } else {
            "session only"
        };
        let scope_offset = after_name
            .find(scope)
            .expect("each rendered row reports its grant scope");
        assert!(
            scope_offset > 0
                && after_name[..scope_offset]
                    .chars()
                    .all(|character| character.is_whitespace()),
            "{line:?} does not separate the name and scope with whitespace"
        );
        assert!(line.contains(row.exposes));
    }
    assert_eq!(
        lines[8],
        "gh              team or sessionyour GitHub CLI login (~/.config/gh, read-write) and its tokens in the environment: the agent can push, open and merge pull requests, read private repositories as you, and change or delete the login"
    );
    assert_eq!(
        lines[9],
        "git-credentials team or sessionthe plain-text git credential files (read-write in place): every host and password stored there, which the agent can also overwrite"
    );
    assert_eq!(
        lines[10],
        "ssh             team or sessionyour ssh keys and config (~/.ssh, read-write): the agent can log in to every host those keys open as you, push with them, and change or delete them"
    );
    assert_eq!(
        lines.last().copied(),
        Some(
            "Give a grant with --confine-grant NAME, or grants = [\"NAME\"] in [session.confine]."
        )
    );
}

#[test]
fn binary_confine_list_needs_no_home_or_daemon() {
    let output = Command::new(env!("CARGO_BIN_EXE_a2amx"))
        .env_clear()
        .args(["confine", "list"])
        .output()
        .expect("run confine list");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, grant::render_list().as_bytes());
    assert!(output.stderr.is_empty(), "{:?}", output.stderr);
}
