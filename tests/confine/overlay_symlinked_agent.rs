use std::collections::BTreeSet;
use std::os::unix::fs::symlink;
use std::path::Path;

use super::*;

fn symlinked_agent_fixture(inside_root: bool) -> (Fixture, std::path::PathBuf) {
    let fixture = Fixture::new();
    let omp_root = fixture.home.join(".omp");
    std::fs::create_dir_all(&omp_root).expect("OMP root");
    let target = if inside_root {
        omp_root.join("agent-real")
    } else {
        fixture._root.path().join("omp-agent")
    };
    std::fs::create_dir_all(target.join("sessions")).expect("OMP sessions");
    std::fs::write(target.join("a.db"), "agent").expect("agent database");
    symlink(&target, omp_root.join("agent")).expect("OMP agent symlink");
    let target = std::fs::canonicalize(target).expect("target canonicalization");
    (fixture, target)
}

#[test]
fn omp_agent_symlinked_outside_the_root_plans_the_target_as_overlay_parent() {
    let (fixture, target) = symlinked_agent_fixture(false);
    let plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Omp,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("OMP plan");
    assert_eq!(plan.overlay_parents(), std::slice::from_ref(&target));
    assert!(plan.unsaved_files().is_empty());
    let omp_root =
        std::fs::canonicalize(fixture.home.join(".omp")).expect("OMP root canonicalization");
    assert!(plan.writable.contains(&omp_root));
}

#[test]
fn overlay_parent_shows_in_the_notice_and_the_dry_run() {
    let (fixture, target) = symlinked_agent_fixture(false);
    let plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Omp,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("OMP plan");
    let notice_line = format!(
        "{}: writable, but only the harness's data files are saved; other writes are discarded at exit.",
        target.display()
    );
    assert!(confine::notice(&plan).contains(&notice_line));
    let dry_run = confine::dry_run_lines(&plan, &fixture.cwd, &BTreeSet::new());
    assert!(dry_run.contains(&format!("  overlay: {}\n", target.display())));
}

#[test]
fn omp_agent_symlinked_inside_the_root_has_no_overlay_parent() {
    let (fixture, _) = symlinked_agent_fixture(true);
    let plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Omp,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("OMP plan");
    assert!(plan.overlay_parents().is_empty());
}

#[test]
fn writing_the_symlinked_agent_target_has_no_overlay_parent() {
    let (fixture, target) = symlinked_agent_fixture(false);
    let plan = confine::plan(
        &ConfineRequest {
            write: vec![target.display().to_string()],
            ..ConfineRequest::default()
        },
        Harness::Omp,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("OMP plan");
    assert!(plan.overlay_parents().is_empty());
}

fn symlinked_agent_with_entries_fixture() -> (Fixture, std::path::PathBuf) {
    let (fixture, target) = symlinked_agent_fixture(false);
    std::fs::create_dir_all(target.join("extra")).expect("extra directory");
    std::fs::write(target.join("notes.txt"), "notes").expect("notes file");
    symlink(target.join("notes.txt"), target.join("link")).expect("notes symlink");
    (fixture, target)
}

fn ro_bind_try_names(argv: &[String], target: &Path) -> Vec<String> {
    let mut names: Vec<String> = argv
        .windows(3)
        .filter(|window| window[0] == "--ro-bind-try" && window[1] == window[2])
        .filter_map(|window| {
            let path = Path::new(&window[1]);
            if path.parent() == Some(target) {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_owned)
            } else {
                None
            }
        })
        .collect();
    names.sort();
    names
}

#[test]
fn overlay_protected_entries_are_the_unlisted_plain_entries_of_the_overlay_parent() {
    let (fixture, target) = symlinked_agent_with_entries_fixture();
    let plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Omp,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("OMP plan");
    let wrapped = plan.wrap(vec!["true".into()]).expect("wrap");
    assert_eq!(
        ro_bind_try_names(&wrapped, &target),
        vec!["extra", "notes.txt"]
    );
}

#[test]
fn a_written_entry_is_not_overlay_protected() {
    let (fixture, target) = symlinked_agent_with_entries_fixture();
    let extra = target.join("extra");
    let request = ConfineRequest {
        write: vec![extra.display().to_string()],
        ..ConfineRequest::default()
    };
    let plan =
        confine::plan(&request, Harness::Omp, &fixture.cwd, &fixture.host()).expect("OMP plan");
    assert!(plan.writable.contains(&extra));
    let wrapped = plan.wrap(vec!["true".into()]).expect("wrap");
    assert_eq!(ro_bind_try_names(&wrapped, &target), vec!["notes.txt"]);
}

#[test]
fn a_hidden_entry_is_not_overlay_protected() {
    let (fixture, target) = symlinked_agent_with_entries_fixture();
    let request = ConfineRequest {
        hide: vec![target.join("extra").display().to_string()],
        ..ConfineRequest::default()
    };
    let wrapped = confine::plan(&request, Harness::Omp, &fixture.cwd, &fixture.host())
        .expect("OMP plan")
        .wrap(vec!["true".into()])
        .expect("wrap");
    assert_eq!(ro_bind_try_names(&wrapped, &target), vec!["notes.txt"]);
}

fn pinned_notice_plan() -> (Fixture, std::path::PathBuf, confine::Plan) {
    let (fixture, target) = symlinked_agent_fixture(false);
    let mut plan = confine::plan(
        &ConfineRequest {
            grants: vec!["docker".into()],
            keep_env: vec!["GH_TOKEN".into()],
            ..ConfineRequest::default()
        },
        Harness::Omp,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("OMP plan");
    if plan.writable.first() == Some(&plan.cwd) {
        plan.writable.rotate_left(1);
    }
    assert_ne!(plan.writable.first(), Some(&plan.cwd));
    plan.hidden.retain(|p| p.starts_with(fixture._root.path()));
    plan.skipped.retain(|p| p.starts_with(fixture._root.path()));
    (fixture, target, plan)
}

fn expected_pinned_notice(fixture: &Fixture, middle_line: &str) -> String {
    let home = fixture.home.display();
    format!(
        "This session is confined.\n\
         Writable locations: {cwd}, {home}/.omp, {target}/a.db, {target}/sessions.\n\
         Read-only protected paths: .\n\
         Hidden locations (a hidden directory looks empty; a hidden file looks empty): {home}/.aws, {home}/.ssh, {runtime}, {state}/config/secrets, {state}/run, {state}/state.\n\
         Skipped missing paths: {home}/.azure, {home}/.cache/huggingface/token, {home}/.cargo/credentials, {home}/.cargo/credentials.toml, {home}/.claude/.credentials.json, {home}/.claude/ide, {home}/.codex/auth.json, {home}/.config/antigravity, {home}/.config/chromium, {home}/.config/gcloud, {home}/.config/gh, {home}/.config/git/credentials, {home}/.config/google-chrome, {home}/.config/op, {home}/.config/rclone, {home}/.docker, {home}/.gemini/antigravity-cli/antigravity-oauth-token, {home}/.git-credentials, {home}/.gnupg, {home}/.kube, and 10 more.\n\
         {middle_line}\n\
         {home}/.omp: harness config, writable and persistent (default for this harness).\n\
         Granted: docker.\n\
         Credential environment variables are unset: GITHUB_TOKEN, GH_ENTERPRISE_TOKEN, GITLAB_TOKEN, GITLAB_PRIVATE_TOKEN, CI_JOB_TOKEN, BITBUCKET_TOKEN, NPM_TOKEN, NODE_AUTH_TOKEN, PYPI_TOKEN, TWINE_PASSWORD, CARGO_REGISTRY_TOKEN, CARGO_REGISTRIES_CRATES_IO_TOKEN, AZURE_CLIENT_SECRET, ARM_CLIENT_SECRET, VAULT_TOKEN, HF_TOKEN, HUGGING_FACE_HUB_TOKEN, SONAR_TOKEN, SENTRY_AUTH_TOKEN, DIGITALOCEAN_ACCESS_TOKEN, HCLOUD_TOKEN, CLOUDFLARE_API_TOKEN, NETLIFY_AUTH_TOKEN, VERCEL_TOKEN, FLY_API_TOKEN, HEROKU_API_KEY, PGPASSWORD, MYSQL_PWD; kept: GH_TOKEN.\n\
         Everything else is read-only. /tmp is private to this session and discarded when it ends; paths under a peer's /tmp cannot be read.\n\
         Container engines allowed: docker. Other container engines are restricted.\n\
         A Read-only file system error is authoritative where this notice and the system disagree.\n\n\
         Do not try to get around these limits, whether through other programs, services or sockets, through other agents, or by editing configuration to loosen them. If your task needs access they do not allow, stop that part and tell the person running this session which path or access you need and why. Other errors, such as a missing file or a failing build or test, are ordinary: handle them as usual.",
        cwd = fixture.cwd.display(),
        runtime = fixture.runtime.display(),
        state = fixture.state.display(),
        target = fixture._root.path().join("omp-agent").display(),
    )
}

#[test]
fn notice_text_is_pinned_with_an_overlay_parent() {
    let (fixture, target, plan) = pinned_notice_plan();
    let expected = expected_pinned_notice(
        &fixture,
        &format!(
            "{}: writable, but only the harness's data files are saved; other writes are discarded at exit.",
            target.display()
        ),
    );
    assert_eq!(confine::notice(&plan), expected);
}

#[test]
fn notice_text_is_pinned_after_the_overlays_are_dropped() {
    let (fixture, _target, plan) = pinned_notice_plan();
    let plan = plan.without_overlays("reason");
    let expected = expected_pinned_notice(
        &fixture,
        "harness login cannot be renewed in this session: reason; bubblewrap 0.11 or newer is needed (or open the harness directory with `write`).",
    );
    assert_eq!(confine::notice(&plan), expected);
}

#[test]
fn dry_run_text_is_pinned_for_an_auto_plan_with_every_section() {
    let (fixture, target) = symlinked_agent_fixture(false);
    let missing_hide = fixture.cwd.join("missing-hide");
    let mut plan = confine::plan(
        &ConfineRequest {
            grants: vec!["docker".into()],
            keep_env: vec!["GH_TOKEN".into()],
            hide: vec![missing_hide.display().to_string()],
            ..ConfineRequest::default()
        },
        Harness::Omp,
        &fixture.cwd,
        &fixture.host(),
    )
    .expect("OMP plan");
    plan.auto = true;
    plan.reasons = vec![
        a2amx::wire::GrantReason {
            grant: "docker".into(),
            reason: "Dockerfile".into(),
        },
        a2amx::wire::GrantReason {
            grant: "cargo".into(),
            reason: "Cargo.toml".into(),
        },
    ];
    plan.protected = vec![fixture.cwd.join("protected-file")];
    plan.hidden.retain(|p| p.starts_with(fixture._root.path()));
    plan.skipped.retain(|p| p.starts_with(fixture._root.path()));
    plan.skipped.truncate(1);
    let accepted = BTreeSet::from(["docker".to_owned()]);
    let home = fixture.home.display();
    let expected = format!(
        "  confinement: auto\n\
         \x20 docker: Dockerfile\n\
         \x20 cargo: Cargo.toml\n\
         \x20 not yet accepted: cargo\n\
         \x20 grants: docker\n\
         \x20 keep_env: GH_TOKEN\n\
         \x20 writable: {home}/.omp\n\
         \x20 writable: {target}/a.db\n\
         \x20 writable: {target}/sessions\n\
         \x20 protected: {cwd}/protected-file\n\
         \x20 hidden: {home}/.aws\n\
         \x20 hidden: {home}/.ssh\n\
         \x20 hidden: {runtime}\n\
         \x20 hidden: {state}/config/secrets\n\
         \x20 hidden: {state}/run\n\
         \x20 hidden: {state}/state\n\
         \x20 overlay: {target}\n\
         \x20 unresolved: {cwd}/missing-hide (hide: not found)\n\
         \x20 hidden: {home}/.azure (skipped missing)\n",
        cwd = fixture.cwd.display(),
        runtime = fixture.runtime.display(),
        state = fixture.state.display(),
        target = target.display(),
    );
    assert_eq!(
        confine::dry_run_lines(&plan, &fixture.cwd, &accepted),
        expected
    );
}
