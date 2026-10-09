use a2amx::client::version_warning;

#[test]
fn equal_versions_do_not_warn() {
    assert_eq!(version_warning("ab12cd3", Some("ab12cd3")), None);
}

#[test]
fn different_versions_warn_with_restart_guidance() {
    assert_eq!(
        version_warning("ab12cd3", Some("9f8e7d6")),
        Some(
            "warning: daemon version 9f8e7d6 differs from this binary ab12cd3; restart the daemon to use this binary (sessions end)".into(),
        )
    );
}

#[test]
fn missing_daemon_version_warns_as_pre_version_daemon() {
    assert_eq!(
        version_warning("ab12cd3", None),
        Some(
            "warning: daemon predates version reporting; restart it to use this binary (sessions end)".into(),
        )
    );
}
