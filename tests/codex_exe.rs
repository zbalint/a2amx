use a2amx::codex::executable_error;

#[test]
fn existing_regular_executable_has_no_error() {
    let file = tempfile::NamedTempFile::new().expect("temporary executable file");
    assert_eq!(executable_error(file.path()), None);
}

#[test]
fn missing_executable_tells_operator_to_restart_from_installed_binary() {
    let dir = tempfile::tempdir().expect("temporary executable directory");
    let exe = dir.path().join("a2amx");
    assert_eq!(
        executable_error(&exe),
        Some(format!(
            "the daemon's executable is gone ({}); restart the daemon from the installed binary",
            exe.to_string_lossy()
        ))
    );
}

#[test]
fn directory_is_not_a_regular_executable() {
    let dir = tempfile::tempdir().expect("temporary executable directory");
    assert!(executable_error(dir.path()).is_some());
}
