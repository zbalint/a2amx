use a2amx::harness::Harness;

fn infer(command: &[&str]) -> Harness {
    let command: Vec<String> = command.iter().map(|word| (*word).to_owned()).collect();
    Harness::infer(&command)
}

#[test]
fn names_a_harness_by_executable_file_name() {
    assert_eq!(infer(&["claude"]), Harness::Claude);
    assert_eq!(infer(&["/usr/bin/claude", "--resume"]), Harness::Claude);
    assert_eq!(infer(&["./codex"]), Harness::Codex);
    assert_eq!(infer(&["omp"]), Harness::Omp);
}

#[test]
fn anything_else_is_generic() {
    assert_eq!(infer(&["sh"]), Harness::Generic);
    assert_eq!(infer(&["Claude"]), Harness::Generic);
    assert_eq!(infer(&["claude-code"]), Harness::Generic);
    assert_eq!(infer(&["env", "FOO=1", "claude"]), Harness::Generic);
    assert_eq!(infer(&["/"]), Harness::Generic);
    assert_eq!(infer(&[]), Harness::Generic);
}
