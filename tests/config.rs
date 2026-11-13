#![allow(clippy::expect_used, clippy::unwrap_used)]

use a2amx::config::{self, Overrides, Source};
use a2amx::layout::Layout;
use a2amx::messaging::Limits;
use std::fs;
use std::net::SocketAddr;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::process::Command;
use std::time::Duration;
use tempfile::tempdir;

fn layout() -> (tempfile::TempDir, Layout) {
    let dir = tempdir().unwrap();
    let layout = Layout::new(dir.path().join("root"));
    layout.ensure().unwrap();
    (dir, layout)
}
fn empty_overrides() -> Overrides {
    Overrides {
        listen: vec![],
        host_name: None,
        owner_away_secs: None,
    }
}
fn source_all(loaded: &config::LoadedDaemon, source: Source) {
    assert!(loaded.sources.iter().all(|(_, actual)| *actual == source));
}

#[test]
fn defaults_without_file() {
    let (_dir, layout) = layout();
    let loaded = config::load_daemon(&layout, &empty_overrides()).unwrap();
    assert_eq!(
        loaded.settings.listen,
        vec!["127.0.0.1:0".parse::<SocketAddr>().unwrap()]
    );
    assert_eq!(loaded.settings.host_name, None);
    assert_eq!(loaded.settings.limits, Limits::default());
    source_all(&loaded, Source::Default);
}

#[test]
fn full_file_overrides_every_key() {
    let (_dir, layout) = layout();
    fs::write(
        layout.daemon_config(),
        r#"listen = ["127.0.0.1:9000", "[::1]:9001"]
host_name = "host-a"
owner_away = "2m"
[limits]
max_pending_per_recipient = 51
rate_per_minute = 21
human_inbound_per_minute = 22
human_outbound_per_minute = 31
max_stored_messages = 10001
retention = "3d"
lease = "2d"
code_ttl = "11m"
"#,
    )
    .unwrap();
    let loaded = config::load_daemon(&layout, &empty_overrides()).unwrap();
    assert_eq!(
        loaded.settings.listen,
        vec![
            "127.0.0.1:9000".parse::<SocketAddr>().unwrap(),
            "[::1]:9001".parse::<SocketAddr>().unwrap()
        ]
    );
    assert_eq!(loaded.settings.host_name.as_deref(), Some("host-a"));
    assert_eq!(loaded.settings.limits.max_pending_per_recipient, 51);
    assert_eq!(loaded.settings.limits.rate_per_minute, 21);
    assert_eq!(loaded.settings.limits.human_inbound_per_minute, 22);
    assert_eq!(loaded.settings.limits.human_outbound_per_minute, 31);
    assert_eq!(loaded.settings.limits.max_stored_messages, 10001);
    assert_eq!(loaded.settings.limits.away_secs, 120);
    assert_eq!(loaded.settings.limits.retention_secs, 3 * 86400);
    assert_eq!(loaded.settings.limits.lease_secs, 2 * 86400);
    assert_eq!(loaded.settings.limits.code_secs, 11 * 60);
    source_all(&loaded, Source::File);
}

#[test]
fn overrides_beat_file() {
    let (_dir, layout) = layout();
    fs::write(
        layout.daemon_config(),
        "listen=[\"127.0.0.1:9\"]\nhost_name=\"file\"\nowner_away=\"2m\"\n",
    )
    .unwrap();
    let overrides = Overrides {
        listen: vec!["127.0.0.1:10".parse().unwrap()],
        host_name: Some("flag".into()),
        owner_away_secs: Some(5),
    };
    let loaded = config::load_daemon(&layout, &overrides).unwrap();
    assert_eq!(loaded.settings.listen, overrides.listen);
    assert_eq!(loaded.settings.host_name.as_deref(), Some("flag"));
    assert_eq!(loaded.settings.limits.away_secs, 5);
    assert_eq!(
        loaded
            .sources
            .iter()
            .find(|(key, _)| *key == "owner_away")
            .unwrap()
            .1,
        Source::Override
    );
}

#[test]
fn template_round_trip_and_mode() {
    let (_dir, layout) = layout();
    config::ensure_daemon_file(&layout).unwrap();
    let loaded = config::load_daemon(&layout, &empty_overrides()).unwrap();
    assert_eq!(loaded.settings.limits, Limits::default());
    assert_eq!(
        fs::metadata(layout.daemon_config())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let mut text = fs::read_to_string(layout.daemon_config()).unwrap();
    text = text
        .lines()
        .map(|line| {
            if line == "# [limits]" {
                "[limits]".to_owned()
            } else if let Some(rest) = line.strip_prefix("# ") {
                let key_line = rest.split_once(" = ").is_some_and(|(key, _)| {
                    key.chars().all(|c| c.is_ascii_lowercase() || c == '_')
                });
                if key_line {
                    rest.to_owned()
                } else {
                    line.to_owned()
                }
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(layout.daemon_config(), text).unwrap();
    let loaded = config::load_daemon(&layout, &empty_overrides()).unwrap();
    assert_eq!(
        loaded.settings.listen,
        vec!["127.0.0.1:0".parse::<SocketAddr>().unwrap()]
    );
    assert_eq!(loaded.settings.host_name.as_deref(), Some("host-a"));
    assert_eq!(loaded.settings.limits, Limits::default());
    source_all(&loaded, Source::File);
}

#[test]
fn existing_file_is_unchanged() {
    let (_dir, layout) = layout();
    for body in ["", "not = valid = toml\n"] {
        fs::write(layout.daemon_config(), body).unwrap();
        config::ensure_daemon_file(&layout).unwrap();
        assert_eq!(fs::read(layout.daemon_config()).unwrap(), body.as_bytes());
        fs::remove_file(layout.daemon_config()).unwrap();
    }
}

#[test]
fn invalid_values_name_key_and_file() {
    let cases = [
        ("rate = 1", "rate"),
        ("limits.rate = 1", "limits.rate"),
        ("limits.unknown = 1", "limits.unknown"),
        ("limits.rate_per_minute = \"x\"", "limits.rate_per_minute"),
        (
            "limits.max_pending_per_recipient = 0",
            "limits.max_pending_per_recipient",
        ),
        ("limits.rate_per_minute = 0", "limits.rate_per_minute"),
        (
            "limits.human_inbound_per_minute = 0",
            "limits.human_inbound_per_minute",
        ),
        (
            "limits.human_outbound_per_minute = 0",
            "limits.human_outbound_per_minute",
        ),
        (
            "limits.max_stored_messages = 0",
            "limits.max_stored_messages",
        ),
        ("owner_away = \"3651d\"", "owner_away"),
        ("limits.retention = \"0s\"", "limits.retention"),
        ("limits.lease = \"0s\"", "limits.lease"),
        ("limits.code_ttl = \"0s\"", "limits.code_ttl"),
        ("limits.retention = \"3651d\"", "limits.retention"),
        ("limits.lease = \"3651d\"", "limits.lease"),
        ("limits.code_ttl = \"3651d\"", "limits.code_ttl"),
        ("limits.retention = \"60\"", "limits.retention"),
        ("owner_away = \"é\"", "owner_away"),
        ("owner_away = \"123é\"", "owner_away"),
        (
            "limits.retention = \"213503982334602d\"",
            "limits.retention",
        ),
        ("listen = []", "listen"),
        ("listen = [\"bad\"]", "listen"),
        ("host_name = \"\"", "host_name"),
    ];
    for (body, key) in cases {
        let (_dir, layout) = layout();
        fs::write(layout.daemon_config(), body).unwrap();
        let error = config::load_daemon(&layout, &empty_overrides())
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("daemon.toml") && error.contains(key),
            "{error}"
        );
    }
    let (_dir, layout) = layout();
    fs::write(layout.daemon_config(), "owner_away = \"0s\"\n").unwrap();
    assert_eq!(
        config::load_daemon(&layout, &empty_overrides())
            .unwrap()
            .settings
            .limits
            .away_secs,
        0
    );
    let overrides = Overrides {
        owner_away_secs: Some(315_360_001),
        ..empty_overrides()
    };
    assert!(
        config::load_daemon(&layout, &overrides)
            .unwrap_err()
            .to_string()
            .contains("owner_away")
    );
}

#[test]
fn non_directory_config_parent_is_an_error() {
    let (_dir, layout) = layout();
    fs::remove_dir_all(layout.config_dir()).unwrap();
    fs::write(layout.config_dir(), "not a directory").unwrap();
    let error = config::load_daemon(&layout, &empty_overrides())
        .unwrap_err()
        .to_string();
    assert!(
        error.contains(&layout.daemon_config().display().to_string()),
        "{error}"
    );
}

#[test]
fn bounded_and_nonregular_files_are_rejected() {
    let (_dir, layout) = layout();
    let mut body = b"#".to_vec();
    body.resize(64 * 1024 + 1, b'a');
    fs::write(layout.daemon_config(), &body).unwrap();
    let oversized = config::load_daemon(&layout, &empty_overrides())
        .unwrap_err()
        .to_string();
    assert!(oversized.contains("exceeds 65536"));
    fs::remove_file(layout.daemon_config()).unwrap();
    fs::write(layout.daemon_config(), [0xff, 0xfe]).unwrap();
    assert!(config::load_daemon(&layout, &empty_overrides()).is_err());
    fs::remove_file(layout.daemon_config()).unwrap();
    fs::create_dir(layout.daemon_config()).unwrap();
    assert!(config::load_daemon(&layout, &empty_overrides()).is_err());
    fs::remove_dir(layout.daemon_config()).unwrap();
    let status = Command::new("mkfifo")
        .arg(layout.daemon_config())
        .status()
        .unwrap();
    assert!(status.success());
    assert!(config::load_daemon(&layout, &empty_overrides()).is_err());
    fs::remove_file(layout.daemon_config()).unwrap();
    symlink(layout.root().join("missing"), layout.daemon_config()).unwrap();
    config::ensure_daemon_file(&layout).unwrap();
    assert!(!layout.root().join("missing").exists());
    assert_eq!(
        config::load_daemon(&layout, &empty_overrides())
            .unwrap()
            .settings
            .limits,
        Limits::default()
    );
}

#[test]
fn duration_parser_and_formatter() {
    for (text, seconds) in [
        ("0s", 0),
        ("1m", 60),
        ("24h", 86400),
        ("7d", 604800),
        ("90s", 90),
        ("3600s", 3600),
        ("60s", 60),
    ] {
        let duration = config::parse_duration(text).unwrap();
        assert_eq!(duration, Duration::from_secs(seconds));
        assert_eq!(
            config::parse_duration(&config::format_duration(duration)).unwrap(),
            duration
        );
    }
    assert_eq!(config::format_duration(Duration::from_secs(0)), "0s");
    assert_eq!(config::format_duration(Duration::from_secs(86400)), "1d");
    assert_eq!(config::format_duration(Duration::from_secs(90)), "90s");
    assert_eq!(config::format_duration(Duration::from_secs(3600)), "1h");
    assert_eq!(config::format_duration(Duration::from_secs(60)), "1m");
    for text in [
        "60",
        "1.5h",
        "1h30m",
        "-1s",
        "",
        "5w",
        "é",
        "123é",
        "99999999999999999999d",
    ] {
        assert!(config::parse_duration(text).is_err(), "{text}");
    }
}

#[test]
fn show_daemon_reports_values_and_sources() {
    let (_dir, layout) = layout();
    fs::write(layout.daemon_config(), "[limits]\nretention = \"3d\"\n").unwrap();
    let expected = format!(
        "daemon.toml: {} (present)\n\
listen = [\"127.0.0.1:0\"]  (default)\n\
host_name = (unset)  (default)\n\
owner_away = \"1m\"  (default)\n\
limits.max_pending_per_recipient = 50  (default)\n\
limits.rate_per_minute = 20  (default)\n\
limits.human_inbound_per_minute = 20  (default)\n\
limits.human_outbound_per_minute = 30  (default)\n\
limits.max_stored_messages = 10000  (default)\n\
limits.retention = \"3d\"  (file)\n\
limits.lease = \"1d\"  (default)\n\
limits.code_ttl = \"10m\"  (default)\n",
        layout.daemon_config().display()
    );
    assert_eq!(config::show_daemon(&layout, None).unwrap(), expected);
    let expected_env = expected.replace(
        "owner_away = \"1m\"  (default)",
        "owner_away = \"5s\"  (env)",
    );
    assert_eq!(config::show_daemon(&layout, Some(5)).unwrap(), expected_env);
}

#[test]
fn key_table_drives_show_and_template() {
    let (_dir, layout) = layout();
    let show = config::show_daemon(&layout, None).unwrap();
    let template = config::daemon_template();
    for name in [
        "listen",
        "host_name",
        "owner_away",
        "limits.max_pending_per_recipient",
        "limits.rate_per_minute",
        "limits.human_inbound_per_minute",
        "limits.human_outbound_per_minute",
        "limits.max_stored_messages",
        "limits.retention",
        "limits.lease",
        "limits.code_ttl",
    ] {
        assert!(show.contains(&format!("{name} = ")), "{name}");
        let short_name = name.rsplit('.').next().unwrap_or(name);
        assert!(template.contains(&format!("# {short_name} = ")), "{name}");
    }
}

#[test]
fn docs_template_is_consistent() {
    let docs = include_str!("../docs/configuration.md");
    assert!(docs.contains(&config::daemon_template()));
}
