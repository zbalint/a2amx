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
