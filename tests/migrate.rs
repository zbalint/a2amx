#![allow(clippy::expect_used, clippy::unwrap_used)]
use std::fs::{self, OpenOptions};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use a2amx::layout::Layout;
use a2amx::migrate::{self, Outcome};
use tempfile::tempdir;

fn paths() -> (tempfile::TempDir, std::path::PathBuf, Layout) {
    let dir = tempdir().unwrap();
    let old = dir.path().join("old");
    let root = dir.path().join("new");
    fs::create_dir(&old).unwrap();
    (dir, old, Layout::new(root))
}

fn write(path: &Path, text: &[u8], mode: u32) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

#[test]
fn test_1_all_table_files_move_with_modes_and_bytes() {
    let (_dir, old, layout) = paths();
    write(&old.join("messages.db"), b"db", 0o640);
    write(&old.join("messages.db-wal"), b"wal", 0o640);
    write(&old.join("messages.db-shm"), b"shm", 0o640);
    write(&old.join("messages.db-journal"), b"journal", 0o640);
    write(&old.join("gateway/pairing.json"), b"pair", 0o640);
    write(&old.join("gateway/seen.json"), b"seen", 0o640);
    write(
        &old.join("team.toml"),
        b"allow = [\"a\"]\n[[session]]\nname = \"s\"\nallow = [\"b\"]\n",
        0o640,
    );
    write(&old.join("gateway/config.toml"), b"sync_ms = 5000\n", 0o640);
    write(&old.join("gateway/discord.token"), b"secret", 0o600);
    write(&old.join("daemon.log"), b"daemon", 0o640);
    write(&old.join("gateway/gateway.log"), b"gateway", 0o640);
    write(&old.join("addr"), b"keep", 0o600);
    write(&old.join("auto-grants"), b"keep", 0o600);

    let Outcome::Migrated(report) = migrate::migrate(&layout, &old).unwrap() else {
        panic!()
    };
    assert_eq!(report.moved.len(), 11);
    for pair in [
        (old.join("messages.db"), layout.messages_db()),
        (
            old.join("messages.db-wal"),
            layout.state_dir().join("messages.db-wal"),
        ),
        (
            old.join("messages.db-shm"),
            layout.state_dir().join("messages.db-shm"),
        ),
        (
            old.join("messages.db-journal"),
            layout.state_dir().join("messages.db-journal"),
        ),
        (old.join("gateway/pairing.json"), layout.pairing()),
        (old.join("gateway/seen.json"), layout.seen()),
        (old.join("team.toml"), layout.team_file()),
        (old.join("gateway/config.toml"), layout.gateway_config()),
        (old.join("gateway/discord.token"), layout.discord_token()),
        (old.join("daemon.log"), layout.daemon_log()),
        (old.join("gateway/gateway.log"), layout.gateway_log()),
    ] {
        assert!(report.moved.contains(&pair), "missing moved pair: {pair:?}");
    }
    assert_eq!(fs::read(layout.messages_db()).unwrap(), b"db");
    assert_eq!(
        fs::read(layout.team_file()).unwrap(),
        b"allow_teams = [\"a\"]\n[[session]]\nname = \"s\"\nallow_agents = [\"b\"]\n"
    );
    assert_eq!(
        fs::read(layout.gateway_config()).unwrap(),
        b"sync = \"5000ms\"\n"
    );
    assert_eq!(
        fs::metadata(layout.team_file())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o640
    );
    assert_eq!(
        fs::metadata(layout.gateway_config())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o640
    );
    assert_eq!(fs::read(layout.discord_token()).unwrap(), b"secret");
    assert_eq!(
        fs::metadata(layout.discord_token())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(old.join("addr").exists());
    assert!(old.join("auto-grants").exists());
    assert!(!layout.root().join("auto-grants").exists());
    assert!(!old.join("messages.db").exists());
}

#[test]
fn test_2_convert_team_lexical_cases() {
    let text = "allow=[\"a\"]\r\n[[ session ]] # c\r\n\t\"allow\" = [\"b\"]\r\n[confine]\r\nallow = [\"c\"]\r\n# allow = x\r\ncommand = [\r\n  [\"a\"],\r\n  \"allow\" = [\"d\"],\r\n]\n";
    let converted = migrate::convert_team(text);
    assert!(converted.contains("allow_teams=[\"a\"]\r\n"));
    assert!(converted.contains("\"allow_agents\" = [\"b\"]"));
    assert!(converted.contains("[confine]\r\nallow = [\"c\"]"));
    let continuation = "[[session]]\ncommand = [\n  [\"a\"],\n]\nallow = [\"x\"]\n";
    assert!(migrate::convert_team(continuation).contains("allow_agents = [\"x\"]"));
    assert!(converted.contains("\"allow\" = [\"d\"]"));
    let quoted_header = "[[\"session\"]]\nallow = [\"x\"]\n";
    assert_eq!(
        migrate::convert_team(quoted_header),
        "[[\"session\"]]\nallow_agents = [\"x\"]\n"
    );
}

#[test]
fn test_3_convert_gateway_preserves_spacing_and_comments() {
    assert_eq!(
        migrate::convert_gateway("sync_ms = 5000 # note\nidle_ms=5_000#c\nidle_ms = \"x\"\n"),
        "sync = \"5000ms\" # note\nidle=\"5000ms\"#c\nidle_ms = \"x\"\n"
    );
}

#[test]
fn test_3b_structural_verification_reports_warning_and_moves_unconverted() {
    let (_dir, old, layout) = paths();
    write(
        &old.join("team.toml"),
        b"[[session]]\nname=\"s\"\ninline = { allow = [\"x\"] }\n",
        0o600,
    );
    write(&old.join("gateway/config.toml"), b"sync_ms = 0x10\n", 0o600);
    let Outcome::Migrated(report) = migrate::migrate(&layout, &old).unwrap() else {
        panic!()
    };
    assert_eq!(
        fs::read(layout.team_file()).unwrap(),
        b"[[session]]\nname=\"s\"\ninline = { allow = [\"x\"] }\n"
    );
    assert_eq!(
        fs::read(layout.gateway_config()).unwrap(),
        b"sync_ms = 0x10\n"
    );
    assert_eq!(report.warnings.len(), 2);
    assert!(
        report
            .warnings
            .iter()
            .all(|warning| warning.contains("could not convert keys automatically"))
    );
}
#[test]
fn test_3b_unparseable_team_is_unconverted_with_warning() {
    let (_dir, old, layout) = paths();
    let bytes = b"allow = [\n";
    write(&old.join("team.toml"), bytes, 0o600);
    let Outcome::Migrated(report) = migrate::migrate(&layout, &old).unwrap() else {
        panic!()
    };
    assert_eq!(fs::read(layout.team_file()).unwrap(), bytes);
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("could not convert keys automatically"))
    );
}

#[test]
fn test_4_not_needed_does_not_create_new_root() {
    let (dir, old, layout) = paths();
    fs::remove_dir(&old).unwrap();
    assert_eq!(migrate::migrate(&layout, &old).unwrap(), Outcome::NotNeeded);
    assert!(!layout.root().exists());
    let old = dir.path().join("old2");
    fs::create_dir(&old).unwrap();
    write(&old.join("messages.db-wal"), b"wal", 0o600);
    assert_eq!(migrate::migrate(&layout, &old).unwrap(), Outcome::NotNeeded);
}

#[test]
fn test_5_new_data_refuses_and_names_both() {
    let (_dir, old, layout) = paths();
    write(&old.join("messages.db"), b"db", 0o600);
    write(&layout.config_dir().join("team.toml"), b"existing", 0o600);
    let error = migrate::migrate(&layout, &old).unwrap_err().to_string();
    assert!(error.contains(old.to_str().unwrap()));
    assert!(error.contains(layout.root().to_str().unwrap()));
    assert!(old.join("messages.db").exists());
}

#[test]
fn test_6_bin_only_root_is_allowed() {
    let (_dir, old, layout) = paths();
    write(&old.join("messages.db"), b"db", 0o600);
    write(&layout.root().join("bin/a2amx"), b"bin", 0o755);
    assert!(matches!(
        migrate::migrate(&layout, &old).unwrap(),
        Outcome::Migrated(_)
    ));
}

#[test]
fn test_9b_messages_db_symlink_refuses() {
    let (_dir, old, layout) = paths();
    std::os::unix::fs::symlink("target", old.join("messages.db")).unwrap();
    let error = migrate::migrate(&layout, &old).unwrap_err().to_string();
    assert!(error.contains("messages.db") && error.contains("is a symlink"));
}

#[test]
fn test_2b_multiline_string_does_not_change_context() {
    let text = "allow = [\"a\"]\nvalue = \"\"\"\n[[session]]\nallow = [\"not-a-key\"]\n\"\"\"\n";
    let converted = migrate::convert_team(text);
    assert!(converted.contains("allow_teams = [\"a\"]"));
    assert!(converted.contains("[[session]]\nallow = [\"not-a-key\"]"));
}

#[test]
fn test_7_locked_old_daemon_and_gateway_refuse() {
    let (_dir, old, layout) = paths();
    write(&old.join("messages.db"), b"db", 0o600);
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(old.join("daemon.lock"))
        .unwrap();
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap();
    let error = migrate::migrate(&layout, &old).unwrap_err().to_string();
    assert!(
        error.contains("still running") && error.contains("kill -TERM"),
        "{error}"
    );
    assert!(error.contains(&std::process::id().to_string()));
    assert!(old.join("messages.db").exists());
    assert!(!layout.root().exists());
    drop(lock);
    fs::remove_file(old.join("daemon.lock")).unwrap();
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(old.join("gateway.lock"))
        .unwrap();
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap();
    let error = migrate::migrate(&layout, &old).unwrap_err().to_string();
    assert!(error.contains("still running"));
    assert!(error.contains(&std::process::id().to_string()));
    assert!(old.join("messages.db").exists());
    assert!(!layout.root().exists());
}

#[test]
fn test_7b_fifo_and_symlink_locks_do_not_block_or_follow() {
    let (_dir, old, layout) = paths();
    write(&old.join("messages.db"), b"db", 0o600);
    let fifo = std::ffi::CString::new(old.join("daemon.lock").to_str().unwrap()).unwrap();
    // SAFETY: the path is a fresh test fixture under a temporary directory.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    let error = migrate::migrate(&layout, &old).unwrap_err().to_string();
    assert!(error.contains("not a regular file"), "{error}");
    fs::remove_file(old.join("daemon.lock")).unwrap();
    write(&old.join("lock-target"), b"lock", 0o600);
    std::os::unix::fs::symlink("lock-target", old.join("daemon.lock")).unwrap();
    let error = migrate::migrate(&layout, &old).unwrap_err().to_string();
    assert!(!error.is_empty());
}

#[test]
fn test_8_database_sidecars_move_only_with_database() {
    let (_dir, old, layout) = paths();
    write(&old.join("messages.db"), b"db", 0o600);
    write(&old.join("messages.db-wal"), b"wal", 0o600);
    write(&old.join("messages.db-shm"), b"shm", 0o600);
    assert!(matches!(
        migrate::migrate(&layout, &old).unwrap(),
        Outcome::Migrated(_)
    ));
    assert_eq!(
        fs::read(layout.state_dir().join("messages.db-wal")).unwrap(),
        b"wal"
    );
}

#[test]
fn test_9_symlink_table_and_gateway_refuse() {
    let (_dir, old, layout) = paths();
    std::os::unix::fs::symlink("target", old.join("team.toml")).unwrap();
    let error = migrate::migrate(&layout, &old).unwrap_err().to_string();
    assert!(error.contains("is a symlink"));
    fs::remove_file(old.join("team.toml")).unwrap();
    std::os::unix::fs::symlink("target", old.join("gateway")).unwrap();
    let error = migrate::migrate(&layout, &old).unwrap_err().to_string();
    assert!(error.contains("gateway") && error.contains("is a symlink"));
}

#[test]
fn test_10_prepare_failure_rolls_back() {
    let (_dir, old, layout) = paths();
    write(&old.join("messages.db"), b"db", 0o600);
    write(&old.join("gateway/pairing.json"), b"pair", 0o600);
    fs::create_dir_all(layout.gateway_state_dir()).unwrap();
    fs::create_dir(layout.gateway_state_dir().join("pairing.json")).unwrap();
    let error = migrate::migrate(&layout, &old).unwrap_err();
    assert!(!error.to_string().is_empty());
    assert_eq!(fs::read(old.join("messages.db")).unwrap(), b"db");
    assert_eq!(
        fs::metadata(old.join("messages.db"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(!layout.messages_db().exists());
    assert!(!layout.state_dir().join(".migrating").exists());
}

#[test]
fn test_11_empty_gateway_after_success_is_not_needed() {
    let (_dir, old, layout) = paths();
    write(&old.join("messages.db"), b"db", 0o600);
    write(&old.join("gateway/pairing.json"), b"pair", 0o600);
    write(&old.join("gateway/config.toml"), b"sync_ms = 5\n", 0o600);
    assert!(matches!(
        migrate::migrate(&layout, &old).unwrap(),
        Outcome::Migrated(_)
    ));
    assert_eq!(migrate::migrate(&layout, &old).unwrap(), Outcome::NotNeeded);

    let (_dir2, old2, layout2) = paths();
    write(&old2.join("gateway/gateway.token"), b"token", 0o600);
    assert_eq!(
        migrate::migrate(&layout2, &old2).unwrap(),
        Outcome::NotNeeded
    );
}

#[test]
fn test_11_second_migration_is_not_needed() {
    let (_dir, old, layout) = paths();
    write(&old.join("messages.db"), b"db", 0o600);
    assert!(matches!(
        migrate::migrate(&layout, &old).unwrap(),
        Outcome::Migrated(_)
    ));
    assert_eq!(migrate::migrate(&layout, &old).unwrap(), Outcome::NotNeeded);
}

fn journal(path: &Path, entries: &[(&Path, &Path, bool)]) {
    let data = serde_json::json!({"entries": entries.iter().map(|(old,new,converted)| serde_json::json!({"old":old,"new":new,"converted":converted})).collect::<Vec<_>>(), "warnings": []});
    fs::write(path, serde_json::to_vec(&data).unwrap()).unwrap();
}

#[test]
fn test_12_crash_resume_finishes_forward() {
    let (_dir, old, layout) = paths();
    write(&old.join("messages.db"), b"db", 0o600);
    write(&old.join("gateway/pairing.json"), b"pair", 0o600);
    write(&old.join("daemon.log"), b"log", 0o600);
    layout.ensure().unwrap();
    fs::rename(old.join("messages.db"), layout.messages_db()).unwrap();
    journal(
        &layout.state_dir().join(".migrating"),
        &[
            (&old.join("messages.db"), &layout.messages_db(), false),
            (&old.join("gateway/pairing.json"), &layout.pairing(), false),
            (&old.join("daemon.log"), &layout.daemon_log(), false),
        ],
    );
    assert!(matches!(
        migrate::migrate(&layout, &old).unwrap(),
        Outcome::Migrated(_)
    ));
    assert_eq!(fs::read(layout.messages_db()).unwrap(), b"db");
    assert_eq!(fs::read(layout.pairing()).unwrap(), b"pair");
    assert_eq!(fs::read(layout.daemon_log()).unwrap(), b"log");
    assert!(!old.join("messages.db").exists());
    assert!(!old.join("gateway/pairing.json").exists());
    assert!(!old.join("daemon.log").exists());
    assert!(!layout.state_dir().join(".migrating").exists());
}

#[test]
fn test_12b_resume_existing_converted_output() {
    let (_dir, old, layout) = paths();
    write(&old.join("team.toml"), b"allow=[\"a\"]\n", 0o600);
    layout.ensure().unwrap();
    write(&layout.team_file(), b"allow_teams=[\"a\"]\n", 0o600);
    journal(
        &layout.state_dir().join(".migrating"),
        &[(&old.join("team.toml"), &layout.team_file(), true)],
    );
    migrate::migrate(&layout, &old).unwrap();
    assert_eq!(
        fs::read(layout.team_file()).unwrap(),
        b"allow_teams=[\"a\"]\n"
    );
    assert!(!old.join("team.toml").exists());
    assert!(!layout.state_dir().join(".migrating").exists());
}

#[test]
fn test_12c_resume_leftover_converted_temp() {
    let (_dir, old, layout) = paths();
    write(&old.join("team.toml"), b"allow=[\"a\"]\n", 0o600);
    layout.ensure().unwrap();
    write(
        &layout.team_file().with_extension("toml.migrating"),
        b"partial",
        0o600,
    );
    journal(
        &layout.state_dir().join(".migrating"),
        &[(&old.join("team.toml"), &layout.team_file(), true)],
    );
    migrate::migrate(&layout, &old).unwrap();
    assert_eq!(
        fs::read(layout.team_file()).unwrap(),
        b"allow_teams=[\"a\"]\n"
    );
    assert!(!old.join("team.toml").exists());
    assert!(!layout.team_file().with_extension("toml.migrating").exists());
    assert!(!layout.state_dir().join(".migrating").exists());
}

#[test]
fn test_12d_prepare_failure_keeps_converted_old() {
    let (_dir, old, layout) = paths();
    write(&old.join("team.toml"), b"allow=[\"a\"]\n", 0o600);
    write(&old.join("daemon.log"), b"log", 0o600);
    layout.ensure().unwrap();
    fs::create_dir(layout.daemon_log()).unwrap();
    let error = migrate::migrate(&layout, &old).unwrap_err();
    assert!(!error.to_string().is_empty());
    assert_eq!(fs::read(old.join("team.toml")).unwrap(), b"allow=[\"a\"]\n");
    assert!(!layout.team_file().exists());
    assert!(!layout.state_dir().join(".migrating").exists());
}

#[test]
fn test_12e_nonregular_converted_destination_refuses() {
    let (_dir, old, layout) = paths();
    write(&old.join("team.toml"), b"allow=[\"a\"]\n", 0o600);
    layout.ensure().unwrap();
    fs::create_dir(layout.team_file()).unwrap();
    let error = migrate::migrate(&layout, &old).unwrap_err().to_string();
    assert!(
        error.contains("not a regular file") && error.contains("rollback failed"),
        "{error}"
    );
    assert!(old.join("team.toml").exists());
}

#[test]
fn test_12f_resume_preserves_journal_warnings() {
    let (_dir, old, layout) = paths();
    write(&old.join("messages.db"), b"db", 0o600);
    layout.ensure().unwrap();
    fs::rename(old.join("messages.db"), layout.messages_db()).unwrap();
    let journal_path = layout.state_dir().join(".migrating");
    let data = serde_json::json!({
        "entries": [{
            "old": old.join("messages.db"),
            "new": layout.messages_db(),
            "converted": false
        }],
        "warnings": ["warning from previous attempt"]
    });
    fs::write(&journal_path, serde_json::to_vec(&data).unwrap()).unwrap();

    let Outcome::Migrated(report) = migrate::migrate(&layout, &old).unwrap() else {
        panic!()
    };
    assert_eq!(report.warnings, vec!["warning from previous attempt"]);
}

#[test]
fn test_13_old_dir_resolution() {
    assert_eq!(
        migrate::old_dir(Some(std::ffi::OsStr::new("/x")), None).unwrap(),
        std::path::PathBuf::from("/x/a2amx")
    );
    assert_eq!(
        migrate::old_dir(
            Some(std::ffi::OsStr::new("relative")),
            Some(Path::new("/h"))
        )
        .unwrap(),
        std::path::PathBuf::from("/h/.local/state/a2amx")
    );
    assert_eq!(migrate::old_dir(Some(std::ffi::OsStr::new("")), None), None);
}
