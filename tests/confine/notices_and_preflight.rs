use super::*;

#[test]
fn notice_describes_restrictions_and_keeps_closing_sentence() {
    let fixture = Fixture::new();
    let restricted = confine::plan(
        &ConfineRequest {
            hide: vec![fixture.cwd.join("missing").to_string_lossy().into_owned()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("plan");
    let restricted_notice = confine::notice(&restricted);
    assert!(restricted_notice.contains("Skipped missing paths:"));
    assert!(restricted_notice.contains("Container engines are restricted."));
    assert!(restricted_notice.contains("/tmp is private"));
    assert!(restricted_notice.ends_with(confine::CLOSING_NOTICE));
    let allowed = confine::plan(
        &ConfineRequest {
            grants: vec!["docker".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("plan");
    assert!(
        confine::notice(&allowed)
            .contains("Container engines allowed: docker. Other container engines are restricted.")
    );
}

#[test]
fn notice_caps_each_category_at_twenty_entries() {
    let fixture = Fixture::new();
    for (count, suffix) in [(21, "and 1 more"), (25, "and 5 more")] {
        let mut plan = confine::plan(
            &ConfineRequest::default(),
            Harness::Generic,
            &fixture.cwd,
            &fixture.host(),
        )
        .expect("plan");
        plan.writable.clear();
        plan.hidden.clear();
        plan.writable
            .extend((0..count).map(|index| PathBuf::from(format!("/cap-writable-{index}"))));
        plan.hidden
            .extend((0..count).map(|index| PathBuf::from(format!("/cap-hidden-{index}"))));
        let text = confine::notice(&plan);
        assert_eq!(text.matches("/cap-writable-").count(), 20);
        assert_eq!(text.matches("/cap-hidden-").count(), 20);
        assert!(text.contains(suffix));
        assert!(text.len() <= 12 * 1024);
        assert!(text.ends_with(confine::CLOSING_NOTICE));
    }
    let mut under_cap = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("under-cap plan");
    under_cap.writable.clear();
    under_cap
        .writable
        .extend((0..19).map(|index| PathBuf::from(format!("/under-cap-{index}"))));
    assert!(!confine::notice(&under_cap).contains("and 0 more"));
}
#[test]
fn notice_always_names_the_working_directory() {
    let fixture = Fixture::new();
    let mut plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("plan");
    plan.writable.clear();
    plan.writable
        .extend((0..25).map(|index| PathBuf::from(format!("/a-{index:02}"))));
    plan.writable.push(plan.cwd.clone());
    let text = confine::notice(&plan);
    let writable_line = text
        .lines()
        .find(|line| line.starts_with("Writable locations: "))
        .expect("writable notice line");
    assert!(writable_line.starts_with(&format!("Writable locations: {},", plan.cwd.display())));
    assert!(writable_line.contains("and 6 more"));

    plan.writable.retain(|path| path != &plan.cwd);
    let text = confine::notice(&plan);
    let writable_line = text
        .lines()
        .find(|line| line.starts_with("Writable locations: "))
        .expect("writable notice line");
    assert!(!writable_line.contains(&plan.cwd.display().to_string()));
}
#[test]
fn notice_truncation_handles_multibyte_paths_without_panicking() {
    let fixture = Fixture::new();
    let mut plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("plan");
    for index in 0..20 {
        plan.writable.push(PathBuf::from(format!(
            "/path-{index}-{}",
            "é".repeat(2_000)
        )));
    }
    let text = confine::notice(&plan);
    assert!(text.len() <= 12 * 1024);
    assert!(text.ends_with(confine::CLOSING_NOTICE));
}

#[test]
fn prompt_notice_preserves_bubblewrap_separator() {
    let wrapped = vec![
        "bwrap".into(),
        "--ro-bind".into(),
        "/".into(),
        "/".into(),
        "--".into(),
        "claude".into(),
        "--append-system-prompt=old".into(),
        "--".into(),
        "after".into(),
    ];
    let result =
        a2amx::harness::append_system_prompt(wrapped, "notice", Harness::Claude, Path::new("."))
            .expect("append");
    assert_eq!(result[4], "--");
    assert_eq!(result[6], "--append-system-prompt=old\n\nnotice");
    assert_eq!(result[7], "--");
    assert_eq!(result[8], "after");
}
#[test]
fn append_system_prompt_extends_the_last_occurrence() {
    let result = a2amx::harness::append_system_prompt(
        vec![
            "claude".into(),
            "--append-system-prompt".into(),
            "one".into(),
            "--append-system-prompt=two".into(),
        ],
        "notice",
        Harness::Claude,
        Path::new("."),
    )
    .expect("append");
    assert_eq!(result[2], "one");
    assert_eq!(result[3], "--append-system-prompt=two\n\nnotice");
}

#[test]
fn prompt_file_options_are_refused_for_confined_harnesses() {
    assert!(
        a2amx::harness::append_system_prompt(
            vec![
                "bwrap".into(),
                "--".into(),
                "claude".into(),
                "--append-system-prompt-file=prompt.txt".into(),
            ],
            "notice",
            Harness::Claude,
            Path::new("."),
        )
        .is_err()
    );
    let file = tempfile::NamedTempFile::new().expect("prompt file");
    assert!(
        a2amx::harness::append_system_prompt(
            vec![
                "bwrap".into(),
                "--".into(),
                "omp".into(),
                "--append-system-prompt".into(),
                file.path().to_string_lossy().into_owned(),
            ],
            "notice",
            Harness::Omp,
            Path::new("."),
        )
        .is_err()
    );
    let root = tempfile::tempdir().expect("prompt directory");
    std::fs::write(root.path().join("prompt.txt"), "prompt").expect("relative prompt");
    assert!(
        a2amx::harness::append_system_prompt(
            vec![
                "bwrap".into(),
                "--".into(),
                "omp".into(),
                "--append-system-prompt=prompt.txt".into(),
            ],
            "notice",
            Harness::Omp,
            root.path(),
        )
        .is_err()
    );
    let claude_after_separator = vec![
        "claude".into(),
        "--".into(),
        "--append-system-prompt-file=prompt.txt".into(),
    ];
    assert!(
        a2amx::harness::validate_prompt_options(
            &claude_after_separator,
            Harness::Claude,
            Path::new(".")
        )
        .is_ok()
    );
    let omp_after_separator = vec![
        "omp".into(),
        "--".into(),
        "--append-system-prompt=prompt.txt".into(),
    ];
    assert!(
        a2amx::harness::validate_prompt_options(&omp_after_separator, Harness::Omp, root.path())
            .is_ok()
    );

    assert_eq!(
        a2amx::harness::validate_prompt_options(
            &["claude".into(), "--append-system-prompt".into()],
            Harness::Claude,
            Path::new("."),
        )
        .unwrap_err()
        .to_string(),
        "--append-system-prompt is missing its value"
    );
    assert_eq!(
        a2amx::harness::validate_prompt_options(
            &[
                "claude".into(),
                "--append-system-prompt-file".into(),
                "prompt.txt".into(),
            ],
            Harness::Claude,
            Path::new("."),
        )
        .unwrap_err()
        .to_string(),
        "confinement cannot be combined with --append-system-prompt-file"
    );
    assert_eq!(
        a2amx::harness::validate_prompt_options(
            &[
                "claude".into(),
                "--append-system-prompt".into(),
                "--append-system-prompt-file".into(),
            ],
            Harness::Claude,
            Path::new("."),
        )
        .unwrap_err()
        .to_string(),
        "confinement cannot be combined with --append-system-prompt-file"
    );
    assert_eq!(
        a2amx::harness::validate_prompt_options(
            &[
                "omp".into(),
                "--append-system-prompt=prompt.txt".into(),
                "--append-system-prompt".into(),
            ],
            Harness::Omp,
            root.path(),
        )
        .unwrap_err()
        .to_string(),
        "confinement cannot use an existing file as OMP's append-system-prompt value"
    );
    assert!(
        a2amx::harness::validate_prompt_options(
            &["omp".into(), "--append-system-prompt=".into()],
            Harness::Omp,
            root.path(),
        )
        .is_ok()
    );
    for harness in [Harness::Codex, Harness::Agy] {
        assert!(
            a2amx::harness::validate_prompt_options(
                &["cmd".into(), "--append-system-prompt=prompt.txt".into()],
                harness,
                root.path(),
            )
            .is_ok()
        );
    }
    let generic_argv = vec!["sh".into(), "--append-system-prompt".into()];
    assert_eq!(
        a2amx::harness::append_system_prompt(
            generic_argv.clone(),
            "notice",
            Harness::Generic,
            Path::new("."),
        )
        .unwrap(),
        generic_argv
    );
}

#[test]
fn preflight_rejects_missing_bwrap() {
    let missing = Path::new("/definitely/missing/a2amx-bwrap");
    assert!(confine::preflight(missing).is_err());
}

#[test]
fn launch_argv_prefixes_generic_with_env_wrappers() {
    let fixture = Fixture::new();
    let plan = |harness| {
        confine::plan(
            &ConfineRequest::default(),
            harness,
            &fixture.cwd,
            &fixture.host(),
        )
        .expect("plan")
        .with_bwrap(PathBuf::from("/usr/bin/bwrap"))
    };
    let generic = confine::launch_argv(
        &plan(Harness::Generic),
        "",
        vec!["sh".into(), "-c".into(), "true".into()],
        Harness::Generic,
    )
    .expect("generic launch argv");
    assert_eq!(generic[0], "/usr/bin/env");
    assert_eq!(generic[1], "--ignore-signal=INT");
    assert_eq!(generic[2], "/usr/bin/bwrap");
    assert!(generic.windows(4).any(|window| {
        window
            .iter()
            .map(String::as_str)
            .eq(["--", "/usr/bin/env", "--default-signal=INT", "sh"])
    }));

    for harness in [Harness::Claude, Harness::Omp] {
        let argv = confine::launch_argv(
            &plan(harness),
            "",
            vec!["sh".into(), "-c".into(), "true".into()],
            harness,
        )
        .expect("harness launch argv");
        assert_eq!(argv.first().map(String::as_str), Some("/usr/bin/bwrap"));
        assert!(
            !argv
                .iter()
                .any(|argument| argument == "--default-signal=INT")
        );
    }
}

#[test]
fn preflight_env_refuses_an_env_without_the_flags() {
    let failing = stub_path_dir(42);
    let failing_env = failing.path().join("bwrap");
    let error = confine::preflight_env(&failing_env).expect_err("failing env probe");
    let message = error.to_string();
    assert!(message.contains("--ignore-signal"));
    assert!(message.contains("--default-signal"));

    let passing = stub_path_dir(0);
    assert!(confine::preflight_env(&passing.path().join("bwrap")).is_ok());

    let recorder = tempfile::tempdir().expect("env recorder directory");
    let recorder_script = recorder.path().join("env");
    std::fs::write(
        &recorder_script,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"${0%/*}/args\"\n",
    )
    .expect("env recorder script");
    let mut permissions = std::fs::metadata(&recorder_script)
        .expect("env recorder metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&recorder_script, permissions).expect("env recorder permissions");
    confine::preflight_env(&recorder_script).expect("recording env probe");
    let recorded = std::fs::read_to_string(recorder.path().join("args"))
        .expect("env recorder output")
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let recorder_path = recorder_script.to_string_lossy().into_owned();
    assert_eq!(
        recorded,
        vec![
            "--ignore-signal=INT".to_owned(),
            recorder_path.clone(),
            "--default-signal=INT".to_owned(),
            recorder_path,
        ]
    );

    let missing = Path::new("/definitely/missing/a2amx-env");
    let error = confine::preflight_env(missing).expect_err("missing env probe");
    assert!(error.to_string().contains("a2amx-env"));
}

#[test]
fn find_executable_ignores_relative_and_empty_entries() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let expected = std::fs::canonicalize(manifest.join("scripts/prepush-fast.sh"))
        .expect("tracked executable");
    assert!(
        confine::find_executable("prepush-fast.sh", OsStr::new("scripts")).is_none(),
        "relative PATH entry must be ignored"
    );
    assert!(
        confine::find_executable("scripts/prepush-fast.sh", OsStr::new("")).is_none(),
        "empty PATH entry must be ignored"
    );
    assert!(
        confine::find_executable("scripts/prepush-fast.sh", OsStr::new(":")).is_none(),
        "empty PATH entries must be ignored"
    );
    let absolute_path = OsString::from(format!("{}:/usr/bin", manifest.join("scripts").display()));
    assert_eq!(
        confine::find_executable("prepush-fast.sh", &absolute_path),
        Some(expected)
    );
}

#[test]
fn resolve_bwrap_refuses_a_relative_file_path() {
    let error = confine::resolve_bwrap(Some(OsStr::new("scripts/prepush-fast.sh")))
        .expect_err("relative file path must not select bwrap");
    assert_eq!(error.to_string(), "bubblewrap was not found in PATH");
    let error =
        confine::resolve_bwrap(Some(OsStr::new("scripts"))).expect_err("directory is not bwrap");
    assert_eq!(error.to_string(), "bubblewrap was not found in PATH");
}
