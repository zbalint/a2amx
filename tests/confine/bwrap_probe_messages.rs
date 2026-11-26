use super::*;

fn write_probe_script(dir: &TempDir, name: &str, body: &str) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, body).expect("probe script");
    let mut permissions = std::fs::metadata(&path)
        .expect("probe script metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).expect("probe script permissions");
    std::fs::canonicalize(&path).expect("canonical probe script")
}

// Spawn of a just-written script can fail (ETXTBSY) while parallel tests fork.
fn retry_on_cannot_run<E: std::fmt::Display>(mut op: impl FnMut() -> Result<(), E>) -> E {
    let mut error = op().expect_err("expected probe failure");
    for _ in 1..5 {
        if !error.to_string().starts_with("cannot run") {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
        error = op().expect_err("expected probe failure");
    }
    error
}

#[test]
fn preflight_error_carries_the_exit_status_and_stderr() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = write_probe_script(&dir, "bwrap", "#!/bin/sh\nprintf 'boom\\n' >&2\nexit 3\n");
    let error = retry_on_cannot_run(|| confine::preflight(&script));
    assert_eq!(
        error.to_string(),
        format!("{} exited with exit status: 3: boom\n", script.display())
    );
}

#[test]
fn preflight_error_without_stderr_has_no_suffix() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = write_probe_script(&dir, "bwrap", "#!/bin/sh\nexit 3\n");
    let error = retry_on_cannot_run(|| confine::preflight(&script));
    assert_eq!(
        error.to_string(),
        format!("{} exited with exit status: 3", script.display())
    );
}

#[test]
fn overlay_probe_failure_text_is_pinned() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = write_probe_script(
        &dir,
        "bwrap",
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then exit 0; fi\nprintf 'first line\\nsecond line\\n' >&2\nexit 3\n",
    );
    let error =
        retry_on_cannot_run(|| confine::supports_tmp_overlay(&script, &[dir.path().to_path_buf()]));
    assert_eq!(error, "3: first line");
}

#[test]
fn resolve_bwrap_refuses_an_empty_path_value() {
    let error = confine::resolve_bwrap(Some(OsStr::new(""))).expect_err("empty PATH");
    assert_eq!(error.to_string(), "PATH is empty; cannot find bubblewrap");
}
