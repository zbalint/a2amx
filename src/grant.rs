//! Named opt-in capabilities for confinement.

use std::collections::BTreeSet;

use anyhow::{Result, bail};

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

static GRANTS: [Grant; 30] = [
    Grant {
        name: "docker",
        exposes: "docker container daemon sockets: effectively root on the host",
        escape: None,
        team_default_ok: false,
        sockets: DOCKER_SOCKETS,
        env: DOCKER_ENV,
        own_config: false,
        unhides: NO_PATHS,
        stand_ins: NO_PATHS,
        detect: NO_PATHS,
        create: NO_PATHS,
    },
    Grant {
        name: "podman",
        exposes: "podman container daemon sockets: effectively root on the host",
        escape: None,
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
        exposes: "containerd, buildkit and k3s container daemon sockets: effectively root on the host",
        escape: None,
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
        exposes: "crio container daemon sockets: effectively root on the host",
        escape: None,
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
        exposes: "lxd and incus container daemon sockets: effectively root on the host",
        escape: None,
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

/// Parse accepted auto-grant names, discarding blank and unknown lines.
pub fn parse_accepted(text: &str) -> BTreeSet<String> {
    text.lines()
        .map(str::trim)
        .filter(|name| all().iter().any(|grant| grant.name == *name))
        .map(str::to_owned)
        .collect()
}

/// Render accepted auto-grant names in sorted order.
pub fn render_accepted(names: &BTreeSet<String>) -> String {
    if names.is_empty() {
        return String::new();
    }
    let mut rendered = names
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("\n");
    rendered.push('\n');
    rendered
}

pub const CONSENT_QUESTION: &str = "Accept these for auto mode on all projects? [y/N] ";

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
