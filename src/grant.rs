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
}

static GRANTS: [Grant; 1] = [Grant {
    name: "containers",
    exposes: "every container and virtualization socket on the host (docker, podman, containerd, lxd, libvirt, snapd and others): effectively root on the host",
    team_default_ok: false,
}];

/// Return every grant in stable display order.
pub fn all() -> &'static [Grant] {
    &GRANTS
}

/// Look up a grant by its configuration name.
pub fn lookup(name: &str) -> Result<&'static Grant> {
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
        if edit_distance_at_most(name, grant.name, 2) {
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
