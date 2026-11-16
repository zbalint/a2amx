use super::*;

#[test]
fn harness_config_default_replaces_data_overlays() {
    let claude = Fixture::new();
    populate_claude_overlay(&claude, true);
    let claude_root = std::fs::canonicalize(claude.home.join(".claude")).expect("Claude root");
    let claude_plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &claude.cwd,
        &claude.host(),
    )
    .expect("Claude plan");
    assert!(claude_plan.writable.contains(&claude_root));
    assert!(claude_plan.overlay_parents().is_empty());
    let wrapped = claude_plan
        .wrap(vec!["true".into()])
        .expect("Claude wrapper");
    let root = claude_root.to_string_lossy();
    assert!(
        wrapped
            .windows(3)
            .any(|window| window[0] == "--bind" && window[1] == root && window[2] == root)
    );

    let omp = Fixture::new();
    populate_omp_overlay(&omp);
    let omp_root = std::fs::canonicalize(omp.home.join(".omp")).expect("OMP root");
    let omp_plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Omp,
        &omp.cwd,
        &omp.host(),
    )
    .expect("OMP plan");
    assert!(omp_plan.writable.contains(&omp_root));
    assert!(omp_plan.overlay_parents().is_empty());
}

#[test]
fn overlay_is_not_planned_over_a_hidden_written_or_unhidden_parent() {
    let fixture = Fixture::new();
    populate_claude_overlay(&fixture, true);
    let parent = std::fs::canonicalize(fixture.home.join(".claude")).expect("Claude root");
    for (request, operation) in [
        (
            ConfineRequest {
                hide: vec!["~/.claude".into()],
                ..ConfineRequest::default()
            },
            "--tmpfs",
        ),
        (
            ConfineRequest {
                write: vec!["~/.claude".into()],
                ..ConfineRequest::default()
            },
            "--bind",
        ),
        (
            ConfineRequest {
                unhide: vec!["~/.claude".into()],
                ..ConfineRequest::default()
            },
            "--ro-bind",
        ),
    ] {
        let plan = confine::plan(&request, Harness::Claude, &fixture.cwd, &fixture.host())
            .expect("Claude exclusion plan");
        assert!(!plan.overlay_parents().contains(&parent));
        let wrapped = plan.wrap(vec!["true".into()]).expect("operator wrapper");
        let parent = parent.to_string_lossy();
        let retained = if operation == "--tmpfs" {
            wrapped
                .windows(2)
                .any(|window| window[0] == operation && window[1] == parent)
        } else {
            wrapped
                .windows(3)
                .any(|window| window[0] == operation && window[1] == parent && window[2] == parent)
        };
        assert!(retained, "operator mount was lost: {request:?}");
    }

    let omp = Fixture::new();
    populate_omp_overlay(&omp);
    let omp_root = std::fs::canonicalize(omp.home.join(".omp")).expect("OMP root");
    let plan = confine::plan(
        &ConfineRequest {
            hide: vec!["~/.omp".into()],
            ..ConfineRequest::default()
        },
        Harness::Omp,
        &omp.cwd,
        &omp.host(),
    )
    .expect("OMP exclusion plan");
    assert!(!plan.overlay_parents().contains(&omp_root));
    let wrapped = plan.wrap(vec!["true".into()]).expect("OMP hidden wrapper");
    let omp_root_text = omp_root.to_string_lossy();
    assert!(
        wrapped
            .windows(2)
            .any(|window| window[0] == "--tmpfs" && window[1] == omp_root_text)
    );
}

#[test]
fn harness_config_default_saves_missing_data_files() {
    let claude = Fixture::new();
    populate_claude_overlay(&claude, false);
    let plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &claude.cwd,
        &claude.host(),
    )
    .expect("Claude default plan");
    assert!(plan.unsaved_files().is_empty());
    assert!(plan.overlay_parents().is_empty());

    let omp = Fixture::new();
    populate_omp_overlay(&omp);
    let plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Omp,
        &omp.cwd,
        &omp.host(),
    )
    .expect("OMP default plan");
    assert!(plan.unsaved_files().is_empty());
    assert!(plan.overlay_parents().is_empty());
}

#[test]
fn without_overlays_keeps_the_default_harness_root_bind() {
    let fixture = Fixture::new();
    populate_claude_overlay(&fixture, true);
    let plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("Claude plan");
    let without = plan.without_overlays("x");
    assert!(without.overlay_parents().is_empty());
    assert!(
        without
            .writable
            .contains(&std::fs::canonicalize(fixture.home.join(".claude")).expect("Claude root"))
    );
    assert!(
        confine::notice(&without).contains("harness login cannot be renewed in this session: x")
    );
}

#[test]
fn harness_data_files_are_saved_through_default_root() -> anyhow::Result<()> {
    let root = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR"))?;
    let home = root.path().join("home");
    let state = root.path().join("state");
    let runtime = root.path().join("runtime");
    let cwd = root.path().join("workspace");
    let claude_root = home.join(".claude");
    let credentials = claude_root.join(".credentials.json");
    let settings = claude_root.join("settings.json");
    let plugins = claude_root.join("plugins");
    let hooks_target = root.path().join("external-hooks");
    for path in [
        &home,
        &state,
        &runtime,
        &cwd,
        &claude_root,
        &plugins,
        &hooks_target,
    ] {
        std::fs::create_dir_all(path)?;
    }
    std::fs::write(&credentials, "old")?;
    std::fs::write(&settings, "settings")?;
    std::os::unix::fs::symlink(&hooks_target, claude_root.join("hooks"))?;
    let layout = a2amx::layout::Layout::new(state.clone());
    layout.ensure()?;
    let empty_file = layout.confine_empty();
    confine::ensure_data_dirs(Harness::Claude, &home)?;
    std::fs::write(&empty_file, [])?;
    let host = Host {
        home: home.clone(),
        layout,
        runtime_dir: runtime,
    };
    let bwrap = match confine::resolve_bwrap(None) {
        Ok(path) => path,
        Err(error) if !require_bwrap() => {
            eprintln!("skipping overlay write test: {error:#}");
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let plan = confine::plan(&ConfineRequest::default(), Harness::Claude, &cwd, &host)?;
    let script = format!(
        "set -u; credentials='{}'; root='{}'; settings='{}'; tmp=\"$credentials.tmp.1\"; \
         printf renewed > \"$tmp\"; mv \"$tmp\" \"$credentials\"; \
         printf new > \"$root/new-file\"; printf changed > \"$settings\"",
        credentials.display(),
        claude_root.display(),
        settings.display()
    );
    let wrapped = plan
        .with_bwrap(bwrap)
        .wrap(vec!["sh".into(), "-c".into(), script])?;
    let status = Command::new(&wrapped[0])
        .args(&wrapped[1..])
        .status()
        .context("run real bubblewrap overlay write probe")?;
    assert_eq!(status.code(), Some(0));
    assert_eq!(std::fs::read_to_string(&credentials)?, "renewed");
    assert!(root.path().join("home/.claude/new-file").exists());
    assert!(
        !root
            .path()
            .join("home/.claude/.credentials.json.tmp.1")
            .exists()
    );
    assert_eq!(std::fs::read_to_string(&settings)?, "changed");
    Ok(())
}

#[test]
fn harness_config_grant_opens_the_own_directory() {
    let claude = Fixture::new();
    populate_claude_overlay(&claude, true);
    let claude_root = std::fs::canonicalize(claude.home.join(".claude")).expect("Claude root");
    std::fs::create_dir_all(claude_root.join("ide")).expect("Claude IDE directory");
    let path_dir = claude_root.join("local/bin");
    std::fs::create_dir_all(&path_dir).expect("PATH directory");
    let path_var = OsString::from(format!("{}:/usr/bin", path_dir.display()));
    let claude_plan = confine::plan_with_path(
        &ConfineRequest {
            grants: vec!["harness-config".into()],
            unhide: vec!["~/.claude/settings.json".into()],
            ..ConfineRequest::default()
        },
        Harness::Claude,
        &claude.cwd,
        &claude.host(),
        Some(path_var),
    )
    .expect("Claude harness-config plan");
    assert!(claude_plan.writable.contains(&claude_root));
    assert!(claude_plan.overlay_parents().is_empty());
    let wrapped = claude_plan
        .wrap(vec!["true".into()])
        .expect("Claude wrapper");
    let root_text = claude_root.to_string_lossy();
    let root_index = wrapped
        .windows(3)
        .position(|window| {
            window[0] == "--bind" && window[1] == root_text && window[2] == root_text
        })
        .expect("Claude root writable bind");
    let path_dir = std::fs::canonicalize(path_dir).expect("PATH canonical");
    assert!(claude_plan.protected.contains(&path_dir));
    let path_dir_text = path_dir.to_string_lossy();
    let path_dir_index = wrapped
        .windows(3)
        .position(|window| {
            window[0] == "--ro-bind" && window[1] == path_dir_text && window[2] == path_dir_text
        })
        .expect("nested 3bo protected launch input");
    assert!(root_index < path_dir_index);
    let ide = claude_root.join("ide");
    let ide_text = ide.to_string_lossy();
    let ide_index = wrapped
        .windows(2)
        .position(|window| window[0] == "--tmpfs" && window[1] == ide_text)
        .expect("Claude IDE hide");
    let settings = claude_root.join("settings.json");
    let settings_text = settings.to_string_lossy();
    let settings_index = wrapped
        .windows(3)
        .position(|window| {
            window[0] == "--ro-bind" && window[1] == settings_text && window[2] == settings_text
        })
        .expect("Claude read-only settings bind");
    assert!(root_index < ide_index);
    assert!(root_index < settings_index);
    for path in [
        claude_root.join("projects"),
        claude_root.join("history.jsonl"),
        claude_root.join(".credentials.json"),
    ] {
        assert!(
            !wrapped
                .iter()
                .any(|argument| argument == path.to_string_lossy().as_ref()),
            "harness data path must be covered by the root bind: {}",
            path.display()
        );
    }
    assert!(confine::notice(&claude_plan).contains(&format!(
        "{}: harness config, writable and persistent (grant harness-config).",
        claude_root.display()
    )));

    let plain = confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &claude.cwd,
        &claude.host(),
    )
    .expect("Claude plain plan");
    assert!(confine::notice(&plain).contains("harness config, writable and persistent"));
    assert!(plain.writable.contains(&claude_root));
    assert!(plain.overlay_parents().is_empty());

    let omp = Fixture::new();
    populate_omp_overlay(&omp);
    let omp_root = std::fs::canonicalize(omp.home.join(".omp")).expect("OMP root");
    let omp_plan = confine::plan(
        &ConfineRequest {
            grants: vec!["harness-config".into()],
            ..ConfineRequest::default()
        },
        Harness::Omp,
        &omp.cwd,
        &omp.host(),
    )
    .expect("OMP harness-config plan");
    assert!(omp_plan.writable.contains(&omp_root));
    assert!(confine::notice(&omp_plan).contains(&format!(
        "{}: harness config, writable and persistent (grant harness-config).",
        omp_root.display()
    )));
    assert!(omp_plan.overlay_parents().is_empty());
    let wrapped = omp_plan.wrap(vec!["true".into()]).expect("OMP wrapper");
    let root_text = omp_root.to_string_lossy();
    assert!(wrapped.windows(3).any(|window| {
        window[0] == "--bind" && window[1] == root_text && window[2] == root_text
    }));
    for path in [
        omp_root.join("agent/a.db"),
        omp_root.join("stats.db"),
        omp_root.join("config.yml"),
    ] {
        assert!(
            !wrapped
                .iter()
                .any(|argument| argument == path.to_string_lossy().as_ref()),
            "OMP harness data path must be covered by the root bind: {}",
            path.display()
        );
    }
    assert!(
        !wrapped
            .iter()
            .any(|argument| { argument == omp_root.join("agent").to_string_lossy().as_ref() })
    );
}

#[test]
fn harness_config_is_default_for_claude_and_omp() {
    let claude = Fixture::new();
    populate_claude_overlay(&claude, true);
    let claude_root = std::fs::canonicalize(claude.home.join(".claude")).expect("Claude root");
    let ide = claude_root.join("ide");
    std::fs::create_dir_all(&ide).expect("Claude IDE directory");
    let implicit = confine::plan_with_path(
        &ConfineRequest {
            ..ConfineRequest::default()
        },
        Harness::Claude,
        &claude.cwd,
        &claude.host(),
        None,
    )
    .expect("implicit Claude harness config");
    assert!(implicit.writable.contains(&claude_root));
    assert!(implicit.grants.is_empty());
    assert!(confine::notice(&implicit).contains(&format!(
        "{}: harness config, writable and persistent (default for this harness).",
        claude_root.display()
    )));

    let no_default_hides = confine::plan_with_path(
        &ConfineRequest {
            no_default_hides: true,
            ..ConfineRequest::default()
        },
        Harness::Claude,
        &claude.cwd,
        &claude.host(),
        None,
    )
    .expect("implicit Claude harness config without default hides");
    assert!(no_default_hides.writable.contains(&claude_root));
    assert!(!no_default_hides.hidden_paths().contains(&ide));

    let opt_out = confine::plan(
        &ConfineRequest::default(),
        Harness::Claude,
        &claude.cwd,
        &claude.host(),
    )
    .expect("Claude opt-out plan");
    assert!(opt_out.writable.contains(&claude_root));

    let omp = Fixture::new();
    populate_omp_overlay(&omp);
    let omp_root = std::fs::canonicalize(omp.home.join(".omp")).expect("OMP root");
    let omp_plan = confine::plan_with_path(
        &ConfineRequest {
            ..ConfineRequest::default()
        },
        Harness::Omp,
        &omp.cwd,
        &omp.host(),
        None,
    )
    .expect("implicit OMP harness config");
    assert!(omp_plan.writable.contains(&omp_root));
    assert!(omp_plan.grants.is_empty());
    assert!(confine::notice(&omp_plan).contains(&format!(
        "{}: harness config, writable and persistent (default for this harness).",
        omp_root.display()
    )));

    let generic = Fixture::new();
    confine::plan_with_path(
        &ConfineRequest {
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &generic.cwd,
        &generic.host(),
        None,
    )
    .expect("Generic confinement plan");
}

#[test]
fn operator_harness_root_hide_or_unhide_skips_implicit_grant() {
    let fixture = Fixture::new();
    populate_claude_overlay(&fixture, true);
    let root = std::fs::canonicalize(fixture.home.join(".claude")).expect("Claude root");
    for (field, entry) in [("hide", "~/.claude"), ("unhide", "~/.claude")] {
        let request = if field == "hide" {
            ConfineRequest {
                hide: vec![entry.into()],
                ..ConfineRequest::default()
            }
        } else {
            ConfineRequest {
                unhide: vec![entry.into()],
                ..ConfineRequest::default()
            }
        };
        let plan = confine::plan(&request, Harness::Claude, &fixture.cwd, &fixture.host())
            .expect("operator root override");
        assert!(!plan.writable.contains(&root));
        assert!(plan.grants.is_empty());
        assert!(plan.unresolved.iter().any(|item| {
            item.option == "harness-config"
                && item.path == root
                && item.reason == "operator hide or unhide of the harness root"
        }));
        let notice = confine::notice(&plan);
        assert!(!notice.contains("harness config"));
        assert!(!notice.contains("unresolved"));
    }
}

#[test]
fn harness_config_grant_refuses_generic_hide_and_unhide() {
    let generic = Fixture::new();
    let error = match confine::plan(
        &ConfineRequest {
            grants: vec!["harness-config".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &generic.cwd,
        &generic.host(),
    ) {
        Ok(_) => panic!("Generic harness-config grant was accepted"),
        Err(error) => error,
    };
    assert_eq!(
        error.to_string(),
        "confinement grant \"harness-config\" needs a Claude, OMP, Codex, or Agy session"
    );

    let claude = Fixture::new();
    populate_claude_overlay(&claude, true);
    for (operation, expected) in [
        (
            ConfineRequest {
                grants: vec!["harness-config".into()],
                hide: vec!["~/.claude".into()],
                ..ConfineRequest::default()
            },
            "confinement grant \"harness-config\" conflicts with the hide of",
        ),
        (
            ConfineRequest {
                grants: vec!["harness-config".into()],
                unhide: vec!["~/.claude".into()],
                ..ConfineRequest::default()
            },
            "confinement grant \"harness-config\" conflicts with the unhide of",
        ),
    ] {
        let error = match confine::plan(&operation, Harness::Claude, &claude.cwd, &claude.host()) {
            Ok(_) => panic!("harness-config conflict was accepted"),
            Err(error) => error,
        };
        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn harness_config_grant_persists_regular_writes() -> anyhow::Result<()> {
    let root = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR"))?;
    let home = root.path().join("home");
    let state = root.path().join("state");
    let runtime = root.path().join("runtime");
    let cwd = root.path().join("workspace");
    let claude_root = home.join(".claude");
    let credentials = claude_root.join(".credentials.json");
    let settings = claude_root.join("settings.json");
    for path in [&home, &state, &runtime, &cwd, &claude_root] {
        std::fs::create_dir_all(path)?;
    }
    for name in [
        "projects",
        "todos",
        "shell-snapshots",
        "session-env",
        "sessions",
        "statsig",
        "file-history",
        "paste-cache",
        "debug",
        "cache",
        "telemetry",
        "plans",
        "backups",
    ] {
        std::fs::create_dir_all(claude_root.join(name))?;
    }
    std::fs::write(&credentials, "old")?;
    std::fs::write(&settings, "settings")?;
    confine::ensure_data_dirs(Harness::Claude, &home)?;
    let layout = a2amx::layout::Layout::new(state.clone());
    layout.ensure()?;
    let empty_file = layout.confine_empty();
    std::fs::write(&empty_file, [])?;
    let host = Host {
        home: home.clone(),
        layout,
        runtime_dir: runtime,
    };
    let bwrap = match confine::resolve_bwrap(None) {
        Ok(path) => path,
        Err(error) if !require_bwrap() => {
            eprintln!("skipping harness-config write test: {error:#}");
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let plain = confine::plan(&ConfineRequest::default(), Harness::Claude, &cwd, &host)?;

    let granted = confine::plan(
        &ConfineRequest {
            grants: vec!["harness-config".into()],
            ..ConfineRequest::default()
        },
        Harness::Claude,
        &cwd,
        &host,
    )?;
    let script = format!(
        "set -u; settings='{}'; credentials='{}'; tmp=\"$credentials.tmp.1\"; \
         printf changed > \"$settings\"; printf renewed > \"$tmp\"; mv \"$tmp\" \"$credentials\"",
        settings.display(),
        credentials.display()
    );
    let wrapped = granted
        .with_bwrap(bwrap.clone())
        .wrap(vec!["sh".into(), "-c".into(), script])?;
    let status = Command::new(&wrapped[0])
        .args(&wrapped[1..])
        .status()
        .context("run granted harness-config write probe")?;
    assert_eq!(status.code(), Some(0));
    assert_eq!(std::fs::read_to_string(&settings)?, "changed");
    assert_eq!(std::fs::read_to_string(&credentials)?, "renewed");
    assert!(
        !credentials
            .with_file_name(".credentials.json.tmp.1")
            .exists()
    );

    std::fs::write(&settings, "settings")?;
    let script = format!(
        "if printf changed > '{}'; then exit 1; else exit 0; fi",
        settings.display()
    );
    let wrapped = plain
        .with_bwrap(bwrap)
        .wrap(vec!["sh".into(), "-c".into(), script])?;
    let status = Command::new(&wrapped[0])
        .args(&wrapped[1..])
        .status()
        .context("run ungranted harness-config write probe")?;
    assert_eq!(status.code(), Some(1));
    assert_eq!(std::fs::read_to_string(&settings)?, "changed");
    Ok(())
}
