use std::path::Path;
use std::process::{Command, Stdio};

#[test]
fn omp_extension_behavior_tests_pass() {
    const MIN_TESTS: usize = 17;
    let version = Command::new("node").arg("--version").output();
    let major = version
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| {
            String::from_utf8_lossy(&output.stdout)
                .trim()
                .trim_start_matches('v')
                .split('.')
                .next()
                .and_then(|major| major.parse::<u32>().ok())
        });
    if !major.is_some_and(|major| major >= 22) {
        assert_ne!(
            std::env::var("A2AMX_REQUIRE_NODE").as_deref(),
            Ok("1"),
            "OMP extension behavior tests require node >=22 on PATH; found major {major:?}"
        );
        eprintln!("skipped OMP extension behavior tests: node >=22 is missing on PATH");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let extension = dir.path().join("extension.mjs");
    std::fs::write(&extension, a2amx::omp::render_extension().unwrap()).unwrap();
    let mut node = Command::new("node");
    node.args([
        "--test",
        "--test-reporter=tap",
        "--test-reporter-destination=stdout",
    ]);
    // The coverage run (CI, scripts/prepush-sonar.sh) sets A2AMX_JS_LCOV to a file path. Node then
    // writes an lcov report of the rendered extension, which the caller maps back to
    // extension/omp.ts: rendering only replaces text inside line 5, so line numbers are unchanged.
    let lcov = std::env::var_os("A2AMX_JS_LCOV")
        .map(|path| Path::new(env!("CARGO_MANIFEST_DIR")).join(path));
    if let Some(lcov) = &lcov {
        node.args([
            "--experimental-test-coverage",
            "--experimental-test-isolation=none",
        ])
        .arg(format!("--test-coverage-include={}", extension.display()))
        .args(["--test-reporter=lcov", "--test-reporter-destination"])
        .arg(lcov);
    }
    let output = node
        .arg("tests/omp_extension/*.test.mjs")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("A2AMX_TEST_EXTENSION", &extension)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    let passed = stdout.lines().find_map(|line| {
        line.strip_prefix("# pass ")
            .and_then(|count| count.parse::<usize>().ok())
    });
    assert!(
        passed.is_some_and(|count| count >= MIN_TESTS),
        "expected at least {MIN_TESTS} Node tests; {stdout}\n{stderr}"
    );
    if let Some(lcov) = lcov {
        let report = std::fs::read_to_string(&lcov).unwrap();
        let mapped: String = report
            .lines()
            .map(|line| {
                if line.starts_with("SF:") {
                    "SF:extension/omp.ts"
                } else {
                    line
                }
            })
            .flat_map(|line| [line, "\n"])
            .collect();
        std::fs::write(&lcov, mapped).unwrap();
    }
    println!("{stdout}");
}
