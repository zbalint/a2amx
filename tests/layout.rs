#![allow(clippy::expect_used, clippy::unwrap_used)]

use a2amx::layout::Layout;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use tempfile::tempdir;

#[test]
fn resolve_precedence_and_errors() {
    let explicit = Layout::resolve(Some("/explicit".into()), Some("/home".into())).unwrap();
    assert_eq!(explicit.root(), Path::new("/explicit"));
    let home = Layout::resolve(None, Some("/home".into())).unwrap();
    assert_eq!(home.root(), Path::new("/home/.a2amx"));
    assert_eq!(
        Layout::resolve(None, None).unwrap_err().to_string(),
        "cannot determine the a2amx directory: set HOME or A2AMX_HOME"
    );
    assert_eq!(
        Layout::resolve(Some("".into()), None)
            .unwrap_err()
            .to_string(),
        "the a2amx directory must not be empty"
    );
}

#[test]
fn accessors_match_root_table() {
    let layout = Layout::new("/r".into());
    let checks = [
        (layout.config_dir(), "/r/config"),
        (layout.secrets_dir(), "/r/config/secrets"),
        (layout.state_dir(), "/r/state"),
        (layout.gateway_state_dir(), "/r/state/gateway"),
        (layout.run_dir(), "/r/run"),
        (layout.link_dir(), "/r/run/link"),
        (layout.codex_dir(), "/r/run/codex"),
        (layout.codex_session_dir("s1"), "/r/run/codex/s1"),
        (layout.omp_dir(), "/r/run/omp"),
        (layout.log_dir(), "/r/log"),
        (layout.addr(), "/r/run/addr"),
        (layout.admin_token(), "/r/run/admin.token"),
        (layout.gateway_token(), "/r/run/gateway.token"),
        (layout.daemon_lock(), "/r/run/daemon.lock"),
        (layout.gateway_lock(), "/r/run/gateway.lock"),
        (layout.gateway_pid(), "/r/run/gateway.pid"),
        (layout.link_addr(), "/r/run/link/addr"),
        (layout.link_gateway_token(), "/r/run/link/gateway.token"),
        (layout.confine_empty(), "/r/run/confine-empty"),
        (layout.messages_db(), "/r/state/messages.db"),
        (layout.auto_grants(), "/r/state/auto-grants.toml"),
        (layout.pairing(), "/r/state/gateway/pairing.json"),
        (layout.seen(), "/r/state/gateway/seen.json"),
        (layout.team_file(), "/r/config/team.toml"),
        (layout.gateway_config(), "/r/config/gateway.toml"),
        (layout.daemon_config(), "/r/config/daemon.toml"),
        (layout.client_config(), "/r/config/client.toml"),
        (layout.discord_token(), "/r/config/secrets/discord.token"),
        (layout.daemon_log(), "/r/log/daemon.log"),
        (layout.gateway_log(), "/r/log/gateway.log"),
    ];
    assert_eq!(layout.root(), Path::new("/r"));
    for (actual, expected) in checks {
        assert_eq!(actual, Path::new(expected));
    }
}

#[test]
fn ensure_creates_private_dirs_without_bin_and_preserves_existing_root_mode() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
    let layout = Layout::new(root.clone());
    layout.ensure().unwrap();
    assert_eq!(
        std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
        0o755
    );
    for path in [
        layout.config_dir(),
        layout.secrets_dir(),
        layout.state_dir(),
        layout.gateway_state_dir(),
        layout.run_dir(),
        layout.link_dir(),
        layout.log_dir(),
    ] {
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    assert!(!root.join("bin").exists());
    layout.ensure().unwrap();
    assert_eq!(
        std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
        0o755
    );
}
