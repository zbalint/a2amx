//! Named opt-in capabilities for confinement.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};

use crate::wire::ConfineInfo;

/// One named thing a user opens on purpose.
#[derive(Debug, PartialEq, Eq)]
pub struct Grant {
    pub name: &'static str,
    /// One line for the docs and for the later `confine list`.
    pub exposes: &'static str,
    /// Additional warning when a grant can escape the confinement boundary.
    pub escape: Option<&'static str>,
    /// Whether a team-level table may name this grant.
    pub team_default_ok: bool,
    /// Host paths hidden unless the grant is given (`~/` is the home directory).
    pub sockets: &'static [&'static str],
    /// Environment names unset unless the grant is given.
    pub env: &'static [&'static str],
    /// Whether this grant opens the current harness's own configuration directory.
    pub own_config: bool,
    /// Home-relative or absolute paths made visible and writable when the grant is given.
    pub unhides: &'static [&'static str],
    /// Names in the session's working directory whose presence shows this grant is needed.
    pub detect: &'static [&'static str],
    /// Tool directories created when missing; always a subset of `unhides`.
    pub create: &'static [&'static str],
    /// Absolute host files shown as empty when the grant is given.
    pub stand_ins: &'static [&'static str],
}

const NO_PATHS: &[&str] = &[];
const DOCKER_SOCKETS: &[&str] = &[
    "/run/docker.sock",
    "~/.docker/desktop/docker.sock",
    "~/.docker/run/docker.sock",
];
const DOCKER_ENV: &[&str] = &["DOCKER_HOST", "DOCKER_CONTEXT"];
const PODMAN_SOCKETS: &[&str] = &["/run/podman/podman.sock"];
const PODMAN_ENV: &[&str] = &["CONTAINER_HOST", "CONTAINER_CONNECTION"];
const CONTAINERD_SOCKETS: &[&str] = &[
    "/run/containerd/containerd.sock",
    "/run/containerd/containerd.sock.ttrpc",
    "/run/buildkit/buildkitd.sock",
    "/run/k3s/containerd/containerd.sock",
];
const CONTAINERD_ENV: &[&str] = &["CONTAINERD_ADDRESS", "BUILDKIT_HOST"];
const CRIO_SOCKETS: &[&str] = &["/run/crio/crio.sock"];
const LIBVIRT_SOCKETS: &[&str] = &["/run/libvirt/libvirt-sock"];
const LIBVIRT_ENV: &[&str] = &["LIBVIRT_DEFAULT_URI"];
const LXD_SOCKETS: &[&str] = &[
    "/var/snap/lxd/common/lxd/unix.socket",
    "/var/lib/lxd/unix.socket",
    "/var/lib/incus/unix.socket",
];
const SNAP_SOCKETS: &[&str] = &["/run/snapd.socket", "/run/snapd-snap.socket"];
const GH_UNHIDES: &[&str] = &["~/.config/gh"];
const GH_ENV: &[&str] = &["GH_TOKEN", "GITHUB_TOKEN", "GH_ENTERPRISE_TOKEN"];
const GIT_CREDENTIAL_UNHIDES: &[&str] = &["~/.git-credentials", "~/.config/git/credentials"];
const SSH_UNHIDES: &[&str] = &["~/.ssh"];
const SSH_STAND_INS: &[&str] = &["/etc/ssh/ssh_config"];
const CARGO_UNHIDES: &[&str] = &["~/.cargo", "~/.rustup"];
const GO_UNHIDES: &[&str] = &["~/go", "~/.cache/go-build"];
const PYTHON_UNHIDES: &[&str] = &[
    "~/.cache/pip",
    "~/.cache/uv",
    "~/.local/share/uv",
    "~/.cache/pypoetry",
    "~/.cache/virtualenv",
    "~/.local/share/pipx",
];
const NODE_UNHIDES: &[&str] = &[
    "~/.npm",
    "~/.yarn/berry",
    "~/.local/share/pnpm",
    "~/.bun/install/cache",
];
const JAVA_UNHIDES: &[&str] = &["~/.m2/repository", "~/.gradle"];
const C_CPP_UNHIDES: &[&str] = &["~/.cache/ccache", "~/.ccache", "~/.conan2", "~/.cmake"];
const SEMGREP_UNHIDES: &[&str] = &["~/.semgrep"];
const SONAR_UNHIDES: &[&str] = &["~/.sonar"];
const AWS_UNHIDES: &[&str] = &["~/.aws"];
const GCLOUD_UNHIDES: &[&str] = &["~/.config/gcloud"];
const AZURE_UNHIDES: &[&str] = &["~/.azure"];
const KUBE_UNHIDES: &[&str] = &["~/.kube"];
const TERRAFORM_UNHIDES: &[&str] = &["~/.terraform.d"];
const NPM_TOKEN_UNHIDES: &[&str] = &["~/.npmrc"];
const PYPI_TOKEN_UNHIDES: &[&str] = &["~/.pypirc"];
const CARGO_TOKEN_UNHIDES: &[&str] = &["~/.cargo/credentials", "~/.cargo/credentials.toml"];
const NETRC_UNHIDES: &[&str] = &["~/.netrc"];
const CACHE_UNHIDES: &[&str] = &["~/.cache"];
const HUGGINGFACE_TOKEN_UNHIDES: &[&str] = &["~/.cache/huggingface/token"];

const ROOT_DAEMON_ESCAPE: &str = "a privileged container started through the daemon mounts the host filesystem and runs as root on the host";

static GRANTS: [Grant; 30] = [
    Grant {
        name: "docker",
        exposes: "docker daemon sockets: any docker command as you; an accidental rm -rf ~/ still fails, but on a rootful daemon a deliberate privileged container is root on the host, and userns-remap or a sandboxed runtime does not stop that",
        escape: Some(ROOT_DAEMON_ESCAPE),
        team_default_ok: true,
        sockets: DOCKER_SOCKETS,
        env: DOCKER_ENV,
        own_config: false,
        unhides: NO_PATHS,
        stand_ins: NO_PATHS,
        detect: &[
            "Dockerfile",
            "compose.yaml",
            "compose.yml",
            "docker-compose.yaml",
            "docker-compose.yml",
            ".devcontainer",
        ],
        create: NO_PATHS,
    },
    Grant {
        name: "podman",
        exposes: "podman daemon socket: any podman command through it; the system socket belongs to a root service on a default install, so a deliberate privileged container is root on the host",
        escape: Some(ROOT_DAEMON_ESCAPE),
        team_default_ok: false,
        sockets: PODMAN_SOCKETS,
        env: PODMAN_ENV,
        own_config: false,
        unhides: NO_PATHS,
        stand_ins: NO_PATHS,
        detect: NO_PATHS,
        create: NO_PATHS,
    },
    Grant {
        name: "containerd",
        exposes: "containerd, buildkit and k3s daemon sockets: any command through them; these daemons run as root on a default install, so a deliberate privileged container is root on the host",
        escape: Some(ROOT_DAEMON_ESCAPE),
        team_default_ok: false,
        sockets: CONTAINERD_SOCKETS,
        env: CONTAINERD_ENV,
        own_config: false,
        unhides: NO_PATHS,
        stand_ins: NO_PATHS,
        detect: NO_PATHS,
        create: NO_PATHS,
    },
    Grant {
        name: "crio",
        exposes: "crio daemon socket: any command through it; the daemon runs as root on a default install, so a deliberate privileged container is root on the host",
        escape: Some(ROOT_DAEMON_ESCAPE),
        team_default_ok: false,
        sockets: CRIO_SOCKETS,
        env: NO_PATHS,
        own_config: false,
        unhides: NO_PATHS,
        stand_ins: NO_PATHS,
        detect: NO_PATHS,
        create: NO_PATHS,
    },
    Grant {
        name: "libvirt",
        exposes: "libvirt virtual-machine control: controls virtual machines",
        escape: None,
        team_default_ok: false,
        sockets: LIBVIRT_SOCKETS,
        env: LIBVIRT_ENV,
        own_config: false,
        unhides: NO_PATHS,
        stand_ins: NO_PATHS,
        detect: NO_PATHS,
        create: NO_PATHS,
    },
    Grant {
        name: "lxd",
        exposes: "lxd and incus daemon sockets: any command through them; these daemons run as root on a default install, so a deliberate privileged container is root on the host",
        escape: Some(ROOT_DAEMON_ESCAPE),
        team_default_ok: false,
        sockets: LXD_SOCKETS,
        env: NO_PATHS,
        own_config: false,
        unhides: NO_PATHS,
        stand_ins: NO_PATHS,
        detect: NO_PATHS,
        create: NO_PATHS,
    },
    Grant {
        name: "snap",
        exposes: "snapd package management: installs and removes packages",
        escape: None,
        team_default_ok: false,
        sockets: SNAP_SOCKETS,
        env: NO_PATHS,
        own_config: false,
        unhides: NO_PATHS,
        stand_ins: NO_PATHS,
        detect: NO_PATHS,
        create: NO_PATHS,
    },
    Grant {
        name: "harness-config",
        exposes: "the session's own harness config directory, read-write and persistent: settings, hooks, instructions and programs under it; running sessions of that harness pick up changes immediately",
        escape: None,
        team_default_ok: false,
        sockets: NO_PATHS,
        env: NO_PATHS,
        own_config: true,
        unhides: NO_PATHS,
        stand_ins: NO_PATHS,
        detect: NO_PATHS,
        create: NO_PATHS,
    },
    Grant {
        name: "gh",
        exposes: "your GitHub CLI login (~/.config/gh, read-write) and its tokens in the environment: the agent can push, open and merge pull requests, read private repositories as you, and change or delete the login",
        escape: None,
        team_default_ok: true,
        sockets: NO_PATHS,
        env: GH_ENV,
        own_config: false,
        unhides: GH_UNHIDES,
        stand_ins: NO_PATHS,
        detect: NO_PATHS,
        create: NO_PATHS,
    },
    Grant {
        name: "git-credentials",
        exposes: "the plain-text git credential files (read-write in place): every host and password stored there, which the agent can also overwrite",
        escape: None,
        team_default_ok: true,
        sockets: NO_PATHS,
        env: NO_PATHS,
        own_config: false,
        unhides: GIT_CREDENTIAL_UNHIDES,
        stand_ins: NO_PATHS,
        detect: NO_PATHS,
        create: NO_PATHS,
    },
    Grant {
        name: "ssh",
        exposes: "your ssh keys and config (~/.ssh, read-write): the agent can log in to every host those keys open as you, push with them, and change or delete them",
        escape: None,
        team_default_ok: true,
        sockets: NO_PATHS,
        env: NO_PATHS,
        own_config: false,
        unhides: SSH_UNHIDES,
        stand_ins: SSH_STAND_INS,
        detect: NO_PATHS,
        create: NO_PATHS,
    },
    Grant {
        name: "cargo",
        exposes: "the Rust toolchain, read-write: ~/.cargo (registry and git caches, installed crates; its bin directory stays read-only when it is on PATH) and ~/.rustup (installed toolchains), whose programs and settings (config.toml, env) your own builds outside the session run; your registry login stays hidden while it exists unless you also give cargo-token",
        escape: None,
        team_default_ok: true,
        sockets: NO_PATHS,
        env: NO_PATHS,
        own_config: false,
        unhides: CARGO_UNHIDES,
        stand_ins: NO_PATHS,
        detect: &["Cargo.toml"],
        create: &["~/.cargo"],
    },
    Grant {
        name: "go",
        exposes: "the Go toolchain, read-write: ~/go (module cache, downloaded toolchains and installed binaries, which your own builds outside the session may run) and ~/.cache/go-build; ~/.netrc stays hidden unless you also give netrc",
        escape: None,
        team_default_ok: true,
        sockets: NO_PATHS,
        env: NO_PATHS,
        own_config: false,
        unhides: GO_UNHIDES,
        stand_ins: NO_PATHS,
        detect: &["go.mod"],
        create: &["~/go", "~/.cache/go-build"],
    },
    Grant {
        name: "python",
        exposes: "the Python tool caches, read-write: pip, uv, poetry, virtualenv and pipx data under ~/.cache and ~/.local/share, including tool environments whose programs your own shell may run; ~/.pypirc stays hidden unless you also give pypi-token",
        escape: None,
        team_default_ok: true,
        sockets: NO_PATHS,
        env: NO_PATHS,
        own_config: false,
        unhides: PYTHON_UNHIDES,
        stand_ins: NO_PATHS,
        detect: &["pyproject.toml", "requirements.txt"],
        create: &["~/.cache/pip", "~/.cache/uv"],
    },
    Grant {
        name: "node",
        exposes: "the JavaScript package caches, read-write: ~/.npm, ~/.yarn/berry, ~/.local/share/pnpm and ~/.bun/install/cache; ~/.npmrc stays hidden unless you also give npm-token",
        escape: None,
        team_default_ok: true,
        sockets: NO_PATHS,
        env: NO_PATHS,
        own_config: false,
        unhides: NODE_UNHIDES,
        stand_ins: NO_PATHS,
        detect: &["package.json"],
        create: &["~/.npm"],
    },
    Grant {
        name: "java",
        exposes: "the Java build caches, read-write: ~/.m2/repository and the whole ~/.gradle, where a gradle.properties you wrote may hold repository passwords the agent can then read and an init.d script is run by every gradle build you start outside the session",
        escape: None,
        team_default_ok: true,
        sockets: NO_PATHS,
        env: NO_PATHS,
        own_config: false,
        unhides: JAVA_UNHIDES,
        stand_ins: NO_PATHS,
        detect: &[
            "pom.xml",
            "build.gradle",
            "build.gradle.kts",
            "settings.gradle",
            "settings.gradle.kts",
        ],
        create: &["~/.m2/repository", "~/.gradle"],
    },
    Grant {
        name: "c-cpp",
        exposes: "the C and C++ build caches, read-write: ccache, conan and cmake data in ~/.cache/ccache, ~/.ccache, ~/.conan2 and ~/.cmake, which your own builds outside the session use",
        escape: None,
        team_default_ok: true,
        sockets: NO_PATHS,
        env: NO_PATHS,
        own_config: false,
        unhides: C_CPP_UNHIDES,
        stand_ins: NO_PATHS,
        detect: &["CMakeLists.txt", "conanfile.txt", "conanfile.py"],
        create: &["~/.cache/ccache"],
    },
    Grant {
        name: "semgrep",
        exposes: "semgrep's data directory ~/.semgrep, read-write: its settings file holds the login token if you ran semgrep login",
        escape: None,
        team_default_ok: true,
        sockets: NO_PATHS,
        env: NO_PATHS,
        own_config: false,
        unhides: SEMGREP_UNHIDES,
        stand_ins: NO_PATHS,
        detect: &[".semgrep.yml", ".semgrep.yaml", ".semgrep"],
        create: &["~/.semgrep"],
    },
    Grant {
        name: "sonar",
        exposes: "sonar-scanner's data directory ~/.sonar, read-write, and the SONAR_TOKEN environment variable: the agent can run analyses and publish results to your SonarQube or SonarCloud project as you",
        escape: None,
        team_default_ok: true,
        sockets: NO_PATHS,
        env: &["SONAR_TOKEN"],
        own_config: false,
        unhides: SONAR_UNHIDES,
        stand_ins: NO_PATHS,
        detect: &["sonar-project.properties"],
        create: &["~/.sonar"],
    },
    Grant {
        name: "aws",
        exposes: "your AWS CLI configuration and credentials (~/.aws, read-write, including cached SSO tokens): the agent acts in your AWS accounts as you and can change or delete the files",
        escape: None,
        team_default_ok: true,
        sockets: NO_PATHS,
        env: NO_PATHS,
        own_config: false,
        unhides: AWS_UNHIDES,
        stand_ins: NO_PATHS,
        detect: NO_PATHS,
        create: NO_PATHS,
    },
    Grant {
        name: "gcloud",
        exposes: "your Google Cloud CLI login (~/.config/gcloud, read-write): the agent acts in your Google Cloud projects as you and can change or delete the login",
        escape: None,
        team_default_ok: true,
        sockets: NO_PATHS,
        env: NO_PATHS,
        own_config: false,
        unhides: GCLOUD_UNHIDES,
        stand_ins: NO_PATHS,
        detect: NO_PATHS,
        create: NO_PATHS,
    },
    Grant {
        name: "azure",
        exposes: "your Azure CLI login (~/.azure, read-write) and the AZURE_CLIENT_SECRET and ARM_CLIENT_SECRET environment variables: the agent acts in your Azure subscriptions as you and can change or delete the login",
        escape: None,
        team_default_ok: true,
        sockets: NO_PATHS,
        env: &["AZURE_CLIENT_SECRET", "ARM_CLIENT_SECRET"],
        own_config: false,
        unhides: AZURE_UNHIDES,
        stand_ins: NO_PATHS,
        detect: NO_PATHS,
        create: NO_PATHS,
    },
    Grant {
        name: "kube",
        exposes: "your kubectl configuration (~/.kube, read-write): the cluster credentials in it let the agent act in your clusters as you, and it can change or delete them",
        escape: None,
        team_default_ok: true,
        sockets: NO_PATHS,
        env: NO_PATHS,
        own_config: false,
        unhides: KUBE_UNHIDES,
        stand_ins: NO_PATHS,
        detect: NO_PATHS,
        create: NO_PATHS,
    },
    Grant {
        name: "terraform",
        exposes: "terraform's user directory (~/.terraform.d, read-write): the saved Terraform Cloud or registry tokens in it let the agent act as you, and it can change or delete them",
        escape: None,
        team_default_ok: true,
        sockets: NO_PATHS,
        env: NO_PATHS,
        own_config: false,
        unhides: TERRAFORM_UNHIDES,
        stand_ins: NO_PATHS,
        detect: NO_PATHS,
        create: NO_PATHS,
    },
    Grant {
        name: "npm-token",
        exposes: "your ~/.npmrc (read-write in place) and the NPM_TOKEN and NODE_AUTH_TOKEN environment variables: the agent can publish packages as you and change the file",
        escape: None,
        team_default_ok: true,
        sockets: NO_PATHS,
        env: &["NPM_TOKEN", "NODE_AUTH_TOKEN"],
        own_config: false,
        unhides: NPM_TOKEN_UNHIDES,
        stand_ins: NO_PATHS,
        detect: NO_PATHS,
        create: NO_PATHS,
    },
    Grant {
        name: "pypi-token",
        exposes: "your ~/.pypirc (read-write in place) and the PYPI_TOKEN and TWINE_PASSWORD environment variables: the agent can upload packages as you and change the file",
        escape: None,
        team_default_ok: true,
        sockets: NO_PATHS,
        env: &["PYPI_TOKEN", "TWINE_PASSWORD"],
        own_config: false,
        unhides: PYPI_TOKEN_UNHIDES,
        stand_ins: NO_PATHS,
        detect: NO_PATHS,
        create: NO_PATHS,
    },
    Grant {
        name: "cargo-token",
        exposes: "your cargo registry login (~/.cargo/credentials and ~/.cargo/credentials.toml, read-write in place) and the CARGO_REGISTRY_TOKEN and CARGO_REGISTRIES_CRATES_IO_TOKEN environment variables: the agent can publish crates as you and change the file",
        escape: None,
        team_default_ok: true,
        sockets: NO_PATHS,
        env: &["CARGO_REGISTRY_TOKEN", "CARGO_REGISTRIES_CRATES_IO_TOKEN"],
        own_config: false,
        unhides: CARGO_TOKEN_UNHIDES,
        stand_ins: NO_PATHS,
        detect: NO_PATHS,
        create: NO_PATHS,
    },
    Grant {
        name: "netrc",
        exposes: "your ~/.netrc (read-write in place): every host login and password stored there, which the agent can also overwrite",
        escape: None,
        team_default_ok: true,
        sockets: NO_PATHS,
        env: NO_PATHS,
        own_config: false,
        unhides: NETRC_UNHIDES,
        stand_ins: NO_PATHS,
        detect: NO_PATHS,
        create: NO_PATHS,
    },
    Grant {
        name: "cache",
        exposes: "your whole ~/.cache, read-write: every tool's cache and scratch data, including tool environments and hooks whose programs your own shell and builds outside the session run; the default hides under it (for example the Hugging Face token) stay hidden only while they exist when the session starts, and only until something moves them; a credential file created or renamed later is open to the agent, and a state directory under ~/.cache makes the grant fail",
        escape: None,
        team_default_ok: true,
        sockets: NO_PATHS,
        env: NO_PATHS,
        own_config: false,
        unhides: CACHE_UNHIDES,
        stand_ins: NO_PATHS,
        detect: NO_PATHS,
        create: &["~/.cache"],
    },
    Grant {
        name: "huggingface-token",
        exposes: "your Hugging Face login (~/.cache/huggingface/token, read-write in place) and the HF_TOKEN and HUGGING_FACE_HUB_TOKEN environment variables: the agent can act on your Hugging Face account and change the file",
        escape: None,
        team_default_ok: true,
        sockets: NO_PATHS,
        env: &["HF_TOKEN", "HUGGING_FACE_HUB_TOKEN"],
        own_config: false,
        unhides: HUGGINGFACE_TOKEN_UNHIDES,
        stand_ins: NO_PATHS,
        detect: NO_PATHS,
        create: NO_PATHS,
    },
];

/// Return every grant in stable display order.
pub fn all() -> &'static [Grant] {
    &GRANTS
}

/// Explain the paths a confined session cannot reach and the options that open them.
pub fn denial_help(
    info: &ConfineInfo,
    cwd: Option<&str>,
    default_hides: &[String],
    rows: &[Grant],
    exists: &dyn Fn(&str) -> bool,
) -> String {
    let matches_unhide = |path: &str| info.unhide.iter().any(|entry| entry.trim() == path.trim());
    let present_defaults = if info.no_default_hides {
        Vec::new()
    } else {
        default_hides
            .iter()
            .filter(|path| exists(path) && !matches_unhide(path))
            .cloned()
            .collect::<Vec<_>>()
    };

    let grant_names = info
        .grants
        .iter()
        .filter(|name| name.as_str() != "harness-config")
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    let grant_names = if grant_names.is_empty() {
        "none"
    } else {
        grant_names.as_str()
    };
    let harness_config = info.grants.iter().any(|name| name == "harness-config");
    let mut output = String::new();
    output.push_str(&format!(
        "confinement: {}; grants: {grant_names}",
        if info.auto { "auto" } else { "manual" }
    ));
    if harness_config {
        output.push_str("; harness config");
    }
    output.push('\n');

    let mut writable = vec![format!(
        "working directory {}, its git directory",
        cwd.unwrap_or("unknown")
    )];
    if harness_config {
        writable.push("harness config".to_owned());
    }
    writable.extend(info.write.iter().cloned());
    output.push_str(&format!("writable: {}\n", writable.join(", ")));
    output.push_str("read-only: the rest of the home directory and the system\n");
    output.push_str(
        "protected (no option opens them): git hooks and config, the hooksPath target, tool directories on PATH inside writable paths, the a2amx state and runtime directories, /run/WSL, /mnt/wslg\n",
    );

    let mut hidden = present_defaults.clone();
    hidden.extend(info.hide.iter().cloned());
    if hidden.is_empty() {
        output.push_str("hidden: nothing from the default set is present\n");
    } else {
        output.push_str(&format!(
            "hidden (present on this host): {}\n",
            hidden.join(", ")
        ));
    }

    output.push_str("to open a denied path:\n");
    for row in rows {
        if info.grants.iter().any(|name| name == row.name) {
            continue;
        }
        let mut paths = Vec::new();
        for path in row.unhides.iter().chain(row.sockets.iter()) {
            if exists(path) && !paths.contains(path) {
                paths.push(*path);
            }
        }
        if paths.is_empty() {
            continue;
        }
        let scope = if row.team_default_ok {
            "team or session"
        } else {
            "session only"
        };
        output.push_str(&format!(
            "  {}  grant {} ({scope})\n",
            paths.join(", "),
            row.name
        ));
    }

    if !info.no_default_hides {
        let uncovered = present_defaults
            .iter()
            .filter(|default| {
                let default = default.trim_end_matches('/');
                !rows.iter().any(|row| {
                    row.unhides
                        .iter()
                        .chain(row.sockets.iter())
                        .any(|path| path.trim_end_matches('/') == default)
                })
            })
            .map(String::as_str)
            .collect::<Vec<_>>();
        if !uncovered.is_empty() {
            output.push_str(&format!(
                "  {}  --confine-unhide PATH (no grant opens it)\n",
                uncovered.join(", ")
            ));
        }
    }

    output.push_str(
        "any other home path  write = [\"PATH\"] in [session.confine], or --confine-write PATH\n",
    );
    output.push_str(
        "a grant goes in [session.confine] grants = [\"NAME\"], or --confine-grant NAME\n",
    );
    if !info.unhide.is_empty() {
        output.push_str(&format!(
            "this session also unhides: {}\n",
            info.unhide.join(", ")
        ));
    }
    if !info.hide.is_empty() {
        output.push_str(&format!(
            "this session also hides: {}\n",
            info.hide.join(", ")
        ));
    }
    output
}

pub type Accepted = BTreeMap<String, BTreeSet<String>>;

/// Parse accepted auto-grants.toml entries by canonical project path.
pub fn parse_accepted(text: &str) -> Result<Accepted> {
    if text.trim().is_empty() {
        return Ok(BTreeMap::new());
    }
    let root: toml::Table =
        toml::from_str(text).context("invalid accepted auto-grants.toml TOML")?;
    let Some(projects) = root.get("projects") else {
        bail!("accepted auto-grants.toml must contain only a projects table");
    };
    let Some(projects) = projects.as_table() else {
        bail!("accepted auto-grants.toml must contain only a projects table");
    };
    if root.len() != 1 {
        bail!("accepted auto-grants.toml must contain only a projects table");
    }
    let mut accepted = BTreeMap::new();
    for (project, value) in projects {
        if !project.starts_with('/') {
            bail!("accepted auto-grants.toml project key is not absolute: {project}");
        }
        let table = value.as_table().ok_or_else(|| {
            anyhow::anyhow!("accepted auto-grants.toml project {project} is not a table")
        })?;
        if table.len() != 1 || !table.contains_key("grants") {
            bail!("accepted auto-grants.toml project {project} must contain only grants");
        }
        let grants = table["grants"].as_array().ok_or_else(|| {
            anyhow::anyhow!("accepted auto-grants.toml project {project} grants is not a list")
        })?;
        let mut names = BTreeSet::new();
        for grant in grants {
            let name = grant.as_str().ok_or_else(|| {
                anyhow::anyhow!(
                    "accepted auto-grants.toml project {project} has a non-string grant"
                )
            })?;
            if all().iter().any(|row| row.name == name) {
                names.insert(name.to_owned());
            }
        }
        if !names.is_empty() {
            accepted.insert(project.clone(), names);
        }
    }
    Ok(accepted)
}

/// Render accepted auto-grants.toml entries by canonical project path.
pub fn render_accepted(accepted: &Accepted) -> String {
    let mut projects = toml::Table::new();
    for (project, grants) in accepted {
        if grants.is_empty() {
            continue;
        }
        let grants = grants
            .iter()
            .cloned()
            .map(toml::Value::String)
            .collect::<Vec<_>>();
        let mut table = toml::Table::new();
        table.insert("grants".to_owned(), toml::Value::Array(grants));
        projects.insert(project.clone(), toml::Value::Table(table));
    }
    if projects.is_empty() {
        return String::new();
    }
    let mut root = toml::Table::new();
    root.insert("projects".to_owned(), toml::Value::Table(projects));
    match toml::to_string(&toml::Value::Table(root)) {
        Ok(rendered) => rendered,
        Err(error) => unreachable!("generated accepted auto-grants.toml are serializable: {error}"),
    }
}

/// Render the project-specific consent question.
pub fn consent_question(project: &std::path::Path) -> String {
    format!(
        "Accept these for auto mode in this project ({})? [y/N] ",
        project.display()
    )
}

/// Render the refusal shown when auto grants are not accepted.
pub fn consent_refusal(new: &[(String, String)]) -> String {
    let names = new
        .iter()
        .map(|(grant, _)| grant.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "auto confinement would open: {names}; run a2amx team up on a terminal to accept them, or pass --yes for this run"
    )
}

/// Render the grant reasons and exposure text shown before auto-grant consent.
pub fn consent_lines(new: &[(String, String)], rows: &[Grant]) -> String {
    let mut output = String::new();
    for (name, reason) in new {
        output.push_str(name);
        output.push_str(": ");
        output.push_str(reason);
        if let Some(row) = rows.iter().find(|row| row.name == name) {
            output.push_str(" (");
            output.push_str(row.exposes);
            output.push(')');
            if let Some(escape) = row.escape {
                output.push_str("\n  can escape: ");
                output.push_str(escape);
            }
        }
        output.push('\n');
    }
    output
}

/// Render escape warnings for grants that are already present in a session plan.
pub fn escape_notice(session: &str, grants: &[String], rows: &[Grant]) -> Vec<String> {
    grants
        .iter()
        .filter_map(|name| {
            rows.iter()
                .find(|row| row.name == name && row.escape.is_some())
                .and_then(|row| {
                    row.escape.map(|escape| {
                        format!("session {session}: grant {name} can escape: {escape}")
                    })
                })
        })
        .collect()
}
/// Look up a grant by its configuration name.
pub fn lookup(name: &str) -> Result<&'static Grant> {
    if name == "containers" {
        bail!(
            "confinement grant \"containers\" was split into one grant per runtime: docker, podman, containerd, crio, libvirt, lxd, snap; run a2amx confine list"
        );
    }
    if let Some(grant) = all().iter().find(|grant| grant.name == name) {
        return Ok(grant);
    }
    let valid = all()
        .iter()
        .map(|grant| grant.name)
        .collect::<Vec<_>>()
        .join(", ");
    let mut closest = None;
    let mut close_count = 0;
    for grant in all() {
        let limit = if grant.name.chars().count() <= 3 {
            1
        } else {
            2
        };
        if edit_distance_at_most(name, grant.name, limit) {
            close_count += 1;
            closest = Some(grant);
        }
    }
    if close_count == 1
        && let Some(closest) = closest
    {
        bail!(
            "unknown confinement grant {name:?}; valid grants: {valid}; closest match: {}",
            closest.name
        );
    }
    bail!("unknown confinement grant {name:?}; valid grants: {valid}");
}

/// Validate a list of grant names and reject exact duplicates.
pub fn validate(names: &[String]) -> Result<()> {
    let mut seen = Vec::with_capacity(names.len());
    for name in names {
        lookup(name)?;
        if seen.contains(&name) {
            bail!("confinement grant {name:?} is repeated");
        }
        seen.push(name);
    }
    Ok(())
}

/// Render the grants in stable table order for `a2amx confine list`.
pub fn render_list() -> String {
    let mut output = String::new();
    let name_width = all()
        .iter()
        .map(|grant| grant.name.len())
        .max()
        .unwrap_or(0)
        + 1;
    for grant in all() {
        let scope = if grant.team_default_ok {
            "team or session"
        } else {
            "session only"
        };
        output.push_str(&format!(
            "{:<name_width$}{:<15}{}\n",
            grant.name, scope, grant.exposes
        ));
        if let Some(escape) = grant.escape {
            output.push_str(&format!("  can escape: {escape}\n"));
        }
    }
    output.push_str(
        "Give a grant with --confine-grant NAME, or grants = [\"NAME\"] in [session.confine].",
    );
    output
}

fn edit_distance_at_most(left: &str, right: &str, limit: usize) -> bool {
    edit_distance(left, right) <= limit
}

fn edit_distance(left: &str, right: &str) -> usize {
    let mut previous: Vec<usize> = (0..=right.chars().count()).collect();
    for (left_index, left_char) in left.chars().enumerate() {
        let mut current = vec![left_index + 1; previous.len()];
        for (right_index, right_char) in right.chars().enumerate() {
            current[right_index + 1] = if left_char == right_char {
                previous[right_index]
            } else {
                1 + previous[right_index]
                    .min(previous[right_index + 1])
                    .min(current[right_index])
            };
        }
        previous = current;
    }
    previous[right.chars().count()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consent_refusal_names_grants_in_order_and_hints_yes() {
        let new = vec![
            ("cache".to_owned(), "cache files".to_owned()),
            ("cargo".to_owned(), "Cargo files".to_owned()),
        ];

        assert_eq!(
            consent_refusal(&new),
            "auto confinement would open: cache, cargo; run a2amx team up on a terminal to accept them, or pass --yes for this run"
        );
    }
}
