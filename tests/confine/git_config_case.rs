use super::*;

fn plan_with_hooks_config(header: &str, key: &str) -> (confine::Plan, PathBuf) {
    let fixture = Fixture::new();
    let repo = fixture.cwd.join("repo");
    let git = repo.join(".git");
    let husky = repo.join(".husky");
    std::fs::create_dir_all(git.join("hooks")).expect("git hooks");
    std::fs::create_dir_all(&husky).expect("husky hooks");
    std::fs::write(git.join("config"), format!("{header}\n{key} = .husky\n")).expect("git config");
    let plan = confine::plan(
        &ConfineRequest::default(),
        Harness::Generic,
        &repo,
        &fixture.host(),
    )
    .expect("plan");
    (plan, std::fs::canonicalize(husky).expect("husky canonical"))
}

#[test]
fn hooks_path_accepts_a_capitalised_core_section() {
    let (lower_plan, lower_husky) = plan_with_hooks_config("[core]", "hooksPath");
    let (capital_plan, capital_husky) = plan_with_hooks_config("[Core]", "hooksPath");
    assert!(lower_plan.protected.contains(&lower_husky));
    assert!(capital_plan.protected.contains(&capital_husky));
}

#[test]
fn hooks_path_accepts_a_lowercase_hookspath_key() {
    let (plan, husky) = plan_with_hooks_config("[core]", "hookspath");
    assert!(plan.protected.contains(&husky));
}
