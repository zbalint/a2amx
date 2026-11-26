use std::collections::BTreeSet;
use std::os::unix::fs::symlink;

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
