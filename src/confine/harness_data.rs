use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};

use crate::grant;
use crate::harness::Harness;

use super::{
    ConfineRequest, PlanBuilder, add_canonical_existing, add_unique, canonical_if_exists,
    check_write_allowed, check_write_runtime_allowed, is_within, user_path,
};

pub(super) fn harness_data(harness: Harness, home: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    match harness {
        Harness::Claude => {
            let root = home.join(".claude");
            for name in [
                "projects",
                "todos",
                "shell-snapshots",
                "session-env",
                "sessions",
                "statsig",
                "file-history",
                "paste-cache",
                "debug",
                "cache",
                "telemetry",
                "plans",
                "backups",
            ] {
                dirs.push(root.join(name));
            }
            files.extend([
                root.join("history.jsonl"),
                root.join("stats-cache.json"),
                root.join(".credentials.json"),
                home.join(".claude.json"),
            ]);
        }
        Harness::Omp => {
            let root = home.join(".omp");
            for name in [
                "cache",
                "logs",
                "run",
                "agent/sessions",
                "agent/projects",
                "agent/cache",
                "agent/blobs",
            ] {
                dirs.push(root.join(name));
            }
            let parent = root.join("agent");
            let mut databases = BTreeSet::new();
            if let Ok(entries) = fs::read_dir(&parent) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path
                        .file_name()
                        .and_then(OsStr::to_str)
                        .is_some_and(|name| name.ends_with(".db"))
                    {
                        databases.insert(path);
                    }
                }
            }
            for database in databases {
                files.push(database.clone());
                files.push(PathBuf::from(format!("{}-wal", database.display())));
                files.push(PathBuf::from(format!("{}-shm", database.display())));
            }
            let stats = root.join("stats.db");
            if stats.is_file() {
                files.extend([
                    stats.clone(),
                    PathBuf::from(format!("{}-wal", stats.display())),
                    PathBuf::from(format!("{}-shm", stats.display())),
                ]);
            }
        }
        Harness::Agy => {
            dirs.push(home.join(".gemini/antigravity-cli"));
        }
        Harness::Generic | Harness::Codex => {}
    }
    (dirs, files)
}

pub(super) fn harness_overlay_parents(
    home: &Path,
    data_files: &[PathBuf],
    writable: &[PathBuf],
    hidden: &[PathBuf],
    readonly: &[PathBuf],
    protected: &[PathBuf],
) -> Vec<PathBuf> {
    let mut parents = Vec::new();
    for file in data_files {
        let Some(parent) = file.parent().and_then(canonical_if_exists) else {
            continue;
        };
        if parent == home || !parent.is_dir() {
            continue;
        }
        if [writable, hidden, readonly, protected]
            .into_iter()
            .any(|paths| paths.iter().any(|path| is_within(&parent, path)))
        {
            continue;
        }
        add_unique(&mut parents, parent);
    }
    parents.sort();
    parents
}

pub(super) fn overlay_protected_entries(
    parents: &[PathBuf],
    data_dirs: &[PathBuf],
    data_files: &[PathBuf],
    writable: &[PathBuf],
    readonly: &[PathBuf],
    protected: &[PathBuf],
    hidden: &[PathBuf],
) -> Vec<PathBuf> {
    let mut entries_protected = Vec::new();
    for parent in parents {
        let Ok(entries) = fs::read_dir(parent) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.file_type().is_symlink()
                || (!metadata.file_type().is_file() && !metadata.file_type().is_dir())
            {
                continue;
            }
            let excluded = data_dirs
                .iter()
                .chain(data_files)
                .chain(writable)
                .chain(readonly)
                .chain(protected)
                .chain(parents)
                .chain(hidden)
                .any(|other| {
                    other == &path
                        || canonical_if_exists(other).is_some_and(|canonical| canonical == path)
                });
            if !excluded {
                add_unique(&mut entries_protected, path);
            }
        }
    }
    entries_protected.sort();
    entries_protected
}

pub(super) fn unsaved_harness_files(
    data_files: &[PathBuf],
    overlay_parents: &[PathBuf],
) -> Vec<PathBuf> {
    let mut unsaved = Vec::new();
    for file in data_files {
        if file.exists() {
            continue;
        }
        let Some(parent) = file.parent().and_then(canonical_if_exists) else {
            continue;
        };
        if !overlay_parents.contains(&parent) {
            continue;
        }
        let Some(name) = file.file_name().and_then(OsStr::to_str) else {
            continue;
        };
        if let Some(base) = name
            .strip_suffix("-wal")
            .or_else(|| name.strip_suffix("-shm"))
            && file.with_file_name(base).is_file()
        {
            continue;
        }
        add_unique(&mut unsaved, file.clone());
    }
    unsaved.sort();
    unsaved
}

fn validate_harness_data_paths(
    harness: Harness,
    home: &Path,
    root: &Path,
    runtime_dir: &Path,
    data_dirs: &[PathBuf],
    data_files: &[PathBuf],
    explicit_writes: &BTreeSet<PathBuf>,
) -> Result<()> {
    let expected_sources = match harness {
        Harness::Claude => vec![home.join(".claude")],
        Harness::Omp => vec![home.join(".omp/agent"), home.join(".omp")],
        Harness::Agy => vec![home.join(".gemini")],
        Harness::Generic | Harness::Codex => Vec::new(),
    };
    let expected_roots = expected_sources
        .iter()
        .filter_map(|source| fs::canonicalize(source).ok().map(|root| (source, root)))
        .collect::<Vec<_>>();
    let home_file = home.join(".claude.json");
    let outside_error = |path: &Path, target: &Path, root: &Path| -> anyhow::Error {
        anyhow!(
            "harness data path {} resolves to {}, outside {}; remove the link, keep the data inside {}, or open it with write = [\"{}\"] if that is intended",
            path.display(),
            target.display(),
            root.display(),
            root.display(),
            target.display()
        )
    };
    for path in expected_sources
        .iter()
        .cloned()
        .chain(data_dirs.iter().cloned())
        .chain(data_files.iter().cloned())
    {
        let Some(target) = canonical_if_exists(&path) else {
            continue;
        };
        let explicit = explicit_writes.contains(&target);
        if !explicit {
            if harness == Harness::Claude && path == home_file {
                let is_file = fs::metadata(&target).is_ok_and(|metadata| metadata.is_file());
                if !is_file || target.parent() != Some(home) {
                    return Err(outside_error(&path, &target, home));
                }
            } else {
                let Some((_, root)) = expected_roots
                    .iter()
                    .find(|(source, _)| is_within(&path, source))
                else {
                    return Err(outside_error(&path, &target, home));
                };
                if !is_within(&target, root) {
                    return Err(outside_error(&path, &target, root));
                }
            }
        }
        check_write_allowed(&target, home, root, explicit)?;
        check_write_runtime_allowed(&target, runtime_dir, explicit)?;
    }
    Ok(())
}
/// Create the existing-harness data directories that are writable by a plan.
pub fn ensure_data_dirs(harness: Harness, home: &Path) -> Result<()> {
    let root = match harness {
        Harness::Claude => home.join(".claude"),
        Harness::Omp => home.join(".omp"),
        Harness::Agy => home.join(".gemini"),
        Harness::Generic | Harness::Codex => return Ok(()),
    };
    if !root.is_dir() {
        return Ok(());
    }
    let (dirs, _) = harness_data(harness, home);
    for directory in dirs {
        if directory.exists() {
            if !directory.is_dir() {
                bail!(
                    "harness data path is not a directory: {}",
                    directory.display()
                );
            }
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        } else {
            fs::create_dir_all(&directory)?;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(())
}
/// Create only OMP's SQLite sidecars; never create a credential or database file.
pub fn ensure_data_files(harness: Harness, home: &Path) -> Result<()> {
    if harness != Harness::Omp || !home.join(".omp").is_dir() {
        return Ok(());
    }
    let (_, files) = harness_data(Harness::Omp, home);
    let dbs = files
        .into_iter()
        .filter(|path| {
            path.file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|name| name.ends_with(".db"))
        })
        .collect::<Vec<_>>();
    for db in dbs {
        if !db.is_file() {
            continue;
        }
        for suffix in ["-wal", "-shm"] {
            let sidecar = PathBuf::from(format!("{}{}", db.display(), suffix));
            if !sidecar.exists() {
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&sidecar)?;
            }
            fs::set_permissions(&sidecar, fs::Permissions::from_mode(0o600))?;
        }
    }
    Ok(())
}

fn add_harness_data_mounts(
    harness: Harness,
    home: &Path,
    data_dirs: &[PathBuf],
    data_files: &[PathBuf],
    writable: &mut Vec<PathBuf>,
    skipped: &mut Vec<PathBuf>,
) -> Result<()> {
    if harness == Harness::Generic || harness == Harness::Codex {
        return Ok(());
    }
    let root = match harness {
        Harness::Claude => home.join(".claude"),
        Harness::Omp => home.join(".omp"),
        Harness::Agy => home.join(".gemini"),
        Harness::Generic | Harness::Codex => unreachable!(),
    };
    if !root.is_dir() {
        return Ok(());
    }
    for path in data_dirs {
        match fs::canonicalize(path) {
            Ok(path) => add_unique(writable, path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if fs::symlink_metadata(path).is_ok() {
                    bail!(
                        "harness data path cannot be canonicalized: {}",
                        path.display()
                    );
                }
                add_unique(writable, path.clone());
            }
            Err(error) => return Err(error.into()),
        }
    }
    for path in data_files {
        if let Some(path) = add_canonical_existing(path, "harness data", skipped) {
            add_unique(writable, path);
            continue;
        }
        let Some(name) = path.file_name().and_then(OsStr::to_str) else {
            continue;
        };
        let Some(base) = name
            .strip_suffix("-wal")
            .or_else(|| name.strip_suffix("-shm"))
        else {
            continue;
        };
        let database = path.with_file_name(base);
        if database.is_file() {
            add_unique(writable, path.clone());
        }
    }
    Ok(())
}

impl<'a> PlanBuilder<'a> {
    pub(super) fn session_harness_root(&self) -> Option<PathBuf> {
        match self.harness {
            Harness::Claude => Some(self.home.join(".claude")),
            Harness::Omp => Some(self.home.join(".omp")),
            Harness::Codex => Some(self.home.join(".codex")),
            Harness::Agy => Some(self.home.join(".gemini")),
            Harness::Generic => None,
        }
    }

    pub(super) fn bind_harness_data(&mut self, request: &ConfineRequest) -> Result<()> {
        let (data_dirs, data_files) = harness_data(self.harness, &self.home);
        let explicit_writes = request
            .write
            .iter()
            .filter_map(|entry| canonical_if_exists(&user_path(entry, &self.home)))
            .collect::<BTreeSet<_>>();
        validate_harness_data_paths(
            self.harness,
            &self.home,
            self.layout.root(),
            &self.runtime_dir,
            &data_dirs,
            &data_files,
            &explicit_writes,
        )?;
        add_harness_data_mounts(
            self.harness,
            &self.home,
            &data_dirs,
            &data_files,
            &mut self.writable,
            &mut self.skipped,
        )
    }

    pub(super) fn bind_harness_config(&mut self, grants: &[String]) -> Result<()> {
        let own_config = grants.iter().any(|name| {
            grant::all()
                .iter()
                .any(|row| row.name == name && row.own_config)
        });
        if !own_config {
            return Ok(());
        }
        let Some(root) = self.session_harness_root() else {
            bail!("confinement grant \"harness-config\" needs a Claude, OMP, Codex, or Agy session")
        };
        let Some(root) = add_canonical_existing(&root, "harness config", &mut self.skipped) else {
            return Ok(());
        };
        let (data_dirs, data_files) = harness_data(self.harness, &self.home);
        let data_paths = data_dirs
            .into_iter()
            .chain(data_files)
            .filter_map(|path| {
                let path = canonical_if_exists(&path).unwrap_or(path);
                is_within(&path, &root).then_some(path)
            })
            .collect::<Vec<_>>();
        self.writable
            .retain(|path| !data_paths.iter().any(|data| data == path));
        self.skipped
            .retain(|path| !data_paths.iter().any(|data| data == path));
        self.harness_config = Some(root.clone());
        add_unique(&mut self.writable, root);
        Ok(())
    }
}
