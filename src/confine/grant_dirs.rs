use std::fs;
use std::path::{Path, PathBuf};

use crate::grant;
use crate::harness::Harness;

use super::hides::default_hide_sources_for;
use super::{
    ConfineRequest, Host, Unresolved, canonical_if_exists, check_write_allowed, is_within,
    paths_overlap, symlink_component, user_path,
};

struct GrantDirScope {
    home: PathBuf,
    root: PathBuf,
    runtime_dir: PathBuf,
    hidden: Vec<PathBuf>,
}

impl GrantDirScope {
    fn overlaps_reserved(&self, path: &Path) -> bool {
        paths_overlap(path, &self.root) || paths_overlap(path, &self.runtime_dir)
    }

    fn hides(&self, path: &Path) -> bool {
        self.hidden.iter().any(|hidden| is_within(path, hidden))
    }

    fn ancestor_unwritable(&self, ancestor: &Path) -> bool {
        if !is_within(ancestor, &self.home) || self.overlaps_reserved(ancestor) {
            return true;
        }
        if ancestor != self.home
            && check_write_allowed(ancestor, &self.home, &self.root, false).is_err()
        {
            return true;
        }
        self.hides(ancestor)
    }
}

fn grant_dir_scope(
    request: &ConfineRequest,
    harness: Harness,
    host: &Host,
) -> Option<GrantDirScope> {
    let home = fs::canonicalize(&host.home).ok()?;
    let root = fs::canonicalize(host.layout.root()).ok()?;
    let runtime_dir = fs::canonicalize(&host.runtime_dir).ok()?;
    let mut hidden = if request.no_default_hides {
        Vec::new()
    } else {
        default_hide_sources_for(&home, harness)
    };
    hidden.extend(
        request
            .hide
            .iter()
            .map(|path| user_path(path, &home))
            .filter_map(|path| canonical_if_exists(&path)),
    );
    let unhidden = request
        .unhide
        .iter()
        .map(|path| user_path(path, &home))
        .filter_map(|path| canonical_if_exists(&path))
        .collect::<Vec<_>>();
    hidden.retain(|path| !unhidden.iter().any(|unhidden| unhidden == path));
    Some(GrantDirScope {
        home,
        root,
        runtime_dir,
        hidden,
    })
}

fn existing_source_problem(source: &Path, scope: &GrantDirScope) -> Option<String> {
    if fs::canonicalize(source)
        .is_ok_and(|canonical| scope.overlaps_reserved(&canonical) || scope.hides(&canonical))
    {
        return Some("path overlaps protected location".into());
    }
    symlink_component(source).map(|path| format!("symlink component: {}", path.display()))
}

fn missing_source_problem(source: &Path, scope: &GrantDirScope) -> Option<String> {
    if let Some(path) = symlink_component(source) {
        return Some(format!("symlink component: {}", path.display()));
    }
    let Some(ancestor) = source.ancestors().find(|path| path.exists()) else {
        return Some("no existing ancestor".into());
    };
    let Ok(ancestor) = fs::canonicalize(ancestor) else {
        return Some("ancestor cannot be canonicalized".into());
    };
    scope
        .ancestor_unwritable(&ancestor)
        .then(|| "ancestor is not writable".into())
}

fn grant_dir_problem(source: &Path, scope: &GrantDirScope) -> Option<String> {
    if source.exists() {
        existing_source_problem(source, scope)
    } else {
        missing_source_problem(source, scope)
    }
}

fn ensure_grant_dir(source: PathBuf, scope: &GrantDirScope, create: bool) -> Option<Unresolved> {
    let reason = match grant_dir_problem(&source, scope) {
        Some(reason) => reason,
        None if !create || source.exists() => return None,
        None => fs::create_dir_all(&source).err()?.to_string(),
    };
    Some(Unresolved {
        option: "grant",
        path: source,
        reason,
    })
}

/// Best-effort creation for the tool directories requested by grants.
///
/// The planner repeats the path checks and records missing paths as skipped in
/// dry runs; this phase only creates safe directories for a real daemon start.
pub fn ensure_grant_dirs(
    request: &ConfineRequest,
    harness: Harness,
    host: &Host,
    create: bool,
) -> Vec<Unresolved> {
    let Some(scope) = grant_dir_scope(request, harness, host) else {
        return Vec::new();
    };
    request
        .grants
        .iter()
        .filter_map(|name| grant::lookup(name).ok())
        .flat_map(|grant| grant.create.iter())
        .filter_map(|relative| ensure_grant_dir(user_path(relative, &scope.home), &scope, create))
        .collect()
}
