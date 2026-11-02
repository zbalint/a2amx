use std::process::Command;

use tempfile::tempdir;

use a2amx::suggest::{self, Ecosystem};

#[test]
fn detect_reports_markers_in_fixed_order_and_uses_first_marker() {
    assert_eq!(
        suggest::detect(|name| name == "Cargo.toml"),
        vec![(Ecosystem::Rust, "Cargo.toml")]
    );
    assert_eq!(
        suggest::detect(|name| name == "build.gradle.kts"),
        vec![(Ecosystem::Gradle, "build.gradle.kts")]
    );
    assert_eq!(
        suggest::detect(|name| matches!(name, "build.gradle" | "build.gradle.kts")),
        vec![(Ecosystem::Gradle, "build.gradle")]
    );
    assert_eq!(
        suggest::detect(|name| matches!(name, "Cargo.toml" | "package.json" | "pom.xml")),
        vec![
            (Ecosystem::Rust, "Cargo.toml"),
            (Ecosystem::Node, "package.json"),
            (Ecosystem::Maven, "pom.xml"),
        ]
    );
    assert!(suggest::detect(|_| false).is_empty());
}

#[test]
fn default_write_paths_put_common_cache_before_detected_paths() {
    let found = [(Ecosystem::Rust, "Cargo.toml")];
    assert_eq!(
        suggest::default_write_paths(&found),
        vec!["~/.cache", "~/.cargo"]
    );
}

#[test]
fn write_paths_return_the_static_cache_union_in_table_order() {
    assert_eq!(
        suggest::write_paths(&[
            (Ecosystem::Rust, "Cargo.toml"),
            (Ecosystem::Node, "package.json"),
            (Ecosystem::Gradle, "build.gradle"),
        ]),
        vec![
            "~/.cargo",
            "~/.npm",
            "~/.gradle/caches",
            "~/.gradle/wrapper"
        ]
    );
}

#[test]
fn render_rust_and_node_is_the_documented_fragment() {
    let found = [
        (Ecosystem::Rust, "Cargo.toml"),
        (Ecosystem::Node, "package.json"),
    ];
    assert_eq!(
        suggest::render(&found),
        "# Found: Cargo.toml (Rust), package.json (Node)\n\
# Add this to the session's table in your team file, or pass --confine-write for each path. Confinement must be enabled (enabled = true here or in the team's [confine] table).\n\
[session.confine]\n\
write = [\"~/.cargo\", \"~/.npm\"]\n\
# Rust: a Git dependency also needs ~/.cargo/git (untested).\n\
# Node: global installs and version-manager directories are not covered.\n\
# More: docs/confinement.md, Recipes by ecosystem\n"
    );
}

#[test]
fn render_gradle_includes_both_write_paths() {
    assert_eq!(
        suggest::render(&[(Ecosystem::Gradle, "build.gradle")]),
        "# Found: build.gradle (Gradle)\n\
# Add this to the session's table in your team file, or pass --confine-write for each path. Confinement must be enabled (enabled = true here or in the team's [confine] table).\n\
[session.confine]\n\
write = [\"~/.gradle/caches\", \"~/.gradle/wrapper\"]\n\
# Gradle: gradle.properties and init.d stay closed; do not open all of ~/.gradle.\n\
# More: docs/confinement.md, Recipes by ecosystem\n"
    );
}

#[test]
fn render_go_and_python_without_write_paths_has_notes_only() {
    let found = [
        (Ecosystem::Go, "go.mod"),
        (Ecosystem::Python, "pyproject.toml"),
    ];
    let rendered = suggest::render(&found);
    assert_eq!(
        rendered,
        "# Found: go.mod (Go), pyproject.toml (Python)\n\
# Go: run go env GOMODCACHE GOCACHE and add both paths as write entries; do not open ~/go/bin.\n\
# Python: a virtual environment inside the workspace needs no write entry.\n\
# More: docs/confinement.md, Recipes by ecosystem\n"
    );
    assert!(!rendered.contains("[session.confine]"));
    assert!(!rendered.contains("write ="));
    assert!(!rendered.contains("Add this to"));
}

#[test]
fn render_empty_has_the_no_known_build_files_message() {
    assert_eq!(
        suggest::render(&[]),
        "# No known build files here. The workspace itself needs no write entry.\n\
# Run the tool once, read the failing path, and see docs/confinement.md, Recipes by ecosystem.\n"
    );
}

#[test]
fn rendered_all_ecosystems_parse_as_a_session_confine_fragment() {
    let fragment = suggest::render(&[
        (Ecosystem::Rust, "Cargo.toml"),
        (Ecosystem::Node, "package.json"),
        (Ecosystem::Go, "go.mod"),
        (Ecosystem::Maven, "pom.xml"),
        (Ecosystem::Gradle, "build.gradle"),
        (Ecosystem::Python, "pyproject.toml"),
    ]);
    let team_file = format!(
        "team = \"x\"\n\
[confine]\n\
enabled = true\n\
[[session]]\n\
name = \"x\"\n\
command = [\"cat\"]\n\
{fragment}"
    );
    a2amx::team::parse(&team_file).expect("rendered fragment parses");
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
        suggest::render(&[
            (Ecosystem::Rust, "Cargo.toml"),
            (Ecosystem::Maven, "pom.xml"),
        ])
        .as_bytes()
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
        suggest::render(&[(Ecosystem::Node, "package.json")]).as_bytes()
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
        suggest::render(&[(Ecosystem::Rust, "Cargo.toml")]).as_bytes()
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("injected"));
}
