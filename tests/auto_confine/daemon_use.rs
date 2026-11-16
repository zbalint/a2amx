use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_mode_stays_true_when_auto_adds_nothing() {
    let bwrap = AsyncCommand::new("bwrap").arg("--version").output().await;
    if !matches!(bwrap, Ok(output) if output.status.success()) {
        eprintln!("skipping auto-adds-nothing test: bubblewrap unavailable");
        return;
    }
    let state = tempfile::tempdir().expect("state");
    let project = tempfile::tempdir().expect("project with no markers");
    let bwrap = String::from_utf8(
        AsyncCommand::new("which")
            .arg("bwrap")
            .output()
            .await
            .expect("which bwrap")
            .stdout,
    )
    .expect("bwrap path");
    let bwrap_dir = state.path().join("bwrap-path");
    std::fs::create_dir_all(&bwrap_dir).expect("bwrap directory");
    std::os::unix::fs::symlink(bwrap.trim(), bwrap_dir.join("bwrap")).expect("bwrap symlink");
    let _daemon = common::start_daemon_in(
        state.path(),
        Some("host-a"),
        a2amx::messaging::Limits::default(),
    )
    .await;
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    // The session already names every grant auto would add here (cache only: no
    // markers, no git remote), so auto runs and adds nothing; `auto` stays true.
    std::fs::write(
        state.path().join("config/team.toml"),
        "[confine]\nenabled = true\nauto = true\n[[session]]\nname = \"worker\"\ncommand = [\"sh\", \"-c\", \"sleep 30\"]\n[session.confine]\ngrants = [\"cache\"]\n",
    )
    .expect("global team file");

    let dry = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("dry run");
    assert!(
        dry.status.success(),
        "{}",
        String::from_utf8_lossy(&dry.stderr)
    );
    let dry_stdout = String::from_utf8_lossy(&dry.stdout);
    assert!(dry_stdout.contains("  confinement: auto\n"), "{dry_stdout}");
    assert!(!dry_stdout.contains("always added by auto"), "{dry_stdout}");
    assert!(
        !dry_stdout.contains("  cargo: ") && !dry_stdout.contains("  ssh: "),
        "{dry_stdout}"
    );
    let dry_stderr = String::from_utf8_lossy(&dry.stderr);
    assert!(
        !dry_stderr.contains("auto confinement for session"),
        "{dry_stderr}"
    );

    let real = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--detach", "--yes"],
    )
    .expect("real up");
    assert!(
        real.status.success(),
        "{}",
        String::from_utf8_lossy(&real.stderr)
    );
    let real_stderr = String::from_utf8_lossy(&real.stderr);
    assert!(
        !real_stderr.contains("auto confinement for session"),
        "{real_stderr}"
    );
    let started_line = String::from_utf8_lossy(&real.stdout)
        .lines()
        .find(|line| line.starts_with("started ") && line.contains("worker"))
        .expect("started worker")
        .to_owned();
    let worker_id = started_line
        .split_whitespace()
        .nth(2)
        .expect("started worker id")
        .to_owned();
    let listed = String::from_utf8(
        run_binary(
            state.path(),
            project.path(),
            user_home.path(),
            runtime.path(),
            &["list"],
        )
        .expect("list")
        .stdout,
    )
    .expect("list output");
    let cells = common::list_cells(&listed, &worker_id);
    assert!(cells.contains(&"auto"), "{listed}");
    assert!(cells.contains(&"running"), "{listed}");

    let _ = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "down", "--now"],
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_auto_and_file_auto_work_before_daemon_use() {
    let (state, _daemon) = start_daemon().await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    std::fs::write(project.path().join("Cargo.toml"), "").expect("cargo marker");
    std::fs::write(
        project.path().join("a2amx.toml"),
        "[confine]\nenabled = true\nauto = true\n[[session]]\nname = \"worker\"\ncommand = [\"cat\"]\n",
    )
    .expect("local team file");
    std::fs::write(
        state.path().join("state/auto-grants.toml"),
        format!(
            "[projects.{:?}]\ngrants = [\"cache\", \"cargo\"]\n",
            project.path().to_string_lossy()
        ),
    )
    .expect("seed first local acceptance");
    let local = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("local auto dry run");
    assert!(
        local.status.success(),
        "{}",
        String::from_utf8_lossy(&local.stderr)
    );
    assert!(String::from_utf8_lossy(&local.stdout).contains("  grants: cache, cargo"));
    assert!(!String::from_utf8_lossy(&local.stdout).contains("not yet accepted"));
    let local_b = tempfile::tempdir().expect("second local project");
    std::fs::write(local_b.path().join("Cargo.toml"), "").expect("second cargo marker");
    std::fs::write(
        local_b.path().join("a2amx.toml"),
        "[confine]\nenabled = true\nauto = true\n[[session]]\nname = \"worker\"\ncommand = [\"cat\"]\n",
    )
    .expect("second local team file");
    let local_b_run = run_binary(
        state.path(),
        local_b.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("second local dry run");
    assert!(local_b_run.status.success());
    assert!(String::from_utf8_lossy(&local_b_run.stdout).contains("not yet accepted"));
    let file_project = tempfile::tempdir().expect("file project");
    std::fs::write(file_project.path().join("Cargo.toml"), "").expect("file cargo marker");
    let file = file_project.path().join("team.toml");
    std::fs::write(
        &file,
        "[confine]\nenabled = true\nauto = true\n[[session]]\nname = \"worker\"\ncommand = [\"cat\"]\n",
    )
    .expect("explicit team file");
    let invoke_project = tempfile::tempdir().expect("invoking project");
    std::fs::write(
        state.path().join("state/auto-grants.toml"),
        format!(
            "[projects.{:?}]\ngrants = [\"cache\", \"cargo\"]\n",
            file_project.path().to_string_lossy()
        ),
    )
    .expect("seed explicit project acceptance");
    let explicit_a = run_binary(
        state.path(),
        file_project.path(),
        user_home.path(),
        runtime.path(),
        &[
            "team",
            "up",
            "--dry-run",
            "--file",
            file.to_str().expect("file path"),
        ],
    )
    .expect("explicit file A dry run");
    assert!(explicit_a.status.success());
    assert!(!String::from_utf8_lossy(&explicit_a.stdout).contains("not yet accepted"));
    let explicit = run_binary(
        state.path(),
        invoke_project.path(),
        user_home.path(),
        runtime.path(),
        &[
            "team",
            "up",
            "--dry-run",
            "--file",
            file.to_str().expect("file path"),
        ],
    )
    .expect("explicit file auto dry run");
    assert!(
        explicit.status.success(),
        "{}",
        String::from_utf8_lossy(&explicit.stderr)
    );
    assert!(String::from_utf8_lossy(&explicit.stdout).contains("  grants: cache, cargo"));
    assert!(!String::from_utf8_lossy(&explicit.stdout).contains("not yet accepted"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_without_auto_has_no_automatic_grants() {
    let (state, _daemon) = start_daemon().await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    std::fs::write(
        state.path().join("config/team.toml"),
        "[confine]\nenabled = true\n[[session]]\nname = \"worker\"\ncommand = [\"cat\"]\n",
    )
    .expect("global team file");
    std::fs::write(project.path().join("Cargo.toml"), "").expect("cargo marker");
    let output = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("dry run");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("  grants:"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_up_reuses_auto_grants_and_running_session_is_not_rescanned() {
    let bwrap = AsyncCommand::new("bwrap").arg("--version").output().await;
    if !matches!(bwrap, Ok(output) if output.status.success()) {
        eprintln!("skipping real auto start test: bubblewrap unavailable");
        return;
    }
    let state = tempfile::tempdir().expect("state");
    let project = tempfile::tempdir().expect("project");
    let bwrap = String::from_utf8(
        AsyncCommand::new("which")
            .arg("bwrap")
            .output()
            .await
            .expect("which bwrap")
            .stdout,
    )
    .expect("bwrap path");
    let bwrap_dir = state.path().join("bwrap-path");
    std::fs::create_dir_all(&bwrap_dir).expect("bwrap directory");
    std::os::unix::fs::symlink(bwrap.trim(), bwrap_dir.join("bwrap")).expect("bwrap symlink");
    let _daemon = common::start_daemon_in(
        state.path(),
        Some("host-a"),
        a2amx::messaging::Limits::default(),
    )
    .await;
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    std::fs::write(
        state.path().join("config/team.toml"),
        "[confine]\nenabled = true\nauto = true\n[[session]]\nname = \"worker\"\ncommand = [\"sh\", \"-c\", \"sleep 30\"]\n",
    )
    .expect("global team file");
    std::fs::write(project.path().join("Cargo.toml"), "").expect("cargo marker");

    let dry_run = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("dry run");
    assert!(dry_run.status.success());
    assert!(String::from_utf8_lossy(&dry_run.stdout).contains("grants: cache, cargo"));

    let real = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--detach", "--yes"],
    )
    .expect("real up");
    assert!(
        real.status.success(),
        "{}",
        String::from_utf8_lossy(&real.stderr)
    );
    let real_stderr = String::from_utf8_lossy(&real.stderr);
    assert_eq!(
        real_stderr
            .lines()
            .filter(|line| line.contains("auto confinement for session"))
            .count(),
        1
    );

    let second = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--detach", "--yes"],
    )
    .expect("second up");
    assert!(second.status.success());
    assert!(!String::from_utf8_lossy(&second.stderr).contains("auto confinement for session"));

    let started_line = String::from_utf8_lossy(&real.stdout)
        .lines()
        .find(|line| line.starts_with("started ") && line.contains("worker"))
        .unwrap_or_else(|| {
            panic!(
                "started worker id\nSTDOUT: {}\nSTDERR: {}",
                String::from_utf8_lossy(&real.stdout),
                String::from_utf8_lossy(&real.stderr)
            )
        })
        .to_owned();
    let mut started_fields = started_line.split_whitespace();
    let _ = started_fields.next();
    let worker_name = started_fields
        .next()
        .expect("started worker name")
        .to_owned();
    let worker_id = started_fields.next().expect("started worker id").to_owned();

    {
        let listed = String::from_utf8(
            run_binary(
                state.path(),
                project.path(),
                user_home.path(),
                runtime.path(),
                &["list"],
            )
            .expect("list")
            .stdout,
        )
        .expect("list output");
        let cells = common::list_cells(&listed, &worker_id);
        // The team cell may itself contain a space ("name (private)"), so assert on
        // the cell set rather than fragile positional indexes.
        assert!(cells.contains(&"generic"), "{listed}");
        assert!(cells.contains(&"auto"), "{listed}");
        assert!(cells.contains(&"running"), "{listed}");
    }
    {
        let listed = String::from_utf8(
            run_binary(
                state.path(),
                project.path(),
                user_home.path(),
                runtime.path(),
                &["list", "--details"],
            )
            .expect("details list")
            .stdout,
        )
        .expect("details output");
        let cells = common::list_cells(&listed, &worker_id);
        assert!(cells.contains(&"generic"), "{listed}");
        assert!(cells.contains(&"auto"), "{listed}");
    }
    {
        let status = String::from_utf8(
            run_binary(
                state.path(),
                project.path(),
                user_home.path(),
                runtime.path(),
                &["team", "status"],
            )
            .expect("team status")
            .stdout,
        )
        .expect("team status output");
        let row = status
            .lines()
            .find(|line| line.starts_with(&worker_name))
            .unwrap_or_else(|| panic!("missing worker row in {status}"));
        assert!(row.contains(" running auto "), "{row}");
        assert!(row.ends_with(&worker_id), "{row}");
    }
    {
        let second_stdout = String::from_utf8_lossy(&second.stdout);
        assert!(
            second_stdout.contains(&format!(
                "already running {worker_name} {worker_id} (confine auto)"
            )),
            "{second_stdout}"
        );
    }

    // A manually confined session must keep `manual`, never `off` or `auto`.
    let manual_project = tempfile::tempdir().expect("manual project");
    std::fs::write(
        manual_project.path().join("manual.toml"),
        "team = \"manual\"\n[confine]\nenabled = true\n[[session]]\nname = \"manualworker\"\ncommand = [\"sh\", \"-c\", \"sleep 30\"]\n",
    )
    .expect("manual team file");
    let manual_up = run_binary(
        state.path(),
        manual_project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--detach", "--file", "manual.toml"],
    )
    .expect("manual up");
    assert!(
        manual_up.status.success(),
        "{}",
        String::from_utf8_lossy(&manual_up.stderr)
    );
    assert!(
        !String::from_utf8_lossy(&manual_up.stderr).contains("auto confinement for session"),
        "{}",
        String::from_utf8_lossy(&manual_up.stderr)
    );
    let manual_id = String::from_utf8_lossy(&manual_up.stdout)
        .lines()
        .find(|line| line.starts_with("started manual-manualworker "))
        .and_then(|line| line.split_whitespace().nth(2))
        .expect("started manual-manualworker id")
        .to_owned();
    {
        let listed = String::from_utf8(
            run_binary(
                state.path(),
                manual_project.path(),
                user_home.path(),
                runtime.path(),
                &["list", "--details"],
            )
            .expect("manual details list")
            .stdout,
        )
        .expect("manual details output");
        let cells = common::list_cells(&listed, &manual_id);
        assert!(cells.contains(&"generic"), "{listed}");
        assert!(cells.contains(&"manual"), "{listed}");
        assert!(cells.contains(&"running"), "{listed}");
    }
    {
        let status = String::from_utf8(
            run_binary(
                state.path(),
                manual_project.path(),
                user_home.path(),
                runtime.path(),
                &["team", "status", "--file", "manual.toml"],
            )
            .expect("manual team status")
            .stdout,
        )
        .expect("manual team status output");
        assert!(
            status.contains(&format!(
                "manual-manualworker manual (private) running manual {manual_id}"
            )),
            "{status}"
        );
    }
    {
        let dry = run_binary(
            state.path(),
            manual_project.path(),
            user_home.path(),
            runtime.path(),
            &["team", "up", "--dry-run", "--file", "manual.toml"],
        )
        .expect("manual dry run");
        assert!(dry.status.success());
        let stdout = String::from_utf8_lossy(&dry.stdout);
        std::eprintln!("DEBUG manual_id={manual_id:?} stdout={stdout:?}");
        assert!(
            stdout.contains(&format!(
                "already running manual-manualworker {manual_id} (confine manual)"
            )),
            "{stdout}"
        );
    }

    let _ = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "down", "--now"],
    );
}
