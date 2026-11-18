use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};

use crate::grant;
use crate::harness::Harness;

use super::git::{git_common_dir, parse_core_hooks_path};
use super::harness_data::{
    harness_data, harness_overlay_parents, overlay_protected_entries, unsaved_harness_files,
};
use super::{
    CREDENTIAL_ENV, ConfineRequest, Host, MAX_PATH_BYTES, Plan, PlanBuilder, Unresolved,
    add_canonical_existing, add_unique, canonical_existing, canonical_if_exists,
    check_absolute_entry, check_entry_count, check_entry_syntax, check_write_allowed,
    check_write_runtime_allowed, is_within, path_key, user_path,
};

fn plan_empty_file(layout: &crate::layout::Layout) -> Result<PathBuf> {
    let candidate = layout.confine_empty();
    let parent = candidate
        .parent()
        .ok_or_else(|| anyhow!("empty file path has no parent"))?;
    let parent = canonical_existing(parent, "empty file directory")?;
    if parent != canonical_existing(&layout.run_dir(), "a2amx run directory")? {
        bail!("empty file must be inside the a2amx run directory");
    }
    if candidate.exists() {
        let path = canonical_existing(&candidate, "empty file")?;
        if !is_within(&path, &layout.run_dir()) || fs::metadata(&path)?.len() != 0 {
            bail!("confinement empty file must be an empty run file");
        }
        Ok(path)
    } else {
        Ok(candidate)
    }
}
fn add_validated_write(
    path: &Path,
    host: &Host,
    writable: &mut Vec<PathBuf>,
    skipped: &mut Vec<PathBuf>,
    explicit: bool,
) -> Result<Option<PathBuf>> {
    let Some(path) = add_canonical_existing(path, "write", skipped) else {
        return Ok(None);
    };
    let path = check_write_allowed(&path, &host.home, host.layout.root(), explicit)?;
    check_write_runtime_allowed(&path, &host.runtime_dir, explicit)?;
    add_unique(writable, path.clone());
    Ok(Some(path))
}

/// Build the complete mount plan using the daemon's current `PATH`.
pub fn plan(request: &ConfineRequest, harness: Harness, cwd: &Path, host: &Host) -> Result<Plan> {
    plan_with_path(request, harness, cwd, host, std::env::var_os("PATH"))
}

/// Build the complete mount plan with an explicit daemon `PATH` value.
pub fn plan_with_path(
    request: &ConfineRequest,
    harness: Harness,
    cwd: &Path,
    host: &Host,
    path_var: Option<OsString>,
) -> Result<Plan> {
    plan_with_preflight(request, harness, cwd, host, path_var, Vec::new())
}

/// Build a plan after the grant-directory preflight.
pub fn plan_with_preflight(
    request: &ConfineRequest,
    harness: Harness,
    cwd: &Path,
    host: &Host,
    path_var: Option<OsString>,
    preflight: Vec<Unresolved>,
) -> Result<Plan> {
    check_request_limits(request)?;
    crate::grant::validate(&request.grants)?;
    let mut builder = PlanBuilder::new(host, harness)?;
    builder.unresolved.extend(preflight);
    let cwd = canonical_existing(cwd, "cwd")?;
    if !cwd.is_dir() {
        bail!("cwd is not a directory: {}", cwd.display());
    }
    plan_empty_file(&builder.layout)?;
    builder.bind_workspace(&cwd)?;
    builder.bind_harness_data(request)?;
    let mut effective_grants = request.grants.clone();
    if matches!(harness, Harness::Claude | Harness::Omp | Harness::Codex)
        && !request.grants.iter().any(|grant| grant == "harness-config")
    {
        let root = match harness {
            Harness::Claude => builder.home.join(".claude"),
            Harness::Omp => builder.home.join(".omp"),
            Harness::Codex => builder.home.join(".codex"),
            Harness::Generic | Harness::Agy => unreachable!(),
        };
        let root = canonical_if_exists(&root);
        let operator_root = root.as_ref().is_some_and(|root| {
            request
                .hide
                .iter()
                .chain(&request.unhide)
                .map(|path| user_path(path, &builder.home))
                .filter_map(|path| canonical_if_exists(&path))
                .any(|path| path.as_path() == root.as_path())
        });
        if operator_root {
            if let Some(root) = root {
                builder.unresolved.push(Unresolved {
                    option: "harness-config",
                    path: root,
                    reason: "operator hide or unhide of the harness root".into(),
                });
            }
        } else {
            effective_grants.push("harness-config".into());
        }
    }
    builder.bind_harness_config(&effective_grants)?;
    builder.bind_operator_writes(request, path_var.as_deref())?;
    builder.hide_mandatory()?;
    builder.hide_defaults(request);
    builder.hide_containers(&request.grants);
    builder.apply_grant_unhides(&request.grants, request.no_default_hides)?;
    builder.apply_grant_stand_ins(&request.grants);
    builder.release_operator_writes()?;
    builder.apply_operator_hides(request)?;
    builder.protect_system_paths(path_var.as_deref());
    builder.finish(request, cwd, effective_grants)
}

fn check_request_limits(request: &ConfineRequest) -> Result<()> {
    check_entry_count(&request.write, "write")?;
    check_entry_count(&request.hide, "hide")?;
    check_entry_count(&request.unhide, "unhide")?;
    check_entry_count(&request.grants, "grants")?;
    check_entry_count(&request.keep_env, "keep_env")?;
    let mut seen_keep_env = BTreeSet::new();
    for name in &request.keep_env {
        if !CREDENTIAL_ENV.contains(&name.as_str()) {
            if let Some(grant) = grant::all()
                .iter()
                .find(|grant| grant.env.contains(&name.as_str()))
            {
                bail!(
                    "keep_env {name:?} is not unset by default, so there is nothing to keep; it returns with the {} grant",
                    grant.name
                );
            }
            bail!("keep_env {name:?} is not unset by default, so there is nothing to keep");
        }
        if !seen_keep_env.insert(name.as_str()) {
            bail!("keep_env {name:?} is repeated");
        }
    }

    for (option, entries) in [
        ("write", &request.write),
        ("hide", &request.hide),
        ("unhide", &request.unhide),
    ] {
        for entry in entries {
            check_entry_syntax(option, entry)?;
            check_absolute_entry(option, entry)?;
        }
    }
    if let Some(path) = request.team_file.as_deref() {
        if path.len() > MAX_PATH_BYTES {
            bail!("team_file path exceeds {MAX_PATH_BYTES} bytes");
        }
        check_absolute_entry("team_file", path)?;
    }
    Ok(())
}

impl<'a> PlanBuilder<'a> {
    fn new(host: &'a Host, harness: Harness) -> Result<Self> {
        Ok(Self {
            host,
            harness,
            home: canonical_existing(&host.home, "home")?,
            layout: crate::layout::Layout::new(canonical_existing(
                host.layout.root(),
                "a2amx root",
            )?),
            runtime_dir: canonical_existing(&host.runtime_dir, "runtime directory")?,
            writable: Vec::new(),
            readonly: Vec::new(),
            hidden: Vec::new(),
            skipped: Vec::new(),
            unresolved: Vec::new(),
            explicit_writes: BTreeSet::new(),
            implicit_writes: Vec::new(),
            mandatory_hidden: Vec::new(),
            default_hidden: Vec::new(),
            container_hidden: Vec::new(),
            container_sockets: Vec::new(),
            grant_unhides: Vec::new(),
            harness_config: None,
        })
    }

    fn bind_workspace(&mut self, cwd: &Path) -> Result<()> {
        if let Some(path) =
            add_validated_write(cwd, self.host, &mut self.writable, &mut self.skipped, false)?
        {
            self.implicit_writes.push((path, "working directory"));
        }
        if let Some((common, worktree)) = git_common_dir(cwd, &self.home) {
            if let Some(path) = add_validated_write(
                &common,
                self.host,
                &mut self.writable,
                &mut self.skipped,
                false,
            )? {
                self.implicit_writes.push((path, "git directory"));
            }
            self.protect_git_metadata(&common, &worktree)?;
        }
        Ok(())
    }

    fn protect_git_metadata(&mut self, common: &Path, worktree: &Path) -> Result<()> {
        for name in ["hooks", "config", "config.worktree"] {
            if let Ok(path) = common.join(name).canonicalize() {
                add_unique(&mut self.readonly, path);
            }
        }
        if let Some(path) = parse_core_hooks_path(common, worktree)?
            && let Ok(path) = path.canonicalize()
        {
            add_unique(&mut self.readonly, path);
        }
        Ok(())
    }

    pub(super) fn resolve_operator_path(
        &mut self,
        option: &'static str,
        source: &Path,
    ) -> Option<PathBuf> {
        match fs::canonicalize(source) {
            Ok(path) => Some(path),
            Err(error) => {
                add_unique(&mut self.skipped, source.to_owned());
                self.unresolved.push(Unresolved {
                    option,
                    path: source.to_owned(),
                    reason: if error.kind() == std::io::ErrorKind::NotFound {
                        "not found".into()
                    } else {
                        error.kind().to_string()
                    },
                });
                tracing::debug!(
                    path = %source.display(),
                    %option,
                    error = %error,
                    "operator confinement path unresolved"
                );
                None
            }
        }
    }

    fn bind_operator_writes(
        &mut self,
        request: &ConfineRequest,
        path_var: Option<&OsStr>,
    ) -> Result<()> {
        for path in &request.write {
            let source = user_path(path, &self.home);
            if let Some(source) = self.resolve_operator_path("write", &source)
                && let Some(path) = add_validated_write(
                    &source,
                    self.host,
                    &mut self.writable,
                    &mut self.skipped,
                    true,
                )?
            {
                self.explicit_writes.insert(path);
            }
        }
        if let Some(team_file) = &request.team_file
            && let Some(path) = add_canonical_existing(
                &user_path(team_file, &self.home),
                "team file",
                &mut self.skipped,
            )
            && self.writable.iter().any(|write| is_within(&path, write))
        {
            add_unique(&mut self.readonly, path);
        }
        self.protect_system_paths(path_var);
        Ok(())
    }

    fn protect_system_paths(&mut self, path_var: Option<&OsStr>) {
        if let Some(path) = canonical_if_exists(self.layout.root())
            && self.writable.iter().any(|write| is_within(&path, write))
        {
            add_unique(&mut self.readonly, path);
        }
        if let Some(path) = canonical_if_exists(&self.home.join(".a2amx"))
            && is_within(&path, &self.home)
            && self.writable.iter().any(|write| is_within(&path, write))
        {
            add_unique(&mut self.readonly, path);
        }
        if let Some(path_var) = path_var {
            for source in std::env::split_paths(path_var) {
                if !source.is_absolute() || !source.is_dir() {
                    continue;
                }
                let Some(path) = canonical_if_exists(&source) else {
                    continue;
                };
                if is_within(&path, &self.home)
                    && self.writable.iter().any(|write| is_within(&path, write))
                {
                    add_unique(&mut self.readonly, path);
                }
            }
        }
    }

    fn check_implicit_writes_not_hidden(&self) -> Result<()> {
        let protected_hides = self
            .default_hidden
            .iter()
            .chain(&self.mandatory_hidden)
            .chain(&self.container_hidden);
        let mut offenders = Vec::new();
        for (path, source) in &self.implicit_writes {
            if self.explicit_writes.contains(path) {
                continue;
            }
            for hidden in protected_hides.clone() {
                if is_within(path, hidden) {
                    offenders.push(format!(
                        "{source} {} resolves to the hidden path {}; open it with write = [\"{}\"] if that is intended",
                        path.display(),
                        hidden.display(),
                        path.display()
                    ));
                }
            }
        }
        if offenders.is_empty() {
            Ok(())
        } else {
            bail!("{}", offenders.join("; "));
        }
    }

    fn finish(
        mut self,
        request: &ConfineRequest,
        cwd: PathBuf,
        mut effective_grants: Vec<String>,
    ) -> Result<Plan> {
        self.readonly
            .retain(|path| !self.explicit_writes.contains(path));
        if let Some(root) = self.harness_config.as_ref() {
            if self.hidden.iter().any(|path| path == root) {
                bail!(
                    "confinement grant \"harness-config\" conflicts with the hide of {}",
                    root.display()
                );
            }
            if self.readonly.iter().any(|path| path == root) {
                bail!(
                    "confinement grant \"harness-config\" conflicts with the unhide of {}",
                    root.display()
                );
            }
        }
        for (path, grant) in &self.grant_unhides {
            if self.hidden.iter().any(|existing| existing == path) {
                bail!(
                    "confinement grant \"{grant}\" conflicts with the hide of {}",
                    path.display()
                );
            }
        }
        self.check_implicit_writes_not_hidden()?;
        let protected = protected_paths(&self.readonly, &self.writable);
        let (data_dirs, data_files) = harness_data(self.harness, &self.home);
        let overlay_parents = harness_overlay_parents(
            &self.home,
            &data_files,
            &self.writable,
            &self.hidden,
            &self.readonly,
            &protected,
        );
        let overlay_protected = overlay_protected_entries(
            &overlay_parents,
            &data_dirs,
            &data_files,
            &self.writable,
            &self.readonly,
            &protected,
            &self.hidden,
        );
        let unsaved_files = unsaved_harness_files(&data_files, &overlay_parents);
        self.readonly.sort();
        self.writable.sort();
        self.hidden.sort();
        self.mandatory_hidden.sort();
        self.default_hidden.sort();
        self.container_hidden.sort();
        self.skipped.sort();
        self.unresolved.sort_by(|left, right| {
            left.option
                .cmp(right.option)
                .then_with(|| left.path.cmp(&right.path))
        });

        let mut grants = request.grants.clone();
        grants.sort();
        effective_grants.sort();
        effective_grants.dedup();
        let mut keep_env = request.keep_env.clone();
        keep_env.sort();
        let plan = Plan {
            bwrap: PathBuf::from("bwrap"),
            layout: self.layout,
            cwd,
            harness: self.harness,
            writable: self.writable,
            readonly: self.readonly,
            protected,
            hidden: self.hidden,
            skipped: self.skipped,
            unresolved: self.unresolved,
            grants,
            effective_grants,
            auto: request.auto,
            reasons: request.reasons.clone(),
            write: request.write.clone(),
            hide: request.hide.clone(),
            unhide: request.unhide.clone(),
            no_default_hides: request.no_default_hides,
            keep_env,
            mandatory_hidden: self.mandatory_hidden,
            default_hidden: self.default_hidden,
            container_hidden: self.container_hidden,
            overlay_parents,
            overlay_protected,
            unsaved_files,
            overlay_notice: None,
            harness_config: self.harness_config,
            codex_dir: None,
        };
        // Force the expensive argument-size validation while still in the
        // daemon-side preflight path, before a PTY can be created.
        plan.wrap(vec!["true".to_owned()])?;
        Ok(plan)
    }
}
fn protected_paths(readonly: &[PathBuf], writable: &[PathBuf]) -> Vec<PathBuf> {
    let mut protected = Vec::new();
    let mut protected_set = BTreeSet::new();
    for path in readonly {
        if writable.iter().any(|write| is_within(path, write))
            && protected_set.insert(path_key(path))
        {
            protected.push(path.clone());
        }
    }
    protected.sort();
    protected
}
