use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn git_output(manifest_dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(manifest_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_owned())
    }
}

fn git_path(manifest_dir: &Path, name: &str) -> Option<PathBuf> {
    let path = PathBuf::from(git_output(
        manifest_dir,
        &["rev-parse", "--git-path", name],
    )?);
    Some(if path.is_absolute() {
        path
    } else {
        manifest_dir.join(path)
    })
}

fn print_existing_rerun_path(path: &Path) {
    if path.exists() {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

fn main() {
    let manifest_dir = match env::var_os("CARGO_MANIFEST_DIR") {
        Some(value) => PathBuf::from(value),
        None => PathBuf::from("."),
    };
    let version = match git_output(
        &manifest_dir,
        &["describe", "--tags", "--always", "--dirty"],
    ) {
        Some(version) => version,
        None => match env::var("CARGO_PKG_VERSION") {
            Ok(version) if !version.is_empty() => version,
            _ => "0.0.0".to_owned(),
        },
    };
    println!("cargo:rustc-env=A2AMX_VERSION={version}");

    print_existing_rerun_path(&manifest_dir.join("build.rs"));
    print_existing_rerun_path(&manifest_dir.join("Cargo.toml"));
    print_existing_rerun_path(&manifest_dir.join("src"));

    // shortcut: -dirty tracks src, Cargo.toml and git state, not edits to tests or docs (ceiling),
    // and the index changing on a plain git status can trigger a harmless rebuild; upgrade trigger:
    // report if a stale or flapping version is seen.
    if git_output(&manifest_dir, &["--version"]).is_none() {
        return;
    }
    for name in ["HEAD", "index", "logs/HEAD", "packed-refs"] {
        if let Some(path) = git_path(&manifest_dir, name) {
            print_existing_rerun_path(&path);
        }
    }
    if let Some(reference) = git_output(&manifest_dir, &["symbolic-ref", "-q", "HEAD"])
        && let Some(path) = git_path(&manifest_dir, &reference)
    {
        print_existing_rerun_path(&path);
    }
}
