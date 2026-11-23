#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::Command;

use a2amx::harness::wire_agy_argv;
use serde_json::{Value, json};
use tempfile::tempdir;

fn settings(argv: &[String]) -> Value {
    let index = argv
        .iter()
        .position(|argument| argument == "--settings")
        .unwrap();
    serde_json::from_str(&argv[index + 1]).unwrap()
}

#[test]
fn default_settings_are_injected_once() {
    let gemini_dir = tempdir().unwrap();
    let cwd = Path::new("/work/project");
    let argv = wire_agy_argv(
        vec!["agy".into(), "--prompt".into(), "hello".into()],
        Path::new("/x/a2amx"),
        cwd,
        gemini_dir.path(),
    )
    .unwrap();
    assert_eq!(
        settings(&argv)["permissions"]["allow"],
        json!(["mcp(a2amx_tools/*)"])
    );
    assert_eq!(settings(&argv)["toolPermission"], Value::Null);
    assert_eq!(
        settings(&argv),
        json!({
            "statusLine": {"command": "'/x/a2amx' statusline --harness agy", "enabled": true},
            "queuedMessages": "queue",
            "trustedWorkspaces": ["/work/project"],
            "permissions": {"allow": ["mcp(a2amx_tools/*)"]}
        })
    );
    assert_eq!(
        argv.iter()
            .filter(|argument| *argument == "--settings")
            .count(),
        1
    );
    assert!(gemini_dir.path().read_dir().unwrap().next().is_none());
}

#[test]
fn user_settings_keep_other_keys_and_override_queue_and_trust() {
    let dir = tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let cwd = root.join("project");
    fs::create_dir(&cwd).unwrap();
    let input = json!({
        "other": 7,
        "queuedMessages": "send-immediately",
        "trustedWorkspaces": [cwd.to_string_lossy()],
        "statusLine": {"command": "echo user", "enabled": true, "padding": 2, "stack_with_default": false, "refreshInterval": 4}
    });
    let argv = wire_agy_argv(
        vec!["agy".into(), "--settings".into(), input.to_string()],
        Path::new("/x/a2amx"),
        &cwd,
        dir.path(),
    )
    .unwrap();
    let actual = settings(&argv);
    assert_eq!(actual["other"], 7);
    assert_eq!(actual["queuedMessages"], "queue");
    assert_eq!(actual["trustedWorkspaces"].as_array().unwrap().len(), 1);
    let status = &actual["statusLine"];
    let command = status["command"].as_str().unwrap();
    let reporter = command
        .find("statusline --harness agy --report")
        .expect("reporter command");
    let user = command.find("echo user").expect("user command");
    assert!(reporter < user);
    assert_eq!(status["enabled"], true);
    assert_eq!(status["padding"], 2);
    assert_eq!(status["stack_with_default"], false);
    assert!(status.get("type").is_none());
    assert!(status.get("refreshInterval").is_none());
}

#[test]
fn user_permissions_are_merged_and_non_array_rejected() {
    let dir = tempdir().unwrap();
    let argv = wire_agy_argv(
        vec![
            "agy".into(),
            "--settings".into(),
            json!({"permissions": {"allow": ["mcp(other/*)"]}}).to_string(),
        ],
        Path::new("/x/a2amx"),
        Path::new("/work"),
        dir.path(),
    )
    .unwrap();
    assert_eq!(
        settings(&argv)["permissions"]["allow"],
        json!(["mcp(other/*)", "mcp(a2amx_tools/*)"])
    );
    let error = wire_agy_argv(
        vec![
            "agy".into(),
            "--settings".into(),
            json!({"permissions": {"allow": "bad"}}).to_string(),
        ],
        Path::new("/x/a2amx"),
        Path::new("/work"),
        dir.path(),
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "--settings permissions.allow must be an array"
    );
}

#[test]
fn global_status_line_is_used_and_disabled_user_line_becomes_report_only() {
    let gemini_dir = tempdir().unwrap();
    let settings_dir = gemini_dir.path().join("antigravity-cli");
    fs::create_dir(&settings_dir).unwrap();
    fs::write(
        settings_dir.join("settings.json"),
        json!({"statusLine": {"command": "echo global"}}).to_string(),
    )
    .unwrap();
    let global = settings(
        &wire_agy_argv(
            vec!["agy".into()],
            Path::new("/x/a2amx"),
            Path::new("/work"),
            gemini_dir.path(),
        )
        .unwrap(),
    )["statusLine"]["command"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(global.contains("echo global"));

    let disabled = settings(
        &wire_agy_argv(
            vec![
                "agy".into(),
                "--settings".into(),
                json!({"statusLine": {"command": "echo user", "enabled": false}}).to_string(),
            ],
            Path::new("/x/a2amx"),
            Path::new("/work"),
            Path::new(""),
        )
        .unwrap(),
    );
    assert_eq!(
        disabled["statusLine"],
        json!({"command": "'/x/a2amx' statusline --harness agy --report", "enabled": true})
    );

    for raw in [
        json!({"statusLine": {"command": "a\u{0000}b", "enabled": false}}),
        json!({"statusLine": {"command": 5, "enabled": false}}),
    ] {
        let argv = wire_agy_argv(
            vec!["agy".into(), "--settings".into(), raw.to_string()],
            Path::new("/x/a2amx"),
            Path::new("/work"),
            Path::new(""),
        )
        .unwrap();
        assert_eq!(settings(&argv)["statusLine"], raw["statusLine"]);
    }
}

#[test]
fn empty_status_line_commands_become_report_only() {
    for command in ["", "   ", "\t", "\n", " \t\n "] {
        let argv = wire_agy_argv(
            vec![
                "agy".into(),
                "--settings".into(),
                json!({"statusLine": {"command": command, "enabled": true, "padding": 2, "stack_with_default": true}}).to_string(),
            ],
            Path::new("/x/a2amx"),
            Path::new("/work"),
            Path::new(""),
        )
        .unwrap();
        assert_eq!(
            settings(&argv)["statusLine"],
            json!({"command": "'/x/a2amx' statusline --harness agy --report", "enabled": true})
        );
    }
}

#[test]
fn malformed_status_line_and_trusted_workspaces_are_preserved_or_rejected() {
    let dir = tempdir().unwrap();
    let malformed = wire_agy_argv(
        vec![
            "agy".into(),
            "--settings".into(),
            json!({"statusLine": {"enabled": true}}).to_string(),
        ],
        Path::new("/x/a2amx"),
        Path::new("/work"),
        dir.path(),
    )
    .unwrap();
    assert_eq!(settings(&malformed)["statusLine"], json!({"enabled": true}));
    let error = wire_agy_argv(
        vec![
            "agy".into(),
            "--settings".into(),
            json!({"trustedWorkspaces": "not-array"}).to_string(),
        ],
        Path::new("/x/a2amx"),
        Path::new("/work"),
        dir.path(),
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "--settings trustedWorkspaces must be an array"
    );
}

#[test]
fn trusted_workspaces_include_symlink_and_canonical_paths() {
    let dir = tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let target = root.join("sub");
    fs::create_dir(&target).unwrap();
    let link = root.join("link");
    symlink(&target, &link).unwrap();
    let argv = wire_agy_argv(
        vec!["agy".into()],
        Path::new("/x/a2amx"),
        &link,
        root.join("gemini").as_path(),
    )
    .unwrap();
    assert_eq!(
        settings(&argv)["trustedWorkspaces"],
        json!([link.to_string_lossy(), target.to_string_lossy()])
    );
}

#[test]
fn trusted_workspaces_include_canonical_dot_paths() {
    let dir = tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let target = root.join("sub");
    fs::create_dir(&target).unwrap();
    let cwd = root.join("sub").join("..").join("sub");
    let argv = wire_agy_argv(
        vec!["agy".into()],
        Path::new("/x/a2amx"),
        &cwd,
        root.join("gemini").as_path(),
    )
    .unwrap();
    assert_eq!(
        settings(&argv)["trustedWorkspaces"],
        json!([cwd.to_string_lossy(), target.to_string_lossy()])
    );
}

#[test]
fn agy_empty_gemini_dir_ignores_cwd_decoy() {
    if std::env::var_os("A2AMX_AGY_DECOY_CHILD").is_some() {
        let argv = wire_agy_argv(
            vec!["agy".into()],
            Path::new("/x/a2amx"),
            Path::new("/work"),
            Path::new(""),
        )
        .unwrap();
        assert_eq!(
            settings(&argv)["statusLine"],
            json!({"command": "'/x/a2amx' statusline --harness agy", "enabled": true})
        );
        return;
    }
    let decoy_root = tempdir().unwrap();
    let decoy_settings = decoy_root.path().join("antigravity-cli");
    fs::create_dir(&decoy_settings).unwrap();
    fs::write(
        decoy_settings.join("settings.json"),
        json!({"statusLine": {"command": "echo decoy"}}).to_string(),
    )
    .unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("agy_empty_gemini_dir_ignores_cwd_decoy")
        .arg("--nocapture")
        .env("A2AMX_AGY_DECOY_CHILD", "1")
        .current_dir(decoy_root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "child failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
