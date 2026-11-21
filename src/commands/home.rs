//! A2AMX root resolution, migration, and old-layout warning.

use std::path::{Path, PathBuf};

pub(super) fn resolve_home(explicit: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    Ok(
        crate::layout::Layout::resolve(explicit, std::env::var_os("HOME").map(PathBuf::from))?
            .root()
            .to_owned(),
    )
}

pub(super) async fn migrate_old_state(
    explicit_home: bool,
    layout: &crate::layout::Layout,
) -> anyhow::Result<()> {
    if explicit_home {
        return Ok(());
    }
    let user_home = std::env::var_os("HOME").map(PathBuf::from);
    let old = crate::migrate::old_dir(
        std::env::var_os("XDG_STATE_HOME").as_deref(),
        user_home.as_deref(),
    );
    let Some(old) = old else { return Ok(()) };
    let layout_for_task = layout.clone();
    let old_for_task = old.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        crate::migrate::migrate(&layout_for_task, &old_for_task)
    })
    .await??;
    if let crate::migrate::Outcome::Migrated(report) = outcome {
        for (old, new) in report.moved {
            eprintln!("moved {} -> {}", old.display(), new.display());
        }
        for warning in report.warnings {
            eprintln!("{warning}");
        }
        eprintln!(
            "a2amx now uses {}; {} is no longer used and can be deleted: rm -r {}",
            layout.root().display(),
            old.display(),
            shell_quote(&old)
        );
    }
    Ok(())
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

pub(super) fn warn_explicit_old_layout(home: &Path) {
    let mut names = Vec::new();
    if home.join("messages.db").exists() {
        names.push("messages.db");
    }
    if home.join("team.toml").exists() {
        names.push("team.toml");
    }
    if home.join("gateway").exists() {
        names.push("gateway/");
    }
    if !names.is_empty() {
        eprintln!(
            "{} holds files in the old layout ({}); a2amx now reads {}/state, {}/config and {}/log; see docs/install.md",
            home.display(),
            names.join(", "),
            home.display(),
            home.display(),
            home.display()
        );
    }
}
