use std::borrow::Cow;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::grant;
use crate::harness::Harness;

use super::{CLOSING_NOTICE, CREDENTIAL_ENV, ENV_PATH, Plan, granted_env};

const NOTICE_LIMIT: usize = 12 * 1024;
fn capped_paths(paths: &[PathBuf]) -> String {
    let shown = paths.len().min(20);
    let mut output = paths[..shown]
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    if paths.len() > shown {
        if !output.is_empty() {
            output.push_str(", ");
        }
        output.push_str(&format!("and {} more", paths.len() - shown));
    }
    output
}

fn truncate_before_closing(mut text: String) -> String {
    let suffix_len = CLOSING_NOTICE.len() + 2;
    if text.len() + suffix_len > NOTICE_LIMIT {
        let mut keep = NOTICE_LIMIT.saturating_sub(suffix_len);
        while !text.is_char_boundary(keep) {
            keep -= 1;
        }
        text.truncate(keep);
    }
    text.push_str("\n\n");
    text.push_str(CLOSING_NOTICE);
    text
}

/// Render the confinement lines in a team's dry-run report.
pub fn dry_run_lines(plan: &Plan, resolved_cwd: &Path, accepted: &BTreeSet<String>) -> String {
    let mode = if plan.auto { "auto" } else { "manual" };
    let mut output = format!("  confinement: {mode}\n");
    if plan.auto {
        for reason in &plan.reasons {
            output.push_str(&format!("  {}: {}\n", reason.grant, reason.reason));
        }
        let not_accepted = plan
            .reasons
            .iter()
            .filter(|reason| !accepted.contains(&reason.grant))
            .map(|reason| reason.grant.as_str())
            .collect::<Vec<_>>();
        if !not_accepted.is_empty() {
            output.push_str(&format!(
                "  not yet accepted: {}\n",
                not_accepted.join(", ")
            ));
        }
    }
    if !plan.grants.is_empty() {
        output.push_str(&format!("  grants: {}\n", plan.grants.join(", ")));
    }
    if !plan.keep_env.is_empty() {
        output.push_str(&format!("  keep_env: {}\n", plan.keep_env.join(", ")));
    }
    for path in plan.writable.iter().filter(|path| **path != resolved_cwd) {
        output.push_str(&format!("  writable: {}\n", path.display()));
    }
    for path in &plan.protected {
        output.push_str(&format!("  protected: {}\n", path.display()));
    }
    for path in plan.hidden_paths() {
        output.push_str(&format!("  hidden: {}\n", path.display()));
    }
    for path in plan.overlay_parents() {
        output.push_str(&format!("  overlay: {}\n", path.display()));
    }
    for path in plan.unsaved_files() {
        output.push_str(&format!("  unsaved: {}\n", path.display()));
    }
    for unresolved in &plan.unresolved {
        output.push_str(&format!(
            "  unresolved: {} ({}: {})\n",
            unresolved.path.display(),
            unresolved.option,
            unresolved.reason
        ));
    }
    for path in plan
        .skipped_paths()
        .iter()
        .filter(|path| !plan.unresolved.iter().any(|item| item.path == **path))
    {
        output.push_str(&format!("  hidden: {} (skipped missing)\n", path.display()));
    }
    output
}

/// Render the operator-facing notice from the exact mount plan.
pub fn notice(plan: &Plan) -> String {
    let writable: Cow<'_, [PathBuf]> = match plan.writable.iter().position(|path| path == &plan.cwd)
    {
        Some(0) | None => Cow::Borrowed(&plan.writable),
        Some(index) => {
            let mut reordered = Vec::with_capacity(plan.writable.len());
            reordered.push(plan.cwd.clone());
            reordered.extend(
                plan.writable
                    .iter()
                    .enumerate()
                    .filter(|(position, _)| *position != index)
                    .map(|(_, path)| path.clone()),
            );
            Cow::Owned(reordered)
        }
    };
    let mut text = String::new();
    text.push_str("This session is confined.\n");
    text.push_str("Writable locations: ");
    text.push_str(&capped_paths(&writable));
    text.push_str(".\n");
    text.push_str("Read-only protected paths: ");
    text.push_str(&capped_paths(&plan.protected));
    text.push_str(".\n");
    text.push_str("Hidden locations (a hidden directory looks empty; a hidden file looks empty): ");
    text.push_str(&capped_paths(&plan.hidden));
    text.push_str(".\n");
    if !plan.skipped.is_empty() {
        text.push_str("Skipped missing paths: ");
        text.push_str(&capped_paths(&plan.skipped));
        text.push_str(".\n");
    }
    for path in &plan.overlay_parents {
        text.push_str(&format!(
            "{}: writable, but only the harness's data files are saved; other writes are discarded at exit.\n",
            path.display()
        ));
    }
    if !plan.unsaved_files.is_empty() {
        text.push_str("Files not present at start, writes to them are not saved: ");
        text.push_str(&capped_paths(&plan.unsaved_files));
        text.push_str(".\n");
    }
    if let Some(reason) = &plan.overlay_notice {
        text.push_str(reason);
        text.push('\n');
    }
    if let Some(path) = &plan.harness_config {
        let source = if plan.grants.iter().any(|grant| grant == "harness-config") {
            "grant harness-config"
        } else {
            "default for this harness"
        };
        text.push_str(&format!(
            "{}: harness config, writable and persistent ({}).\n",
            path.display(),
            source
        ));
    }
    if !plan.grants.is_empty() {
        text.push_str("Granted: ");
        text.push_str(&plan.grants.join(", "));
        text.push_str(".\n");
    }
    let released = granted_env(&plan.grants);
    let unset_credentials = CREDENTIAL_ENV
        .iter()
        .copied()
        .filter(|name| !released.contains(*name) && !plan.keep_env.iter().any(|kept| kept == *name))
        .collect::<Vec<_>>();
    text.push_str("Credential environment variables are unset: ");
    text.push_str(&unset_credentials.join(", "));
    if !plan.keep_env.is_empty() {
        text.push_str("; kept: ");
        text.push_str(&plan.keep_env.join(", "));
    }
    text.push_str(".\n");
    text.push_str(
        "Everything else is read-only. /tmp is private to this session and discarded when it ends; paths under a peer's /tmp cannot be read.\n",
    );
    let granted = plan
        .grants
        .iter()
        .filter_map(|name| grant::all().iter().find(|grant| grant.name == name))
        .filter(|grant| !grant.sockets.is_empty())
        .map(|grant| grant.name)
        .collect::<Vec<_>>();
    if granted.is_empty() {
        text.push_str("Container engines are restricted.\n");
    } else {
        text.push_str(&format!(
            "Container engines allowed: {}. Other container engines are restricted.\n",
            granted.join(", ")
        ));
    }
    text.push_str(
        "A Read-only file system error is authoritative where this notice and the system disagree.",
    );
    truncate_before_closing(text)
}

fn ctrl_c_wrapper(wrapped_inner: Vec<String>) -> Vec<String> {
    let mut wrapped = Vec::with_capacity(wrapped_inner.len() + 2);
    wrapped.push(ENV_PATH.to_owned());
    wrapped.push("--ignore-signal=INT".into());
    // shortcut: the fixed outer prefix is 33 bytes with NULs, far below ARG_MAX; no second size check.
    wrapped.extend(wrapped_inner);
    wrapped
}

/// Build a wrapped command and, where the harness has a system prompt channel,
/// append the notice.  Generic sessions intentionally receive no notice.
pub fn launch_argv(
    plan: &Plan,
    notice: &str,
    argv: Vec<String>,
    harness: Harness,
) -> Result<Vec<String>> {
    match harness {
        Harness::Generic => {
            let mut inner = Vec::with_capacity(argv.len() + 2);
            inner.push(ENV_PATH.to_owned());
            inner.push("--default-signal=INT".into());
            inner.extend(argv);
            Ok(ctrl_c_wrapper(plan.wrap(inner)?))
        }
        Harness::Claude | Harness::Omp => {
            let wrapped = plan.wrap(argv)?;
            crate::harness::append_system_prompt(wrapped, notice, harness, &plan.cwd)
        }
        Harness::Codex | Harness::Agy => plan.wrap(argv),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::GrantReason;

    #[test]
    fn closing_notice_is_always_present() {
        let plan = Plan {
            bwrap: PathBuf::from("bwrap"),
            layout: crate::layout::Layout::new(PathBuf::from("/state")),
            cwd: PathBuf::from("/workspace"),
            harness: Harness::Generic,
            writable: (0..100).map(|n| PathBuf::from(format!("/w/{n}"))).collect(),
            readonly: Vec::new(),
            protected: Vec::new(),
            hidden: Vec::new(),
            skipped: Vec::new(),
            unresolved: Vec::new(),
            grants: Vec::new(),
            effective_grants: Vec::new(),
            auto: false,
            reasons: Vec::new(),
            write: Vec::new(),
            hide: Vec::new(),
            unhide: Vec::new(),
            no_default_hides: false,
            keep_env: Vec::new(),
            mandatory_hidden: Vec::new(),
            default_hidden: Vec::new(),
            container_hidden: Vec::new(),
            overlay_parents: Vec::new(),
            overlay_protected: Vec::new(),
            unsaved_files: Vec::new(),
            overlay_notice: None,
            harness_config: None,
            codex_dir: None,
        };
        let text = notice(&plan);
        assert!(text.len() <= NOTICE_LIMIT);
        assert_eq!(text.matches("/w/0").count(), 1);
        assert!(text.ends_with(CLOSING_NOTICE));
    }
    fn dry_run_plan() -> Plan {
        Plan {
            bwrap: PathBuf::from("bwrap"),
            layout: crate::layout::Layout::new(PathBuf::from("/state")),
            cwd: PathBuf::from("/plan-cwd"),
            harness: Harness::Generic,
            writable: vec![PathBuf::from("/resolved-cwd"), PathBuf::from("/other")],
            readonly: Vec::new(),
            protected: Vec::new(),
            hidden: Vec::new(),
            skipped: vec![PathBuf::from("/missing")],
            unresolved: Vec::new(),
            grants: vec!["cache".to_owned()],
            effective_grants: Vec::new(),
            auto: true,
            reasons: vec![GrantReason {
                grant: "cache".to_owned(),
                reason: "detected cache".to_owned(),
            }],
            write: Vec::new(),
            hide: Vec::new(),
            unhide: Vec::new(),
            no_default_hides: false,
            keep_env: Vec::new(),
            mandatory_hidden: Vec::new(),
            default_hidden: Vec::new(),
            container_hidden: Vec::new(),
            overlay_parents: Vec::new(),
            overlay_protected: Vec::new(),
            unsaved_files: Vec::new(),
            overlay_notice: None,
            harness_config: None,
            codex_dir: None,
        }
    }

    #[test]
    fn dry_run_lines_list_auto_grants_not_yet_accepted() {
        let text = dry_run_lines(
            &dry_run_plan(),
            Path::new("/resolved-cwd"),
            &BTreeSet::new(),
        );

        assert!(text.contains("  confinement: auto\n"));
        assert!(text.contains("  cache: detected cache\n"));
        assert!(text.contains("  not yet accepted: cache\n"));
        assert!(text.contains("  hidden: /missing (skipped missing)\n"));
    }

    #[test]
    fn dry_run_lines_uses_resolved_cwd_when_filtering_writable_paths() {
        let text = dry_run_lines(
            &dry_run_plan(),
            Path::new("/resolved-cwd"),
            &BTreeSet::new(),
        );

        assert!(!text.contains("  writable: /resolved-cwd\n"));
        assert!(text.contains("  writable: /other\n"));
    }

    #[test]
    fn dry_run_lines_only_lists_unaccepted_reason_for_mixed_set() {
        let mut plan = dry_run_plan();
        plan.reasons.push(GrantReason {
            grant: "cargo".to_owned(),
            reason: "detected Cargo files".to_owned(),
        });
        let accepted = BTreeSet::from(["cache".to_owned()]);

        let text = dry_run_lines(&plan, Path::new("/resolved-cwd"), &accepted);

        assert!(text.contains("  not yet accepted: cargo\n"));
        assert!(!text.contains("  not yet accepted: cache"));
    }
}
