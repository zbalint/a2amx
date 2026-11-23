#![allow(clippy::expect_used, clippy::unwrap_used)]

use a2amx::agy::{self, PLUGIN, SERVER};
use a2amx::layout::Layout;
use serde_json::json;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;
use tempfile::TempDir;

static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

struct EnvGuard {
    previous: Vec<(OsString, Option<OsString>)>,
}

impl EnvGuard {
    fn new() -> Self {
        Self {
            previous: Vec::new(),
        }
    }

    fn set(&mut self, name: &str, value: Option<OsString>) {
        let key = OsString::from(name);
        self.previous.push((key.clone(), std::env::var_os(name)));
        // SAFETY: tests hold ENV_LOCK while mutating process environment and restore on Drop.
        unsafe {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, value) in self.previous.drain(..).rev() {
            // SAFETY: this guard owns every environment mutation it restores.
            unsafe {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }
}

struct Stub {
    _dir: TempDir,
    _env_lock: std::sync::MutexGuard<'static, ()>,
    program: PathBuf,
    log: PathBuf,
    state: PathBuf,
    invalid: PathBuf,
    stderr_large: PathBuf,
    stdout_loop: PathBuf,
    install_stdout_loop: PathBuf,
    list_fail_large: PathBuf,
    pipe_holder: PathBuf,
    wrong_after: PathBuf,
    wrong_content: PathBuf,
    install_fail: PathBuf,
    install_sleep: PathBuf,
    install_noop: PathBuf,
    relist_fail: PathBuf,
}

impl Stub {
    fn new() -> Self {
        let env_lock = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("agy");
        let log = dir.path().join("argv.log");
        let state = dir.path().join("state.json");
        let invalid = dir.path().join("invalid");
        let stderr_large = dir.path().join("stderr-large");
        let stdout_loop = dir.path().join("stdout-loop");
        let install_stdout_loop = dir.path().join("install-stdout-loop");
        let list_fail_large = dir.path().join("list-fail-large");
        let pipe_holder = dir.path().join("pipe-holder");
        let wrong_after = dir.path().join("wrong-after");
        let wrong_content = dir.path().join("wrong-content");
        let install_fail = dir.path().join("install-fail");
        let install_sleep = dir.path().join("install-sleep");
        let install_noop = dir.path().join("install-noop");
        let relist_fail = dir.path().join("relist-fail");
        let relist_seen = dir.path().join("relist-seen");
        let plugins = dir.path().join("plugins");
        fs::create_dir(&plugins).unwrap();
        fs::write(&state, "[]").unwrap();
        let script = r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> '@LOG@'
printf 'agy-env:%s|%s|%s\n' "${A2AMX_ADDR-}" "${A2AMX_TOKEN-}" "${A2AMX_SESSION-}" >> '@LOG@'
case "$1 $2" in
  'plugin validate')
    if test -f '@INVALID@'; then echo 'Error: validation failed: 1 error' >&2; exit 1; fi
    if test -f '@PIPE_HOLDER@'; then python3 -c 'import subprocess; subprocess.Popen(["sleep","5"])'; exit 0; fi
    if test -f '@STDERR_LARGE@'; then python3 -c 'import sys; sys.stderr.write("x"*3000)' >&2; exit 1; fi
    if test -f '@STDOUT_LOOP@'; then python3 -c 'import sys; [sys.stdout.write("x"*8192) for _ in range(512)]; sys.stdout.flush()' || exit $?; fi
    python3 - "$3" <<'PY'
import json,sys
with open(sys.argv[1]+'/mcp_config.json') as f: json.load(f)
PY
    ;;
  'plugin list')
    if test -f '@RELIST_FAIL@' && test -f '@RELIST_SEEN@'; then printf 'relist boom' >&2; exit 1; fi
    : > '@RELIST_SEEN@'
    if test -f '@LIST_FAIL_LARGE@'; then python3 -c 'import sys; [sys.stdout.write("x"*8192) for _ in range(16)]; sys.stdout.flush(); sys.stderr.write("boom"); sys.exit(1)'; fi
    cat '@STATE@' ;;
  'plugin install')
    if test -f '@INSTALL_SLEEP@'; then sleep 0.5; fi
    if test -f '@INSTALL_FAIL@'; then echo 'install boom' >&2; exit 1; fi
    if test -f '@INSTALL_NOOP@'; then printf '[]' > '@STATE@'; exit 0; fi
    if test -f '@INSTALL_STDOUT_LOOP@'; then python3 -c 'import sys; [sys.stdout.write("x"*8192) for _ in range(512)]; sys.stdout.flush()' || exit $?; fi
    rm -rf '@PLUGINS@/a2amx'
    cp -R "$3" '@PLUGINS@/a2amx'
    if test -f '@WRONG_CONTENT@'; then printf '{}\n' > '@PLUGINS@/a2amx/mcp_config.json'; fi
    version=$(sed -n 's/.*"version":"\([^"]*\)".*/\1/p' "$3/plugin.json")
    if test -f '@WRONG_AFTER@'; then version=0.0.0; fi
    enabled=true
    if grep -q '"enabled":false' '@STATE@'; then enabled=false; fi
    printf '[{"name":"a2amx","enabled":%s,"version":"%s","source":"stub","path":"@PLUGINS@/a2amx","mcpServers":{"tools":{}}}]' "$enabled" "$version" > '@STATE@'
    ;;
esac
"#
            .replace("@INVALID@", &invalid.to_string_lossy())
            .replace("@PIPE_HOLDER@", &pipe_holder.to_string_lossy())
            .replace("@STDERR_LARGE@", &stderr_large.to_string_lossy())
            .replace("@STDOUT_LOOP@", &stdout_loop.to_string_lossy())
            .replace("@INSTALL_STDOUT_LOOP@", &install_stdout_loop.to_string_lossy())
            .replace("@LIST_FAIL_LARGE@", &list_fail_large.to_string_lossy())
            .replace("@INSTALL_FAIL@", &install_fail.to_string_lossy())
            .replace("@INSTALL_SLEEP@", &install_sleep.to_string_lossy())
            .replace("@INSTALL_NOOP@", &install_noop.to_string_lossy())
            .replace("@RELIST_FAIL@", &relist_fail.to_string_lossy())
            .replace("@RELIST_SEEN@", &relist_seen.to_string_lossy())
            .replace("@WRONG_CONTENT@", &wrong_content.to_string_lossy())
            .replace("@LOG@", &log.to_string_lossy())
            .replace("@WRONG_AFTER@", &wrong_after.to_string_lossy())
            .replace("@STATE@", &state.to_string_lossy())
            .replace("@PLUGINS@", &plugins.to_string_lossy());
        fs::write(&program, script).unwrap();
        let mut permissions = fs::metadata(&program).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&program, permissions).unwrap();
        Self {
            _dir: dir,
            _env_lock: env_lock,
            program,
            log,
            state,
            invalid,
            stderr_large,
            stdout_loop,
            install_stdout_loop,
            list_fail_large,
            pipe_holder,
            wrong_after,
            wrong_content,
            install_fail,
            install_sleep,
            install_noop,
            relist_fail,
        }
    }

    fn layout(&self) -> Layout {
        let root = self._dir.path().join("root");
        fs::create_dir_all(root.join("bin")).unwrap();
        let binary = root.join("bin/a2amx");
        fs::write(
            &binary,
            format!(
                "#!/usr/bin/python3\nimport os,sys\nwith open('{}','a') as log:\n    log.write('support-argv:'+' '.join(sys.argv[1:])+'\\n')\n    log.write('support-env:'+'|'.join(os.environ.get(k,'') for k in ('A2AMX_ADDR','A2AMX_TOKEN','A2AMX_SESSION','PATH'))+'\\n')\n",
                self.log.display()
            ),
        )
        .unwrap();
        let mut permissions = fs::metadata(&binary).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&binary, permissions).unwrap();
        Layout::new(root)
    }

    fn install_log(&self) -> Vec<String> {
        fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

#[test]
fn install_writes_literal_plugin_files_and_is_idempotent() {
    let stub = Stub::new();
    let layout = stub.layout();
    agy::install(&layout, stub.program.as_os_str()).unwrap();
    let plugin = layout.agy_dir().join("plugin");
    assert_eq!(
        fs::read_to_string(plugin.join("plugin.json")).unwrap(),
        format!(
            "{{\"name\":\"a2amx\",\"version\":\"{}\",\"description\":\"a2amx messaging tools for agents started by a2amx\"}}",
            env!("CARGO_PKG_VERSION")
        )
    );
    assert_eq!(
        fs::read_to_string(plugin.join("mcp_config.json")).unwrap(),
        json!({"mcpServers": {SERVER: {"command": layout.installed_binary(), "args": ["mcp", "--optional"]}}}).to_string()
    );
    agy::install(&layout, stub.program.as_os_str()).unwrap();
    let second = stub.install_log();
    assert_eq!(
        second
            .iter()
            .filter(|line| line.contains("plugin install"))
            .count(),
        1
    );
    let log = second.join("\n");
    assert!(log.contains(&format!(
        "plugin validate {} --strict --json",
        plugin.display()
    )));
    assert!(log.contains("plugin list --json"));
    assert!(log.contains(&format!("plugin install {}", plugin.display())));
    assert_eq!(agy::PLUGIN, PLUGIN);
    assert_eq!(agy::tool_rule(), "mcp(a2amx_tools/*)");
}

#[test]
fn install_replaces_lower_or_changed_content_but_skips_higher_version() {
    let stub = Stub::new();
    let layout = stub.layout();
    agy::install(&layout, stub.program.as_os_str()).unwrap();
    let state = fs::read_to_string(&stub.state).unwrap();
    fs::write(
        &stub.state,
        state.replace(env!("CARGO_PKG_VERSION"), "00.3.01"),
    )
    .unwrap();
    agy::install(&layout, stub.program.as_os_str()).unwrap();
    assert_eq!(
        stub.install_log()
            .iter()
            .filter(|line| line.contains("plugin install"))
            .count(),
        2
    );
    fs::write(
        &stub.state,
        state.replace(env!("CARGO_PKG_VERSION"), "999.0.0"),
    )
    .unwrap();
    agy::install(&layout, stub.program.as_os_str()).unwrap();
    assert_eq!(
        stub.install_log()
            .iter()
            .filter(|line| line.contains("plugin install"))
            .count(),
        2
    );
    fs::write(
        &stub.state,
        state.replace(env!("CARGO_PKG_VERSION"), "0.0.1"),
    )
    .unwrap();
    agy::install(&layout, stub.program.as_os_str()).unwrap();
    assert_eq!(
        stub.install_log()
            .iter()
            .filter(|line| line.contains("plugin install"))
            .count(),
        3
    );
    fs::write(
        stub._dir.path().join("plugins/a2amx/mcp_config.json"),
        "{}\n",
    )
    .unwrap();
    agy::install(&layout, stub.program.as_os_str()).unwrap();
    assert_eq!(
        stub.install_log()
            .iter()
            .filter(|line| line.contains("plugin install"))
            .count(),
        4
    );
}

#[test]
fn install_rejects_missing_binary_and_invalid_plugin() {
    let stub = Stub::new();
    let layout = stub.layout();
    fs::remove_file(layout.installed_binary()).unwrap();
    let error = agy::install(&layout, stub.program.as_os_str())
        .unwrap_err()
        .to_string();
    assert!(error.contains("a2amx is not installed at"));
    assert!(stub.install_log().is_empty());

    let layout = stub.layout();
    fs::write(layout.installed_binary(), "#!/bin/sh\nexit 2\n").unwrap();
    let mut permissions = fs::metadata(layout.installed_binary())
        .unwrap()
        .permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(layout.installed_binary(), permissions).unwrap();
    let error = agy::install(&layout, stub.program.as_os_str())
        .unwrap_err()
        .to_string();
    assert!(error.contains("does not support mcp --optional"));
    assert!(stub.install_log().is_empty());
}

#[test]
fn concurrent_install_is_serialized() {
    let stub = Stub::new();
    let layout = Arc::new(stub.layout());
    let program = Arc::new(stub.program.clone());
    let mut handles = Vec::new();
    for _ in 0..2 {
        let layout = Arc::clone(&layout);
        let program = Arc::clone(&program);
        handles.push(thread::spawn(move || {
            agy::install(&layout, program.as_os_str())
        }));
    }
    for handle in handles {
        handle.join().unwrap().unwrap();
    }
    assert_eq!(
        stub.install_log()
            .iter()
            .filter(|line| line.contains("plugin install"))
            .count(),
        1
    );
}

#[test]
fn empty_command_is_rejected() {
    let stub = Stub::new();
    let layout = stub.layout();
    assert_eq!(
        agy::install(&layout, OsStr::new(""))
            .unwrap_err()
            .to_string(),
        "empty command"
    );
}

#[test]
fn invalid_plugin_stops_before_install() {
    let stub = Stub::new();
    let layout = stub.layout();
    fs::write(&stub.invalid, "1").unwrap();
    let error = agy::install(&layout, stub.program.as_os_str())
        .unwrap_err()
        .to_string();
    assert!(error.contains("agy plugin validate failed"));
    assert!(error.contains("validation failed"));
    assert!(
        stub.install_log()
            .iter()
            .all(|line| !line.contains("plugin install"))
    );
}

#[test]
fn agy_timeout_kills_a_hanging_subcommand() {
    let stub = Stub::new();
    let layout = stub.layout();
    fs::write(&stub.program, "#!/bin/sh\nsleep 60\n").unwrap();
    fs::set_permissions(&stub.program, fs::Permissions::from_mode(0o700)).unwrap();
    let started = std::time::Instant::now();
    let error = agy::install_with_timeout(
        &layout,
        stub.program.as_os_str(),
        std::time::Duration::from_millis(100),
    )
    .unwrap_err()
    .to_string();
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    assert!(error.contains("timed out"));
}

#[test]
fn disabled_unparseable_and_stray_install_state_is_handled() {
    let stub = Stub::new();
    let layout = stub.layout();
    agy::install(&layout, stub.program.as_os_str()).unwrap();
    let mut state = fs::read_to_string(&stub.state).unwrap();
    state = state.replace(
        &format!("\"version\":\"{}\"", env!("CARGO_PKG_VERSION")),
        "\"version\":\"not-semver\"",
    );
    state = state.replace("\"enabled\":true", "\"enabled\":false");
    fs::write(&stub.state, state).unwrap();
    let stray = layout.agy_dir().join("plugin/.install-1-0");
    fs::write(&stray, "stale").unwrap();
    agy::install(&layout, stub.program.as_os_str()).unwrap();
    assert!(!stray.exists());
    assert!(
        fs::read_to_string(&stub.state)
            .unwrap()
            .contains("\"enabled\":false")
    );
    let log = stub.install_log().join("\n");
    assert!(!log.contains("plugin enable"));
    assert!(!log.contains("plugin disable"));
    assert!(!log.contains("plugin uninstall"));
    assert_eq!(log.matches("plugin install").count(), 2);
}

#[test]
fn non_agy_command_uses_agy_from_path() {
    let stub = Stub::new();
    let layout = stub.layout();
    let mut env = EnvGuard::new();
    let mut path = stub._dir.path().as_os_str().to_owned();
    if let Some(old_path) = std::env::var_os("PATH") {
        path.push(":");
        path.push(old_path);
    }
    env.set("PATH", Some(path));
    agy::install(&layout, OsStr::new("sh")).unwrap();
    assert!(
        stub.install_log()
            .iter()
            .any(|line| line.contains("plugin install"))
    );
}

#[test]
fn descendant_pipe_holder_fails_with_pipe_error() {
    let stub = Stub::new();
    let layout = stub.layout();
    fs::write(&stub.pipe_holder, "1").unwrap();
    let started = std::time::Instant::now();
    let error = agy::install_with_timeout(
        &layout,
        stub.program.as_os_str(),
        std::time::Duration::from_secs(1),
    )
    .unwrap_err();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(3),
        "{error:#}"
    );
    assert_eq!(
        error.to_string(),
        "agy command left its output pipe open",
        "{error:#}"
    );
}

#[test]
fn stderr_is_truncated_without_masking_validate_failure() {
    let stub = Stub::new();
    let layout = stub.layout();
    fs::write(&stub.stderr_large, "1").unwrap();
    let error = agy::install(&layout, stub.program.as_os_str())
        .unwrap_err()
        .to_string();
    assert_eq!(
        error,
        format!("agy plugin validate failed: {}", "x".repeat(2048))
    );
}

#[test]
fn validate_stdout_over_limit_is_drained_and_succeeds() {
    let stub = Stub::new();
    let layout = stub.layout();
    fs::write(&stub.stdout_loop, "1").unwrap();
    agy::install(&layout, stub.program.as_os_str()).unwrap();
}
#[test]
fn install_stdout_over_limit_is_drained_and_succeeds() {
    let stub = Stub::new();
    let layout = stub.layout();
    fs::write(&stub.install_stdout_loop, "1").unwrap();
    agy::install(&layout, stub.program.as_os_str()).unwrap();
}

#[test]
fn list_failure_precedes_output_overflow_error() {
    let stub = Stub::new();
    let layout = stub.layout();
    fs::write(&stub.list_fail_large, "1").unwrap();
    let error = agy::install(&layout, stub.program.as_os_str())
        .unwrap_err()
        .to_string();
    assert_eq!(error, "agy plugin list failed: boom");
}

#[test]
fn install_failure_reports_the_step_and_stderr() {
    let stub = Stub::new();
    let layout = stub.layout();
    fs::write(&stub.install_fail, "1").unwrap();
    let error = agy::install(&layout, stub.program.as_os_str())
        .unwrap_err()
        .to_string();
    assert_eq!(error, "agy plugin install failed: install boom\n");
}

#[test]
fn post_install_content_mismatch_fails_with_matching_version() {
    let stub = Stub::new();
    let layout = stub.layout();
    fs::write(&stub.wrong_content, "1").unwrap();
    let error = agy::install(&layout, stub.program.as_os_str())
        .unwrap_err()
        .to_string();
    assert_eq!(error, "agy plugin install produced an invalid a2amx plugin");
}

#[test]
fn list_without_version_installs_and_unrelated_missing_version_is_ignored() {
    let stub = Stub::new();
    let layout = stub.layout();
    fs::write(
        &stub.state,
        r#"[{"name":"other","enabled":true,"path":"/missing/other"},{"name":"a2amx","enabled":true,"path":"/missing"}]"#,
    )
    .unwrap();
    agy::install(&layout, stub.program.as_os_str()).unwrap();
    assert_eq!(
        stub.install_log()
            .iter()
            .filter(|line| line
                == &&format!(
                    "plugin install {}",
                    layout.agy_dir().join("plugin").display()
                ))
            .count(),
        1
    );
}

#[test]
fn post_install_relist_mismatch_fails() {
    let stub = Stub::new();
    let layout = stub.layout();
    fs::write(&stub.wrong_after, "1").unwrap();
    let error = agy::install(&layout, stub.program.as_os_str())
        .unwrap_err()
        .to_string();
    assert_eq!(error, "agy plugin install produced an invalid a2amx plugin");
}

#[test]
fn list_output_over_limit_is_rejected_before_install() {
    let stub = Stub::new();
    let layout = stub.layout();
    fs::write(&stub.state, "x".repeat(100 * 1024)).unwrap();
    let error = agy::install(&layout, stub.program.as_os_str())
        .unwrap_err()
        .to_string();
    assert_eq!(error, "agy plugin list output exceeds 64 KiB");
    assert!(
        stub.install_log()
            .iter()
            .all(|line| !line.starts_with("plugin install "))
    );
}

#[test]
fn agy_commands_strip_session_environment_and_probe_is_exact() {
    let stub = Stub::new();
    let mut env = EnvGuard::new();
    env.set("A2AMX_ADDR", Some(OsString::from("addr")));
    env.set("A2AMX_TOKEN", Some(OsString::from("token")));
    env.set("A2AMX_SESSION", Some(OsString::from("session")));
    env.set("PATH", Some(OsString::from("/sentinel:/usr/bin:/bin")));
    let layout = stub.layout();
    agy::install(&layout, stub.program.as_os_str()).unwrap();
    let log = stub.install_log();
    assert!(log.iter().any(|line| line == "support-argv:mcp --optional"));
    assert!(log.iter().any(|line| line == "support-env:|||"), "{log:?}");
    assert!(
        log.iter()
            .filter(|line| line.starts_with("agy-env:"))
            .all(|line| line == "agy-env:||"),
        "{log:?}"
    );
}

#[test]
fn install_characterizes_edge_errors_precedence_and_list_limits() {
    let stub = Stub::new();
    let layout = stub.layout();
    let binary = layout.installed_binary();
    fs::remove_file(&binary).unwrap();
    fs::create_dir(&binary).unwrap();
    assert_eq!(
        agy::install(&layout, stub.program.as_os_str())
            .unwrap_err()
            .to_string(),
        format!(
            "a2amx is not installed at {}; the agy plugin needs the installed binary there",
            binary.display()
        )
    );
    fs::remove_dir(&binary).unwrap();
    assert_eq!(
        agy::install(&layout, OsStr::new(""))
            .unwrap_err()
            .to_string(),
        format!(
            "a2amx is not installed at {}; the agy plugin needs the installed binary there",
            binary.display()
        )
    );

    let layout = stub.layout();
    fs::write(&binary, "#!/bin/sh\nexit 2\n").unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        agy::install(&layout, stub.program.as_os_str())
            .unwrap_err()
            .to_string(),
        format!(
            "the installed a2amx at {} does not support mcp --optional; reinstall it",
            binary.display()
        )
    );

    let layout = stub.layout();
    assert_eq!(
        agy::install(&layout, OsStr::new("/nonexistent/agy"))
            .unwrap_err()
            .to_string(),
        "run /nonexistent/agy"
    );

    fs::write(&stub.state, "nope").unwrap();
    assert_eq!(
        agy::install(&layout, stub.program.as_os_str())
            .unwrap_err()
            .to_string(),
        "agy plugin list returned invalid JSON"
    );
    fs::write(&stub.invalid, "1").unwrap();
    assert_eq!(
        agy::install(&layout, stub.program.as_os_str())
            .unwrap_err()
            .to_string(),
        "agy plugin validate failed: Error: validation failed: 1 error\n"
    );
    fs::remove_file(&stub.invalid).unwrap();
    fs::write(&stub.state, "[]").unwrap();

    let stray_dir = layout.agy_dir().join("plugin/.install-d");
    fs::create_dir_all(&stray_dir).unwrap();
    agy::install(&layout, stub.program.as_os_str()).unwrap();
    assert!(stray_dir.is_dir());

    let state = fs::read_to_string(&stub.state).unwrap();
    let padded_version = env!("CARGO_PKG_VERSION")
        .split('.')
        .enumerate()
        .map(|(i, part)| {
            if i == 0 {
                format!("0{part}")
            } else {
                part.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(".");
    fs::write(
        &stub.state,
        state.replace(env!("CARGO_PKG_VERSION"), &padded_version),
    )
    .unwrap();
    agy::install(&layout, stub.program.as_os_str()).unwrap();
    assert_eq!(
        stub.install_log()
            .iter()
            .filter(|line| line.starts_with("plugin install "))
            .count(),
        2
    );

    let base_state = fs::read_to_string(&stub.state).unwrap();
    let pad_marker = ",\"pad\":\"";
    let head = &base_state[..base_state.len() - 2];
    let exact_len = 65536;
    let pad_len = exact_len - head.len() - pad_marker.len() - 3;
    let at_limit = format!("{head}{pad_marker}{}\"}}]", "a".repeat(pad_len));
    assert_eq!(at_limit.len(), 65536);
    fs::write(&stub.state, &at_limit).unwrap();
    agy::install(&layout, stub.program.as_os_str()).unwrap();
    assert_eq!(
        stub.install_log()
            .iter()
            .filter(|line| line.starts_with("plugin install "))
            .count(),
        2
    );

    let over_limit = format!("{head}{pad_marker}{}\"}}]", "a".repeat(pad_len + 1));
    assert_eq!(over_limit.len(), 65537);
    fs::write(&stub.state, &over_limit).unwrap();
    assert_eq!(
        agy::install(&layout, stub.program.as_os_str())
            .unwrap_err()
            .to_string(),
        "agy plugin list output exceeds 64 KiB"
    );
}

#[test]
fn install_characterizes_noop_relist_fail_and_sleeping_lock() {
    {
        let stub = Stub::new();
        let layout = stub.layout();
        fs::write(&stub.install_noop, "1").unwrap();
        assert_eq!(
            agy::install(&layout, stub.program.as_os_str())
                .unwrap_err()
                .to_string(),
            "agy plugin install did not install a2amx"
        );
        fs::remove_file(&stub.install_noop).unwrap();

        fs::write(&stub.relist_fail, "1").unwrap();
        assert_eq!(
            agy::install(&layout, stub.program.as_os_str())
                .unwrap_err()
                .to_string(),
            "agy plugin list failed: relist boom"
        );
        fs::remove_file(&stub.relist_fail).unwrap();
    }

    let stub_sleep = Stub::new();
    fs::write(&stub_sleep.install_sleep, "1").unwrap();
    let layout = Arc::new(stub_sleep.layout());
    let program = Arc::new(stub_sleep.program.clone());
    let mut handles = Vec::new();
    for _ in 0..2 {
        let layout = Arc::clone(&layout);
        let program = Arc::clone(&program);
        handles.push(thread::spawn(move || {
            agy::install(&layout, program.as_os_str())
        }));
    }
    for handle in handles {
        handle.join().unwrap().unwrap();
    }
    assert_eq!(
        stub_sleep
            .install_log()
            .iter()
            .filter(|line| line.starts_with("plugin install "))
            .count(),
        1
    );
}

#[test]
fn failing_probe_leaves_no_agy_state_and_probe_timeout_is_capped_at_five_seconds() {
    let stub = Stub::new();
    let layout = stub.layout();
    let binary = layout.installed_binary();
    fs::write(&binary, "#!/bin/sh\nexit 2\n").unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        agy::install(&layout, stub.program.as_os_str())
            .unwrap_err()
            .to_string()
            .contains("does not support mcp --optional")
    );
    assert!(!layout.agy_dir().exists());

    let layout = stub.layout();
    let binary = layout.installed_binary();
    fs::write(&binary, "#!/bin/sh\nsleep 60\n").unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    let started = std::time::Instant::now();
    let error = agy::install_with_timeout(
        &layout,
        stub.program.as_os_str(),
        std::time::Duration::from_secs(30),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("timed out"), "{error}");
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
}
