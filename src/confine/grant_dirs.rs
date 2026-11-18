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

fn grant_dir_problem(source: &Path, scope: &GrantDirScope) -> Option<String> {
    if source.exists() {
        if let Ok(canonical) = fs::canonicalize(source)
            && (paths_overlap(&canonical, &scope.root)
                || paths_overlap(&canonical, &scope.runtime_dir)
                || scope
                    .hidden
                    .iter()
                    .any(|hidden| is_within(&canonical, hidden)))
        {
            return Some("path overlaps protected location".into());
        }
        if let Some(path) = symlink_component(source) {
            return Some(format!("symlink component: {}", path.display()));
        }
        return None;
    }
    if let Some(path) = symlink_component(source) {
        return Some(format!("symlink component: {}", path.display()));
    }
    let Some(ancestor) = source.ancestors().find(|path| path.exists()) else {
        return Some("no existing ancestor".into());
    };
    let Ok(ancestor) = fs::canonicalize(ancestor) else {
        return Some("ancestor cannot be canonicalized".into());
    };
    if !is_within(&ancestor, &scope.home)
        || paths_overlap(&ancestor, &scope.root)
        || paths_overlap(&ancestor, &scope.runtime_dir)
        || (ancestor != scope.home
            && check_write_allowed(&ancestor, &scope.home, &scope.root, false).is_err())
        || scope
            .hidden
            .iter()
            .any(|hidden| is_within(&ancestor, hidden))
    {
        return Some("ancestor is not writable".into());
    }
    None
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
    let mut unresolved = Vec::new();
    for name in &request.grants {
        let Ok(grant) = grant::lookup(name) else {
            continue;
        };
        for relative in grant.create {
            let source = user_path(relative, &scope.home);
            if let Some(reason) = grant_dir_problem(&source, &scope) {
                unresolved.push(Unresolved {
                    option: "grant",
                    path: source,
                    reason,
                });
            } else if create
                && !source.exists()
                && let Err(error) = fs::create_dir_all(&source)
            {
                unresolved.push(Unresolved {
                    option: "grant",
                    path: source,
                    reason: error.to_string(),
                });
            }
        }
    }
    unresolved
}
