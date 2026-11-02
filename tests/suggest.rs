use std::process::Command;

use tempfile::tempdir;

use a2amx::suggest;

#[test]
fn detect_finds_cargo_marker() {
    assert_eq!(suggest::detect(|name| name == "Cargo.toml"), vec!["cargo"]);
}

#[test]
fn detect_follows_grant_table_order() {
    assert_eq!(
        suggest::detect(|name| matches!(name, "go.mod" | "package.json")),
        vec!["go", "node"]
    );
}

#[test]
fn detect_supports_each_java_marker() {
    for marker in [
        "pom.xml",
        "build.gradle",
        "build.gradle.kts",
        "settings.gradle",
        "settings.gradle.kts",
    ] {
        assert_eq!(suggest::detect(|name| name == marker), vec!["java"]);
    }
}

#[test]
fn detect_finds_analyzers_and_nothing_else() {
    assert_eq!(
        suggest::detect(|name| matches!(name, ".semgrep" | "sonar-project.properties")),
        vec!["semgrep", "sonar"]
    );
    assert!(suggest::detect(|_| false).is_empty());
}

#[test]
fn ssh_remote_forms_are_detected_once() {
    for value in [
        "git@host-a:team/repo.git",
        "gitea@host-a:x.git",
        "host-a:repo",
        "ssh://host-a/repo",
        "git+ssh://host-a/repo",
    ] {
        let config = format!("[remote \"origin\"]\n\turl = {value}\n");
        assert_eq!(
            suggest::git_remote_grants(&config, |_| false),
            vec!["ssh"],
            "{value}"
        );
    }
}

#[test]
fn https_remote_grants_follow_available_credentials() {
    let config = "[remote \"origin\"]\nurl = \"https://example.invalid/a.git\"\n";
    assert_eq!(
        suggest::git_remote_grants(config, |path| {
            matches!(path, "~/.config/gh" | "~/.git-credentials")
        }),
        vec!["gh", "git-credentials"]
    );
    assert_eq!(
        suggest::git_remote_grants(config, |path| path == "~/.config/gh"),
        vec!["gh"]
    );
    assert_eq!(
        suggest::git_remote_grants(config, |path| path == "~/.config/git/credentials"),
        vec!["git-credentials"]
    );
    assert!(suggest::git_remote_grants(config, |_| false).is_empty());
}

#[test]
fn git_remote_parser_counts_pushurl_and_ignores_other_sections() {
    let config = "[remote \"origin\"]\nurl=git@host-a:x.git\npushurl = https://example.invalid/x.git\n\
[url \"https://example.invalid/\"]\ninsteadOf = host-a:\n";
    assert_eq!(
        suggest::git_remote_grants(config, |path| path == "~/.config/gh"),
        vec!["gh", "ssh"]
    );
    for value in ["http://example.invalid/x", "file:///tmp/repo", "/tmp/repo"] {
        let config = format!("[remote \"origin\"]\nurl = {value}\n");
        assert!(suggest::git_remote_grants(&config, |_| true).is_empty());
    }
}

#[test]
fn git_remote_parser_accepts_mixed_case_url_keys() {
    let config = "[remote \"origin\"]\nURL = git@host-a:team/demo.git\nPushURL = https://example.invalid/team/demo.git\n";
    assert_eq!(
        suggest::git_remote_grants(config, |path| path == "~/.config/gh"),
        vec!["gh", "ssh"]
    );
}

#[test]
fn grants_adds_cache_and_deduplicates_in_table_order() {
    let config = "[remote \"origin\"]\nurl = git@host-a:repo.git\n";
    assert_eq!(
        suggest::grants(
            |name| matches!(name, "Cargo.toml" | "package.json"),
            Some(config),
            |_| false
        ),
        vec!["ssh", "cargo", "node", "cache"]
    );
}

#[test]
fn render_cargo_and_cache_is_a_single_grants_line() {
    assert_eq!(
        suggest::render(&["cargo", "cache"]),
        "# Suggested grants:\n\
grants = [\"cargo\", \"cache\"]\n"
    );
}

#[test]
fn render_empty_is_a_single_empty_grants_line() {
    assert_eq!(
        suggest::render(&[]),
        "# Suggested grants:\n\
grants = []\n"
    );
}

#[test]
fn rendered_grants_parse_as_a_team_confine_fragment() {
    let fragment = suggest::render(&["cargo", "cache"]);
    let team_file = format!(
        "team = \"x\"\n\
[confine]\n\
enabled = true\n\
[[session]]\n\
name = \"x\"\n\
command = [\"cat\"]\n\
[session.confine]\n\
{fragment}"
    );
    a2amx::team::parse(&team_file).expect("rendered grants parse");
}

#[test]
fn binary_suggests_the_fragment_for_files_in_an_explicit_directory() {
    let directory = tempdir().expect("temporary project directory");
    std::fs::write(directory.path().join("Cargo.toml"), "").expect("cargo marker");
    std::fs::write(directory.path().join("pom.xml"), "").expect("maven marker");
    let output = Command::new(env!("CARGO_BIN_EXE_a2amx"))
        .env_clear()
        .args(["confine", "suggest"])
        .arg(directory.path())
        .output()
        .expect("run confine suggest");
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{:?}", output.stderr);
    assert_eq!(
        output.stdout,
        suggest::render(&["cargo", "java", "cache"]).as_bytes()
    );
}

#[test]
fn binary_rejects_missing_and_non_directory_project_paths() {
    let root = tempdir().expect("temporary root");
    let missing = root.path().join("missing");
    let file = root.path().join("not-a-directory");
    std::fs::write(&file, "").expect("file marker");
    for path in [&missing, &file] {
        let output = Command::new(env!("CARGO_BIN_EXE_a2amx"))
            .env_clear()
            .args(["confine", "suggest"])
            .arg(path)
            .output()
            .expect("run confine suggest");
        assert!(!output.status.success(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("cannot read project directory"),
            "{:?}",
            output.stderr
        );
    }
}

#[test]
fn binary_suggests_from_the_current_directory_when_dir_is_omitted() {
    let directory = tempdir().expect("temporary project directory");
    std::fs::write(directory.path().join("package.json"), "").expect("node marker");
    let output = Command::new(env!("CARGO_BIN_EXE_a2amx"))
        .env_clear()
        .current_dir(directory.path())
        .args(["confine", "suggest"])
        .output()
        .expect("run confine suggest");
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{:?}", output.stderr);
    assert_eq!(
        output.stdout,
        suggest::render(&["node", "cache"]).as_bytes()
    );
}
#[test]
fn binary_suggests_static_text_for_a_directory_with_newline_and_hash() {
    let root = tempdir().expect("temporary root");
    let directory = root.path().join("line\n# injected");
    std::fs::create_dir(&directory).expect("project directory");
    std::fs::write(directory.join("Cargo.toml"), "").expect("cargo marker");
    let output = Command::new(env!("CARGO_BIN_EXE_a2amx"))
        .env_clear()
        .args(["confine", "suggest"])
        .arg(&directory)
        .output()
        .expect("run confine suggest");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        output.stdout,
        suggest::render(&["cargo", "cache"]).as_bytes()
    );
}
