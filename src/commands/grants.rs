//! Automatic grant detection, accepted grants, and consent.

use crate::terminal::confirm;
use crate::terminal::write_stderr;
use anyhow::{Context, anyhow, bail};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

const MAX_AUTO_GRANTS_BYTES: usize = 65_536;

fn read_accepted_grants(path: &Path) -> anyhow::Result<crate::grant::Accepted> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(BTreeMap::new());
        }
        Err(error) => {
            return Err(error).with_context(|| format!("cannot read {}", path.display()));
        }
    };
    if !metadata.is_file() {
        bail!(
            "accepted auto-grants.toml file {} is not a regular file",
            path.display()
        );
    }
    let file =
        std::fs::File::open(path).with_context(|| format!("cannot read {}", path.display()))?;
    let mut bytes = Vec::with_capacity(MAX_AUTO_GRANTS_BYTES + 1);
    let read = file
        .take((MAX_AUTO_GRANTS_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .with_context(|| format!("cannot read {}", path.display()))?;
    if read > MAX_AUTO_GRANTS_BYTES {
        bail!(
            "accepted auto-grants.toml file {} is larger than {} bytes",
            path.display(),
            MAX_AUTO_GRANTS_BYTES
        );
    }
    let text = String::from_utf8(bytes).with_context(|| {
        format!(
            "accepted auto-grants.toml file {} is not UTF-8",
            path.display()
        )
    })?;
    crate::grant::parse_accepted(&text)
        .with_context(|| format!("invalid accepted auto-grants.toml file {}", path.display()))
}

pub(super) async fn load_accepted_grants(home: &Path) -> anyhow::Result<crate::grant::Accepted> {
    let path = crate::layout::Layout::new(home.to_owned()).auto_grants();
    tokio::task::spawn_blocking(move || read_accepted_grants(&path)).await?
}

async fn save_accepted_grants(
    home: &Path,
    accepted: &crate::grant::Accepted,
) -> anyhow::Result<()> {
    let path = crate::layout::Layout::new(home.to_owned()).auto_grants();
    let content = crate::grant::render_accepted(accepted);
    tokio::task::spawn_blocking(move || crate::omp::write_if_changed(&path, content.as_bytes()))
        .await??;
    Ok(())
}

pub(super) fn project_key(dir: &Path) -> anyhow::Result<&str> {
    dir.to_str().ok_or_else(|| {
        anyhow!(
            "project directory is not valid UTF-8: {:?}",
            dir.as_os_str()
        )
    })
}

/// `--yes` applies to this run only and is not saved.
pub(super) async fn confirm_new_grants(
    home: &Path,
    project: &Path,
    accepted: &crate::grant::Accepted,
    new_grants: &[(String, String)],
    yes: bool,
) -> anyhow::Result<()> {
    let lines = crate::grant::consent_lines(new_grants, crate::grant::all());
    if yes {
        write_stderr(lines.into_bytes()).await?;
        return Ok(());
    }
    let prompt = format!("{lines}{}", crate::grant::consent_question(project));
    let refusal = crate::grant::consent_refusal(new_grants);
    if !confirm(&prompt, &refusal).await? {
        bail!("auto grants not accepted; nothing started");
    }
    let mut accepted = accepted.clone();
    accepted
        .entry(project_key(project)?.to_owned())
        .or_default()
        .extend(new_grants.iter().map(|(grant, _)| grant.clone()));
    // shortcut: concurrent team up runs can lose one project's acceptance; add a file lock if repeated prompts matter.
    save_accepted_grants(home, &accepted).await
}

const MAX_GIT_CONFIG_BYTES: usize = 65_536;

pub(super) fn auto_grants_for(dir: &Path, home: &Path) -> Vec<(&'static str, String)> {
    let canonical_dir = std::fs::canonicalize(dir).ok();
    let canonical_home = if home.as_os_str().is_empty() {
        home.to_owned()
    } else {
        std::fs::canonicalize(home).unwrap_or_else(|_| home.to_owned())
    };
    let git_config = canonical_dir.as_deref().and_then(|canonical_dir| {
        let git_dir = crate::confine::git_common_dir(canonical_dir, &canonical_home)
            .map(|(common, _)| common)?;
        let config_path = git_dir.join("config");
        let regular = std::fs::metadata(&config_path)
            .map(|metadata| metadata.is_file())
            .unwrap_or(false);
        if !regular {
            return None;
        }
        let mut bytes = Vec::new();
        std::fs::File::open(config_path).ok().and_then(|file| {
            let read = file
                .take((MAX_GIT_CONFIG_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .ok()?;
            (read <= MAX_GIT_CONFIG_BYTES)
                .then(|| String::from_utf8(bytes).ok())
                .flatten()
        })
    });
    crate::suggest::explain(
        |name| dir.join(name).exists(),
        git_config.as_deref(),
        |path| {
            if canonical_home.as_os_str().is_empty() {
                return false;
            }
            let relative = path.strip_prefix("~/").unwrap_or(path);
            canonical_home.join(relative).exists()
        },
    )
}
