//! Refuses commands that would run with effective uid 0.

/// Err when `euid` is 0.
pub fn ensure_not_root_for(euid: u32) -> anyhow::Result<()> {
    if euid == 0 {
        anyhow::bail!("refusing to run as root (effective uid 0): run a2amx as an ordinary user");
    }
    Ok(())
}

/// Checks the process's effective uid.
pub fn ensure_not_root() -> anyhow::Result<()> {
    ensure_not_root_for(rustix::process::geteuid().as_raw())
}
