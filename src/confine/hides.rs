use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use crate::grant;
use crate::harness::Harness;

use super::{
    ConfineRequest, MANDATORY_ROOTS, PlanBuilder, add_unique, canonical_existing,
    canonical_if_exists, check_unhide_allowed, check_write_allowed, check_write_runtime_allowed,
    expand_home, is_within, paths_overlap, symlink_component, user_path,
};

const DEFAULT_HIDES: [&str; 24] = [
    "~/.ssh",
    "~/.gnupg",
    "~/.aws",
    "~/.config/gcloud",
    "~/.kube",
    "~/.docker",
    "~/.netrc",
    "~/.local/share/keyrings",
    "~/.config/chromium",
    "~/.config/google-chrome",
    "~/.mozilla/firefox",
    "~/.config/gh",
    "~/.git-credentials",
    "~/.config/git/credentials",
    "~/.npmrc",
    "~/.pypirc",
    "~/.cargo/credentials",
    "~/.cargo/credentials.toml",
    "~/.azure",
    "~/.terraform.d",
    "~/.vault-token",
    "~/.pgpass",
    "~/.password-store",
    "~/.config/op",
];
const DEFAULT_HIDES_EXTRA: [&str; 4] = [
    "~/.cache/huggingface/token",
    "~/.config/rclone",
    "~/.s3cfg",
    "~/.claude/ide",
];
struct SocketRow<'a> {
    name: &'static str,
    sockets: &'a [&'a str],
}

fn granted_socket_targets(
    table: &[SocketRow<'_>],
    given: &BTreeSet<&str>,
    home: &Path,
) -> BTreeSet<PathBuf> {
    let mut granted_targets = BTreeSet::new();
    for row in table.iter().filter(|row| given.contains(row.name)) {
        for socket in row.sockets {
            let source = expand_home(socket, home);
            if let Some(path) = canonical_if_exists(&source) {
                granted_targets.insert(path);
            }
        }
    }
    granted_targets
}

fn container_socket_hides(
    table: &[SocketRow<'_>],
    given: &[String],
    home: &Path,
) -> (Vec<(PathBuf, &'static str)>, Vec<PathBuf>) {
    let given = given.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let granted_targets = granted_socket_targets(table, &given, home);

    let mut hidden = Vec::new();
    let mut skipped = Vec::new();
    for row in table.iter().filter(|row| !given.contains(row.name)) {
        for socket in row.sockets {
            let source = expand_home(socket, home);
            let Some(path) = canonical_if_exists(&source) else {
                add_unique(&mut skipped, source);
                continue;
            };
            if granted_targets.contains(&path) {
                continue;
            }
            if !hidden.iter().any(|(existing, _)| existing == &path) {
                hidden.push((path, row.name));
            }
        }
    }
    (hidden, skipped)
}

pub fn default_hide_names(harness: Harness) -> Vec<String> {
    let mut names = DEFAULT_HIDES
        .into_iter()
        .chain(DEFAULT_HIDES_EXTRA)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if harness != Harness::Claude {
        names.push("~/.claude/.credentials.json".to_owned());
    }
    if harness != Harness::Codex {
        names.push("~/.codex/auth.json".to_owned());
    }
    if harness != Harness::Agy {
        names.push("~/.gemini/antigravity-cli/antigravity-oauth-token".to_owned());
        names.push("~/.config/antigravity".to_owned());
    }
    names.push("/run/dbus".to_owned());
    names
}

pub(super) fn default_hide_sources_for(home: &Path, harness: Harness) -> Vec<PathBuf> {
    default_hide_names(harness)
        .into_iter()
        .map(|name| user_path(&name, home))
        .collect()
}
impl<'a> PlanBuilder<'a> {
    pub(super) fn hide_mandatory(&mut self) -> Result<()> {
        for path in [
            canonical_existing(&self.runtime_dir, "runtime directory")?,
            canonical_existing(&self.layout.state_dir(), "state directory")?,
            canonical_existing(&self.layout.run_dir(), "run directory")?,
            canonical_existing(&self.layout.secrets_dir(), "secrets directory")?,
        ] {
            add_unique(&mut self.mandatory_hidden, path);
        }
        for path in MANDATORY_ROOTS {
            if let Some(path) = canonical_if_exists(Path::new(path)) {
                add_unique(&mut self.mandatory_hidden, path);
            } else {
                add_unique(&mut self.skipped, PathBuf::from(path));
            }
        }
        for path in &self.mandatory_hidden {
            add_unique(&mut self.hidden, path.clone());
        }
        Ok(())
    }

    fn default_hide_sources(&self) -> Vec<PathBuf> {
        default_hide_sources_for(&self.home, self.harness)
    }

    pub(super) fn hide_defaults(&mut self, request: &ConfineRequest) {
        if request.no_default_hides {
            return;
        }
        for source in self.default_hide_sources() {
            if source == Path::new("/run/dbus") {
                if let Some(path) = canonical_if_exists(&source) {
                    add_unique(&mut self.default_hidden, path.clone());
                    add_unique(&mut self.hidden, path);
                }
            } else {
                self.hide_or_skip(source);
            }
        }
    }

    fn guarded_harness_root(&self) -> Option<PathBuf> {
        if self.harness_config.is_some() {
            return None;
        }
        self.session_harness_root()
            .and_then(|root| canonical_if_exists(&root))
    }

    fn check_grant_path_writable(&self, grant: &str, entry: &str, path: &Path) -> Result<PathBuf> {
        let path = check_write_allowed(path, &self.home, self.layout.root(), false)?;
        check_write_runtime_allowed(&path, &self.runtime_dir, false)?;
        if let Some(root) = canonical_if_exists(&self.home.join(".a2amx"))
            && is_within(&path, &root)
        {
            bail!("confinement grant \"{grant}\" path {entry} is inside ~/.a2amx");
        }
        if let Some(root) = self.guarded_harness_root()
            && is_within(&path, &root)
        {
            bail!(
                "confinement grant \"{grant}\" path {entry} is inside the harness root {}",
                root.display()
            );
        }
        Ok(path)
    }

    fn resolve_grant_unhide(&mut self, grant_name: &str, name: &str) -> Result<Option<PathBuf>> {
        let source = user_path(name, &self.home);
        if let Some(path) = symlink_component(&source) {
            bail!(
                "confinement grant \"{}\" path {} passes through the symlink {}; name the real path with a write entry instead",
                grant_name,
                name,
                path.display()
            );
        }
        let Some(path) = canonical_if_exists(&source) else {
            add_unique(&mut self.skipped, source);
            return Ok(None);
        };
        Ok(Some(
            self.check_grant_path_writable(grant_name, name, &path)?,
        ))
    }

    fn unhide_conflict(
        &self,
        path: &Path,
        default_hide_sources: &[PathBuf],
        own_paths: &BTreeSet<PathBuf>,
    ) -> Option<PathBuf> {
        default_hide_sources
            .iter()
            .filter(|source| !own_paths.contains(*source))
            .filter_map(|source| canonical_if_exists(source))
            .find(|hidden| is_within(path, hidden))
            .or_else(|| {
                self.mandatory_hidden
                    .iter()
                    .chain(&self.container_hidden)
                    .find(|hidden| paths_overlap(path, hidden))
                    .cloned()
            })
    }

    pub(super) fn apply_grant_unhides(
        &mut self,
        given: &[String],
        no_default_hides: bool,
    ) -> Result<()> {
        let given = given.iter().map(String::as_str).collect::<BTreeSet<_>>();
        let own_paths = grant::all()
            .iter()
            .filter(|grant| given.contains(grant.name))
            .flat_map(|grant| grant.unhides)
            .map(|path| user_path(path, &self.home))
            .collect::<BTreeSet<_>>();
        let default_hide_sources = if no_default_hides {
            Vec::new()
        } else {
            self.default_hide_sources()
        };
        for grant in grant::all()
            .iter()
            .filter(|grant| given.contains(grant.name))
        {
            for name in grant.unhides {
                let Some(path) = self.resolve_grant_unhide(grant.name, name)? else {
                    continue;
                };
                if let Some(hidden) = self.unhide_conflict(&path, &default_hide_sources, &own_paths)
                {
                    bail!(
                        "confinement grant \"{}\" path {} resolves to the hidden path {}",
                        grant.name,
                        name,
                        hidden.display()
                    );
                }
                self.hidden.retain(|existing| existing != &path);
                self.default_hidden.retain(|existing| existing != &path);
                add_unique(&mut self.writable, path.clone());
                self.grant_unhides.push((path, grant.name));
            }
        }
        Ok(())
    }

    pub(super) fn apply_grant_stand_ins(&mut self, given: &[String]) {
        let given = given.iter().map(String::as_str).collect::<BTreeSet<_>>();
        for grant in grant::all()
            .iter()
            .filter(|grant| given.contains(grant.name))
        {
            for name in grant.stand_ins {
                let source = PathBuf::from(name);
                let Some(path) = canonical_if_exists(&source) else {
                    add_unique(&mut self.skipped, source);
                    continue;
                };
                add_unique(&mut self.hidden, path);
            }
        }
    }

    pub(super) fn hide_containers(&mut self, given: &[String]) {
        let table = grant::all()
            .iter()
            .map(|grant| SocketRow {
                name: grant.name,
                sockets: grant.sockets,
            })
            .collect::<Vec<_>>();
        let (hidden, skipped) = container_socket_hides(&table, given, &self.home);
        for (path, owner) in hidden {
            add_unique(&mut self.container_hidden, path.clone());
            add_unique(&mut self.hidden, path.clone());
            self.container_sockets.push((path, owner));
        }
        for path in skipped {
            add_unique(&mut self.skipped, path);
        }
    }

    fn hide_or_skip(&mut self, source: PathBuf) {
        if let Some(path) = canonical_if_exists(&source) {
            add_unique(&mut self.default_hidden, path.clone());
            add_unique(&mut self.hidden, path);
        } else {
            add_unique(&mut self.skipped, source);
        }
    }

    fn check_container_socket(&self, path: &Path, option: &str) -> Result<()> {
        if let Some((_, grant)) = self
            .container_sockets
            .iter()
            .find(|(socket, _)| is_within(path, socket))
        {
            bail!(
                "{option} path {} requires the {grant} grant (--confine-grant {grant}, or grants = [\"{grant}\"])",
                path.display()
            );
        }
        Ok(())
    }

    pub(super) fn release_operator_writes(&mut self) -> Result<()> {
        for path in &self.explicit_writes {
            self.check_container_socket(path, "write")?;
            self.hidden.retain(|existing| existing != path);
            self.default_hidden.retain(|existing| existing != path);
        }
        Ok(())
    }

    pub(super) fn apply_operator_hides(&mut self, request: &ConfineRequest) -> Result<()> {
        for path in &request.hide {
            let source = user_path(path, &self.home);
            if let Some(path) = self.resolve_operator_path("hide", &source) {
                add_unique(&mut self.hidden, path);
            }
        }
        for path in &request.unhide {
            self.unhide(path)?;
        }
        Ok(())
    }

    fn unhide(&mut self, path: &str) -> Result<()> {
        let source = user_path(path, &self.home);
        if let Some(path) = self.resolve_operator_path("unhide", &source) {
            check_unhide_allowed(&path, self.layout.root(), &self.runtime_dir, &self.home)?;
            self.check_container_socket(&path, "unhide")?;
            self.hidden.retain(|existing| existing != &path);
            self.default_hidden.retain(|existing| existing != &path);
            self.container_hidden.retain(|existing| existing != &path);
            if !self
                .grant_unhides
                .iter()
                .any(|(granted, _)| granted == &path)
            {
                add_unique(&mut self.readonly, path);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn granted_socket_target_wins_over_an_ungranted_alias() {
        let root = tempfile::tempdir().expect("socket fixture directory");
        let granted = root.path().join("granted.sock");
        let alias = root.path().join("alias.sock");
        std::fs::write(&granted, "socket").expect("granted socket");
        std::os::unix::fs::symlink(&granted, &alias).expect("alias socket");
        let granted_path = granted.to_string_lossy().into_owned();
        let alias_path = alias.to_string_lossy().into_owned();
        let granted_sockets = [granted_path.as_str()];
        let alias_sockets = [alias_path.as_str()];
        let table = [
            SocketRow {
                name: "granted",
                sockets: &granted_sockets,
            },
            SocketRow {
                name: "alias",
                sockets: &alias_sockets,
            },
        ];
        let given = vec!["granted".to_owned()];
        let (hidden, skipped) = container_socket_hides(&table, &given, root.path());
        assert!(hidden.is_empty());
        assert!(skipped.is_empty());
    }
    #[test]
    fn credential_grant_unhides_are_default_hide_sources() {
        let defaults = DEFAULT_HIDES
            .into_iter()
            .chain(DEFAULT_HIDES_EXTRA)
            .collect::<BTreeSet<_>>();
        let credential_grants = [
            "gh",
            "git-credentials",
            "ssh",
            "aws",
            "gcloud",
            "azure",
            "kube",
            "terraform",
            "npm-token",
            "pypi-token",
            "cargo-token",
            "netrc",
            "huggingface-token",
        ];
        for grant in grant::all()
            .iter()
            .filter(|grant| credential_grants.contains(&grant.name))
        {
            for path in grant.unhides {
                assert!(
                    defaults.contains(path),
                    "{} unhide path {path} is not a default hide",
                    grant.name
                );
            }
        }
    }
}
