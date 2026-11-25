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

/// Target environment for building a confinement mount plan.
#[derive(Clone, Copy)]
pub struct PlanTarget<'a> {
    pub harness: Harness,
    pub cwd: &'a Path,
    pub host: &'a Host,
}

/// Build the complete mount plan using the daemon's current `PATH`.
pub fn plan(request: &ConfineRequest, harness: Harness, cwd: &Path, host: &Host) -> Result<Plan> {
    plan_with_path(
        request,
        PlanTarget { harness, cwd, host },
        std::env::var_os("PATH"),
    )
}

/// Build the complete mount plan with an explicit daemon `PATH` value.
pub fn plan_with_path(
    request: &ConfineRequest,
    target: PlanTarget<'_>,
    path_var: Option<OsString>,
) -> Result<Plan> {
    plan_with_preflight(request, target, path_var, Vec::new())
}

/// Build a plan after the grant-directory preflight.
pub fn plan_with_preflight(
    request: &ConfineRequest,
    target: PlanTarget<'_>,
    path_var: Option<OsString>,
    preflight: Vec<Unresolved>,
) -> Result<Plan> {
    let PlanTarget { harness, cwd, host } = target;
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
    let effective_grants = builder.effective_grants(request);
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

fn check_counts(request: &ConfineRequest) -> Result<()> {
    check_entry_count(&request.write, "write")?;
    check_entry_count(&request.hide, "hide")?;
    check_entry_count(&request.unhide, "unhide")?;
    check_entry_count(&request.grants, "grants")?;
    check_entry_count(&request.keep_env, "keep_env")
}

fn check_keep_env(request: &ConfineRequest) -> Result<()> {
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
    Ok(())
}

fn check_entry_syntaxes(request: &ConfineRequest) -> Result<()> {
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
    Ok(())
}

fn check_team_file_path(request: &ConfineRequest) -> Result<()> {
    if let Some(path) = request.team_file.as_deref() {
        if path.len() > MAX_PATH_BYTES {
            bail!("team_file path exceeds {MAX_PATH_BYTES} bytes");
        }
        check_absolute_entry("team_file", path)?;
    }
    Ok(())
}

fn check_request_limits(request: &ConfineRequest) -> Result<()> {
    check_counts(request)?;
    check_keep_env(request)?;
    check_entry_syntaxes(request)?;
    check_team_file_path(request)
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

    fn add_validated_write(&mut self, path: &Path, explicit: bool) -> Result<Option<PathBuf>> {
        let Some(path) = add_canonical_existing(path, "write", &mut self.skipped) else {
            return Ok(None);
        };
        let path = check_write_allowed(&path, &self.host.home, self.host.layout.root(), explicit)?;
        check_write_runtime_allowed(&path, &self.host.runtime_dir, explicit)?;
        add_unique(&mut self.writable, path.clone());
        Ok(Some(path))
    }

    fn operator_names_root(&self, request: &ConfineRequest, root: &Path) -> bool {
        request
            .hide
            .iter()
            .chain(&request.unhide)
            .map(|path| user_path(path, &self.home))
            .filter_map(|path| canonical_if_exists(&path))
            .any(|path| path.as_path() == root)
    }

    fn effective_grants(&mut self, request: &ConfineRequest) -> Vec<String> {
        let Some(dir) = (match self.harness {
            Harness::Claude => Some(".claude"),
            Harness::Omp => Some(".omp"),
            Harness::Codex => Some(".codex"),
            Harness::Generic | Harness::Agy => None,
        }) else {
            return request.grants.clone();
        };
        let mut effective_grants = request.grants.clone();
        if !request.grants.iter().any(|grant| grant == "harness-config") {
            let root = canonical_if_exists(&self.home.join(dir));
            if let Some(root) = root.filter(|root| self.operator_names_root(request, root)) {
                self.unresolved.push(Unresolved {
                    option: "harness-config",
                    path: root,
                    reason: "operator hide or unhide of the harness root".into(),
                });
            } else {
                effective_grants.push("harness-config".into());
            }
        }
        effective_grants
    }

    fn bind_workspace(&mut self, cwd: &Path) -> Result<()> {
        if let Some(path) = self.add_validated_write(cwd, false)? {
            self.implicit_writes.push((path, "working directory"));
        }
        if let Some((common, worktree)) = git_common_dir(cwd, &self.home) {
            if let Some(path) = self.add_validated_write(&common, false)? {
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

    fn under_writable(&self, path: &Path) -> bool {
        self.writable.iter().any(|write| is_within(path, write))
    }

    fn protect_team_file(&mut self, request: &ConfineRequest) {
        let Some(team_file) = &request.team_file else {
            return;
        };
        let Some(path) = add_canonical_existing(
            &user_path(team_file, &self.home),
            "team file",
            &mut self.skipped,
        ) else {
            return;
        };
        if self.under_writable(&path) {
            add_unique(&mut self.readonly, path);
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
                && let Some(path) = self.add_validated_write(&source, true)?
            {
                self.explicit_writes.insert(path);
            }
        }
        self.protect_team_file(request);
        self.protect_system_paths(path_var);
        Ok(())
    }

    fn protect_root(&mut self) {
        if let Some(path) = canonical_if_exists(self.layout.root())
            && self.under_writable(&path)
        {
            add_unique(&mut self.readonly, path);
        }
    }

    fn protect_home_a2amx(&mut self) {
        let Some(path) = canonical_if_exists(&self.home.join(".a2amx")) else {
            return;
        };
        if is_within(&path, &self.home) && self.under_writable(&path) {
            add_unique(&mut self.readonly, path);
        }
    }

    fn protect_path_dirs(&mut self, path_var: &OsStr) {
        for source in std::env::split_paths(path_var) {
            if !source.is_absolute() || !source.is_dir() {
                continue;
            }
            let Some(path) = canonical_if_exists(&source) else {
                continue;
            };
            if is_within(&path, &self.home) && self.under_writable(&path) {
                add_unique(&mut self.readonly, path);
            }
        }
    }

    fn protect_system_paths(&mut self, path_var: Option<&OsStr>) {
        self.protect_root();
        self.protect_home_a2amx();
        if let Some(path_var) = path_var {
            self.protect_path_dirs(path_var);
        }
    }

    fn hidden_offences<'s>(
        &'s self,
        path: &'s Path,
        source: &'s str,
    ) -> impl Iterator<Item = String> + 's {
        self.default_hidden
            .iter()
            .chain(&self.mandatory_hidden)
            .chain(&self.container_hidden)
            .filter(move |hidden| is_within(path, hidden))
            .map(move |hidden| {
                format!(
                    "{source} {} resolves to the hidden path {}; open it with write = [\"{}\"] if that is intended",
                    path.display(),
                    hidden.display(),
                    path.display()
                )
            })
    }

    fn implicit_write_offenders(&self) -> Vec<String> {
        self.implicit_writes
            .iter()
            .filter(|(path, _)| !self.explicit_writes.contains(path))
            .flat_map(|(path, source)| self.hidden_offences(path, source))
            .collect()
    }

    fn check_implicit_writes_not_hidden(&self) -> Result<()> {
        let offenders = self.implicit_write_offenders();
        if offenders.is_empty() {
            Ok(())
        } else {
            bail!("{}", offenders.join("; "));
        }
    }

    fn check_harness_config_conflicts(&self) -> Result<()> {
        let Some(root) = self.harness_config.as_ref() else {
            return Ok(());
        };
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
        Ok(())
    }

    fn check_grant_unhide_conflicts(&self) -> Result<()> {
        for (path, grant) in &self.grant_unhides {
            if self.hidden.iter().any(|existing| existing == path) {
                bail!(
                    "confinement grant \"{grant}\" conflicts with the hide of {}",
                    path.display()
                );
            }
        }
        Ok(())
    }

    fn check_grant_conflicts(&self) -> Result<()> {
        self.check_harness_config_conflicts()?;
        self.check_grant_unhide_conflicts()
    }

    fn overlay_data(&self, protected: &[PathBuf]) -> OverlayData {
        let (data_dirs, data_files) = harness_data(self.harness, &self.home);
        let overlay_parents = harness_overlay_parents(
            &self.home,
            &data_files,
            &self.writable,
            &self.hidden,
            &self.readonly,
            protected,
        );
        let overlay_protected = overlay_protected_entries(
            &overlay_parents,
            &data_dirs,
            &data_files,
            &self.writable,
            &self.readonly,
            protected,
            &self.hidden,
        );
        let unsaved_files = unsaved_harness_files(&data_files, &overlay_parents);
        OverlayData {
            overlay_parents,
            overlay_protected,
            unsaved_files,
        }
    }

    fn sort_parts(&mut self) {
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
    }

    fn finish(
        mut self,
        request: &ConfineRequest,
        cwd: PathBuf,
        mut effective_grants: Vec<String>,
    ) -> Result<Plan> {
        self.readonly
            .retain(|path| !self.explicit_writes.contains(path));
        self.check_grant_conflicts()?;
        self.check_implicit_writes_not_hidden()?;
        let protected = protected_paths(&self.readonly, &self.writable);
        let overlay = self.overlay_data(&protected);
        self.sort_parts();

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
            overlay_parents: overlay.overlay_parents,
            overlay_protected: overlay.overlay_protected,
            unsaved_files: overlay.unsaved_files,
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

struct OverlayData {
    overlay_parents: Vec<PathBuf>,
    overlay_protected: Vec<PathBuf>,
    unsaved_files: Vec<PathBuf>,
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
