//! A2AMX root resolution.

use std::path::PathBuf;

pub(super) fn resolve_home(explicit: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    Ok(
        crate::layout::Layout::resolve(explicit, std::env::var_os("HOME").map(PathBuf::from))?
            .root()
            .to_owned(),
    )
}
