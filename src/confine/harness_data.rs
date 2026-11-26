use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};

use crate::grant;
use crate::harness::Harness;

use super::{
    ConfineRequest, PlanBuilder, add_canonical_existing, add_unique, canonical_if_exists,
    check_write_allowed, check_write_runtime_allowed, is_within, user_path,
};

#[derive(Default)]
pub(super) struct HarnessData {
    pub(super) dirs: Vec<PathBuf>,
    pub(super) files: Vec<PathBuf>,
}

fn harness_dir(harness: Harness) -> Option<&'static str> {
    match harness {
        Harness::Claude => Some(".claude"),
        Harness::Omp => Some(".omp"),
        Harness::Codex => Some(".codex"),
        Harness::Agy => Some(".gemini"),
        Harness::Generic => None,
    }
}

// Root of the harness data a plan opens and validates; Codex has a harness dir (session_harness_root) but no data, so None (removing that case would start refusing a symlinked ~/.codex).
fn data_root(harness: Harness, home: &Path) -> Option<PathBuf> {
    if harness == Harness::Codex {
        return None;
    }
    harness_dir(harness).map(|dir| home.join(dir))
}

fn sqlite_files(database: &Path) -> [PathBuf; 3] {
    [
        database.to_path_buf(),
        PathBuf::from(format!("{}-wal", database.display())),
        PathBuf::from(format!("{}-shm", database.display())),
    ]
}

fn is_database(path: &Path) -> bool {
    path.file_name()
        .and_then(OsStr::to_str)
        .is_some_and(|name| name.ends_with(".db"))
}

fn claude_data(home: &Path, root: &Path) -> HarnessData {
    let dirs = [
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
    ]
    .into_iter()
    .map(|name| root.join(name))
    .collect();
    let files = vec![
        root.join("history.jsonl"),
        root.join("stats-cache.json"),
        root.join(".credentials.json"),
        home.join(".claude.json"),
    ];
    HarnessData { dirs, files }
}

fn omp_data(root: &Path) -> HarnessData {
    let dirs = [
        "cache",
        "logs",
        "run",
        "agent/sessions",
        "agent/projects",
        "agent/cache",
        "agent/blobs",
    ]
    .into_iter()
    .map(|name| root.join(name))
    .collect();
    let mut files = Vec::new();
    let mut databases = BTreeSet::new();
    if let Ok(entries) = fs::read_dir(root.join("agent")) {
        for path in entries.flatten().map(|entry| entry.path()) {
            if is_database(&path) {
                databases.insert(path);
            }
        }
    }
    for database in databases {
        files.extend(sqlite_files(&database));
    }
    let stats = root.join("stats.db");
    if stats.is_file() {
        files.extend(sqlite_files(&stats));
    }
    HarnessData { dirs, files }
}

fn agy_data(root: &Path) -> HarnessData {
    HarnessData {
        dirs: vec![root.join("antigravity-cli")],
        files: Vec::new(),
    }
}

pub(super) fn harness_data(harness: Harness, home: &Path) -> HarnessData {
    let Some(root) = data_root(harness, home) else {
        return HarnessData::default();
    };
    match harness {
        Harness::Claude => claude_data(home, &root),
        Harness::Omp => omp_data(&root),
        Harness::Agy => agy_data(&root),
        // No-data case; data_root already returned None for Generic and Codex.
        Harness::Generic | Harness::Codex => HarnessData::default(),
    }
}

pub(super) struct PathSets<'a> {
    pub(super) writable: &'a [PathBuf],
    pub(super) hidden: &'a [PathBuf],
    pub(super) readonly: &'a [PathBuf],
    pub(super) protected: &'a [PathBuf],
}

impl PathSets<'_> {
    fn entries(&self) -> impl Iterator<Item = &PathBuf> {
        self.writable
            .iter()
            .chain(self.hidden)
            .chain(self.readonly)
            .chain(self.protected)
    }

    fn covers(&self, path: &Path) -> bool {
        self.entries().any(|entry| is_within(path, entry))
    }
}

pub(super) fn harness_overlay_parents(
    home: &Path,
    data_files: &[PathBuf],
    sets: &PathSets,
) -> Vec<PathBuf> {
    let mut parents = Vec::new();
    for file in data_files {
        let Some(parent) = file.parent().and_then(canonical_if_exists) else {
            continue;
        };
        if parent == home || !parent.is_dir() {
            continue;
        }
        if sets.covers(&parent) {
            continue;
        }
        add_unique(&mut parents, parent);
    }
    parents.sort();
    parents
}

fn plain_entries(parent: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut plain = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        let kind = metadata.file_type();
        if !kind.is_file() && !kind.is_dir() {
            continue;
        }
        plain.push(path);
    }
    plain
}

pub(super) fn overlay_protected_entries(
    parents: &[PathBuf],
    data: &HarnessData,
    sets: &PathSets,
) -> Vec<PathBuf> {
    let mut entries_protected = Vec::new();
    for parent in parents {
        for path in plain_entries(parent) {
            let excluded = data
                .dirs
                .iter()
                .chain(&data.files)
                .chain(sets.entries())
                .chain(parents)
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

struct DataRoots<'a> {
    home: &'a Path,
    home_file: Option<PathBuf>,
    sources: Vec<PathBuf>,
    roots: Vec<(PathBuf, PathBuf)>,
}

impl<'a> DataRoots<'a> {
    fn new(harness: Harness, home: &'a Path) -> Self {
        let sources = match data_root(harness, home) {
            Some(root) if harness == Harness::Omp => vec![root.join("agent"), root],
            Some(root) => vec![root],
            None => Vec::new(),
        };
        let roots = sources
            .iter()
            .filter_map(|source| {
                fs::canonicalize(source)
                    .ok()
                    .map(|root| (source.clone(), root))
            })
            .collect();
        let home_file = (harness == Harness::Claude).then(|| home.join(".claude.json"));
        Self {
            home,
            home_file,
            sources,
            roots,
        }
    }

    fn check_inside(&self, path: &Path, target: &Path) -> Result<()> {
        if self.home_file.as_deref() == Some(path) {
            let is_file = fs::metadata(target).is_ok_and(|metadata| metadata.is_file());
            if !is_file || target.parent() != Some(self.home) {
                return Err(outside_error(path, target, self.home));
            }
            return Ok(());
        }
        let Some((_, root)) = self
            .roots
            .iter()
            .find(|(source, _)| is_within(path, source))
        else {
            return Err(outside_error(path, target, self.home));
        };
        if !is_within(target, root) {
            return Err(outside_error(path, target, root));
        }
        Ok(())
    }
}

fn outside_error(path: &Path, target: &Path, root: &Path) -> anyhow::Error {
    anyhow!(
        "harness data path {} resolves to {}, outside {}; remove the link, keep the data inside {}, or open it with write = [\"{}\"] if that is intended",
        path.display(),
        target.display(),
        root.display(),
        root.display(),
        target.display()
    )
}

fn secure_data_dir(directory: &Path) -> Result<()> {
    match fs::symlink_metadata(directory) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            directory.parent().map(fs::create_dir_all).transpose()?;
            let Err(create_err) = fs::DirBuilder::new().mode(0o700).create(directory) else {
                return Ok(());
            };
            if create_err.kind() != io::ErrorKind::AlreadyExists {
                return Err(create_err.into());
            }
        }
        Err(error) => return Err(error.into()),
    }
    match fs::metadata(directory) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => bail!(
            "harness data path is not a directory: {}",
            directory.display()
        ),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Create the existing-harness data directories that are writable by a plan.
pub fn ensure_data_dirs(harness: Harness, home: &Path) -> Result<()> {
    let Some(root) = data_root(harness, home) else {
        return Ok(());
    };
    if !root.is_dir() {
        return Ok(());
    }
    let data = harness_data(harness, home);
    for directory in data.dirs {
        secure_data_dir(&directory)?;
    }
    Ok(())
}

fn ensure_sidecars(database: &Path) -> Result<()> {
    // SQLite cannot create WAL/SHM sidecars inside a read-only directory on a cold start.
    for sidecar in &sqlite_files(database)[1..] {
        let Err(error) = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(sidecar)
        else {
            continue;
        };
        // Leave an existing sidecar or target untouched; fail on a dangling or looping link.
        if error.kind() != io::ErrorKind::AlreadyExists || fs::metadata(sidecar).is_err() {
            return Err(error.into());
        }
    }
    Ok(())
}

/// Create only OMP's SQLite sidecars; never create a credential or database file.
pub fn ensure_data_files(harness: Harness, home: &Path) -> Result<()> {
    if harness != Harness::Omp {
        return Ok(());
    }
    let Some(root) = data_root(harness, home) else {
        return Ok(());
    };
    if !root.is_dir() {
        return Ok(());
    }
    let data = harness_data(Harness::Omp, home);
    for database in data.files.into_iter().filter(|path| is_database(path)) {
        if database.is_file() {
            ensure_sidecars(&database)?;
        }
    }
    Ok(())
}

fn sidecar_of_existing_database(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(OsStr::to_str) else {
        return false;
    };
    let Some(base) = name
        .strip_suffix("-wal")
        .or_else(|| name.strip_suffix("-shm"))
    else {
        return false;
    };
    path.with_file_name(base).is_file()
}

impl<'a> PlanBuilder<'a> {
    pub(super) fn session_harness_root(&self) -> Option<PathBuf> {
        harness_dir(self.harness).map(|dir| self.home.join(dir))
    }

    fn validate_harness_data_paths(
        &self,
        data: &HarnessData,
        explicit_writes: &BTreeSet<PathBuf>,
    ) -> Result<()> {
        let data_roots = DataRoots::new(self.harness, &self.home);
        for path in data_roots
            .sources
            .iter()
            .chain(&data.dirs)
            .chain(&data.files)
        {
            let Some(target) = canonical_if_exists(path) else {
                continue;
            };
            let explicit = explicit_writes.contains(&target);
            if !explicit {
                data_roots.check_inside(path, &target)?;
            }
            check_write_allowed(&target, &self.home, self.layout.root(), explicit)?;
            check_write_runtime_allowed(&target, &self.runtime_dir, explicit)?;
        }
        Ok(())
    }

    fn mount_data_dir(&mut self, path: &Path) -> Result<()> {
        match fs::canonicalize(path) {
            Ok(path) => add_unique(&mut self.writable, path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if fs::symlink_metadata(path).is_ok() {
                    bail!(
                        "harness data path cannot be canonicalized: {}",
                        path.display()
                    );
                }
                add_unique(&mut self.writable, path.to_path_buf());
            }
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }

    fn mount_data_file(&mut self, path: &Path) {
        if let Some(resolved) = add_canonical_existing(path, "harness data", &mut self.skipped) {
            add_unique(&mut self.writable, resolved);
        } else if sidecar_of_existing_database(path) {
            add_unique(&mut self.writable, path.to_path_buf());
        }
    }

    // Relies on validate_harness_data_paths having run first; the containment check lives only there.
    fn mount_harness_data(&mut self, data: &HarnessData) -> Result<()> {
        let Some(root) = data_root(self.harness, &self.home) else {
            return Ok(());
        };
        if !root.is_dir() {
            return Ok(());
        }
        for path in &data.dirs {
            self.mount_data_dir(path)?;
        }
        for path in &data.files {
            self.mount_data_file(path);
        }
        Ok(())
    }

    pub(super) fn bind_harness_data(&mut self, request: &ConfineRequest) -> Result<()> {
        let data = harness_data(self.harness, &self.home);
        let explicit_writes = request
            .write
            .iter()
            .filter_map(|entry| canonical_if_exists(&user_path(entry, &self.home)))
            .collect::<BTreeSet<_>>();
        self.validate_harness_data_paths(&data, &explicit_writes)?;
        self.mount_harness_data(&data)
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
        let data = harness_data(self.harness, &self.home);
        let data_paths = data
            .dirs
            .into_iter()
            .chain(data.files)
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
