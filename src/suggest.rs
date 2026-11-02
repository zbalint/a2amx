//! Pure project and git detection for confinement grants.

use crate::grant;

/// Grants whose marker exists, in grant-table order; only grants a team table may name.
pub fn detect(has_file: impl Fn(&str) -> bool) -> Vec<&'static str> {
    grant::all()
        .iter()
        .filter(|grant| {
            grant.team_default_ok
                && !grant.detect.is_empty()
                && grant.detect.iter().any(|marker| has_file(marker))
        })
        .map(|grant| grant.name)
        .collect()
}

/// Grants requested by the remotes in a git config, in grant-table order.
pub fn git_remote_grants(config: &str, has_host_path: impl Fn(&str) -> bool) -> Vec<&'static str> {
    let mut found = Vec::new();
    let mut in_remote = false;
    for raw_line in config.lines() {
        let line = raw_line.trim();
        if line.starts_with('[') {
            in_remote = line.starts_with("[remote ");
            continue;
        }
        if !in_remote {
            continue;
        }
        let Some((key, raw_value)) = line.split_once('=') else {
            continue;
        };
        if !(key.trim().eq_ignore_ascii_case("url") || key.trim().eq_ignore_ascii_case("pushurl")) {
            continue;
        }
        let value = raw_value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .unwrap_or(value);
        if value.starts_with("ssh://")
            || value.starts_with("git+ssh://")
            || (!value.contains("://") && value.contains(':'))
        {
            found.push("ssh");
        } else if value.starts_with("https://") {
            if has_host_path("~/.config/gh") {
                found.push("gh");
            }
            if has_host_path("~/.git-credentials") || has_host_path("~/.config/git/credentials") {
                found.push("git-credentials");
            }
        }
    }
    table_order(found)
}

/// What auto adds and `confine suggest` prints: cache plus detected grants.
pub fn grants(
    has_file: impl Fn(&str) -> bool,
    git_config: Option<&str>,
    has_host_path: impl Fn(&str) -> bool,
) -> Vec<&'static str> {
    let mut found = detect(has_file);
    if let Some(config) = git_config {
        found.extend(git_remote_grants(config, has_host_path));
    }
    found.push("cache");
    table_order(found)
}

/// Render one copyable grants line.
pub fn render(found: &[&'static str]) -> String {
    let names = found
        .iter()
        .map(|name| format!("\"{name}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!("# Suggested grants:\ngrants = [{names}]\n")
}

fn table_order(found: impl IntoIterator<Item = &'static str>) -> Vec<&'static str> {
    let found = found.into_iter().collect::<std::collections::HashSet<_>>();
    grant::all()
        .iter()
        .filter(|grant| found.contains(grant.name))
        .map(|grant| grant.name)
        .collect()
}
