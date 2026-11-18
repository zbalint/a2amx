use std::fs::{self, OpenOptions};
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rustix::fs::OFlags;

pub(super) fn parse_core_hooks_path(common: &Path, worktree: &Path) -> Result<Option<PathBuf>> {
    let config = common.join("config");
    let Ok(file) = OpenOptions::new()
        .read(true)
        .custom_flags((OFlags::NONBLOCK | OFlags::NOCTTY).bits() as i32)
        .open(&config)
    else {
        return Ok(None);
    };
    let metadata = file
        .metadata()
        .with_context(|| format!("failed to stat git config {}", config.display()))?;
    if !metadata.is_file() {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    let read = file
        .take((GIT_CONFIG_PLAN_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read git config {}", config.display()))?;
    if read > GIT_CONFIG_PLAN_LIMIT {
        bail!(
            "git config {} is larger than {} bytes; the confinement plan cannot check its hooksPath",
            config.display(),
            GIT_CONFIG_PLAN_LIMIT
        );
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut in_core = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_core = line.eq_ignore_ascii_case("[core]");
            continue;
        }
        if !in_core {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("hooksPath") {
            let value = value.trim().trim_matches(['"', '\'']);
            let path = PathBuf::from(value);
            return Ok(Some(if path.is_absolute() {
                path
            } else {
                worktree.join(path)
            }));
        }
    }
    Ok(None)
}

const POINTER_LIMIT: usize = 4096;
// The planner fails closed on oversize; auto detection has a separate, smaller cap.
const GIT_CONFIG_PLAN_LIMIT: usize = 1024 * 1024;

fn read_pointer_text(path: &Path) -> Option<String> {
    if !fs::metadata(path).ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    let read = fs::File::open(path)
        .ok()?
        .take((POINTER_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    if read > POINTER_LIMIT {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn is_git_dir_shape(common: &Path) -> bool {
    fs::metadata(common.join("HEAD")).is_ok_and(|metadata| metadata.is_file())
        && fs::metadata(common.join("objects")).is_ok_and(|metadata| metadata.is_dir())
}

/// Returns the canonical common git directory and worktree root, or `None` for no repository, a home-directory repository, or an unusable pointer; `cwd` must be canonical.
pub fn git_common_dir(cwd: &Path, home: &Path) -> Option<(PathBuf, PathBuf)> {
    let mut current = cwd.to_owned();
    loop {
        if current == home {
            return None;
        }
        let marker = current.join(".git");
        if marker.is_dir() {
            return fs::canonicalize(marker)
                .ok()
                .map(|common| (common, current));
        }
        if marker.is_file() {
            let text = read_pointer_text(&marker)?;
            let target = text
                .lines()
                .find_map(|line| line.trim().strip_prefix("gitdir:"))?
                .trim();
            let gitdir = PathBuf::from(target);
            let gitdir = if gitdir.is_absolute() {
                gitdir
            } else {
                current.join(gitdir)
            };
            let gitdir = fs::canonicalize(gitdir).ok()?;
            let commondir = gitdir.join("commondir");
            let common = match fs::metadata(&commondir) {
                Ok(metadata) if metadata.is_file() => {
                    let relative = read_pointer_text(&commondir)?;
                    let path = PathBuf::from(relative.trim());
                    if path.is_absolute() {
                        path
                    } else {
                        gitdir.join(path)
                    }
                }
                Ok(_) => return None,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => gitdir,
                Err(_) => return None,
            };
            let common = fs::canonicalize(common).ok()?;
            if common == home || !is_git_dir_shape(&common) {
                return None;
            }
            return Some((common, current));
        }
        let parent = current.parent()?.to_owned();
        if parent == current {
            return None;
        }
        current = parent;
    }
}
