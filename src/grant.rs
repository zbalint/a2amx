//! Named opt-in capabilities for confinement.

use anyhow::{Result, bail};

/// One named thing a user opens on purpose.
#[derive(Debug, PartialEq, Eq)]
pub struct Grant {
    pub name: &'static str,
    /// One line for the docs and for the later `confine list`.
    pub exposes: &'static str,
    /// Whether a team-level table may name this grant.
    pub team_default_ok: bool,
    /// Host paths hidden unless the grant is given (`~/` is the home directory).
    pub sockets: &'static [&'static str],
    /// Environment names unset unless the grant is given.
    pub env: &'static [&'static str],
    /// Whether this grant opens the current harness's own configuration directory.
    pub own_config: bool,
    /// Home-relative or absolute paths made visible read-only when the grant is given.
    pub unhides: &'static [&'static str],
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

static GRANTS: [Grant; 11] = [
    Grant {
        name: "docker",
        exposes: "docker container daemon sockets: effectively root on the host",
        team_default_ok: false,
        sockets: DOCKER_SOCKETS,
        env: DOCKER_ENV,
        own_config: false,
        unhides: NO_PATHS,
        stand_ins: NO_PATHS,
    },
    Grant {
        name: "podman",
        exposes: "podman container daemon sockets: effectively root on the host",
        team_default_ok: false,
        sockets: PODMAN_SOCKETS,
        env: PODMAN_ENV,
        own_config: false,
        unhides: NO_PATHS,
        stand_ins: NO_PATHS,
    },
    Grant {
        name: "containerd",
        exposes: "containerd, buildkit and k3s container daemon sockets: effectively root on the host",
        team_default_ok: false,
        sockets: CONTAINERD_SOCKETS,
        env: CONTAINERD_ENV,
        own_config: false,
        unhides: NO_PATHS,
        stand_ins: NO_PATHS,
    },
    Grant {
        name: "crio",
        exposes: "crio container daemon sockets: effectively root on the host",
        team_default_ok: false,
        sockets: CRIO_SOCKETS,
        env: NO_PATHS,
        own_config: false,
        unhides: NO_PATHS,
        stand_ins: NO_PATHS,
    },
    Grant {
        name: "libvirt",
        exposes: "libvirt virtual-machine control: controls virtual machines",
        team_default_ok: false,
        sockets: LIBVIRT_SOCKETS,
        env: LIBVIRT_ENV,
        own_config: false,
        unhides: NO_PATHS,
        stand_ins: NO_PATHS,
    },
    Grant {
        name: "lxd",
        exposes: "lxd and incus container daemon sockets: effectively root on the host",
        team_default_ok: false,
        sockets: LXD_SOCKETS,
        env: NO_PATHS,
        own_config: false,
        unhides: NO_PATHS,
        stand_ins: NO_PATHS,
    },
    Grant {
        name: "snap",
        exposes: "snapd package management: installs and removes packages",
        team_default_ok: false,
        sockets: SNAP_SOCKETS,
        env: NO_PATHS,
        own_config: false,
        unhides: NO_PATHS,
        stand_ins: NO_PATHS,
    },
    Grant {
        name: "harness-config",
        exposes: "the session's own harness config directory, read-write and persistent: settings, hooks, instructions and programs under it; running sessions of that harness pick up changes immediately",
        team_default_ok: false,
        sockets: NO_PATHS,
        env: NO_PATHS,
        own_config: true,
        unhides: NO_PATHS,
        stand_ins: NO_PATHS,
    },
    Grant {
        name: "gh",
        exposes: "your GitHub CLI login (~/.config/gh) and its tokens in the environment: the agent can push, open and merge pull requests and read private repositories as you",
        team_default_ok: true,
        sockets: NO_PATHS,
        env: GH_ENV,
        own_config: false,
        unhides: GH_UNHIDES,
        stand_ins: NO_PATHS,
    },
    Grant {
        name: "git-credentials",
        exposes: "the plain-text git credential files: every host and password stored there",
        team_default_ok: true,
        sockets: NO_PATHS,
        env: NO_PATHS,
        own_config: false,
        unhides: GIT_CREDENTIAL_UNHIDES,
        stand_ins: NO_PATHS,
    },
    Grant {
        name: "ssh",
        exposes: "your ssh keys and config (~/.ssh, read-only): the agent can log in to every host those keys open as you, and push with them",
        team_default_ok: true,
        sockets: NO_PATHS,
        env: NO_PATHS,
        own_config: false,
        unhides: SSH_UNHIDES,
        stand_ins: SSH_STAND_INS,
    },
];

/// Return every grant in stable display order.
pub fn all() -> &'static [Grant] {
    &GRANTS
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
    for grant in all() {
        let scope = if grant.team_default_ok {
            "team or session"
        } else {
            "session only"
        };
        output.push_str(&format!(
            "{:<16}{:<15}{}\n",
            grant.name, scope, grant.exposes
        ));
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
