use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};

use super::{canonical_existing, path_key};

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

fn stderr_suffix(child: &mut Child) -> String {
    let Some(mut stderr) = child.stderr.take() else {
        return String::new();
    };
    use std::io::Read;
    let mut bytes = Vec::new();
    stderr.read_to_end(&mut bytes).ok();
    if bytes.is_empty() {
        return String::new();
    }
    format!(": {}", String::from_utf8_lossy(&bytes))
}

fn exit_result(path: &Path, status: ExitStatus, child: &mut Child) -> Result<()> {
    if status.success() {
        return Ok(());
    }
    bail!(
        "{} exited with {status}{}",
        path.display(),
        stderr_suffix(child)
    );
}

fn command_with_timeout(path: &Path, args: &[String]) -> Result<()> {
    let mut child = Command::new(path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("cannot run {}", path.display()))?;
    let deadline = Instant::now() + PROBE_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait()? {
            return exit_result(path, status, &mut child);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "{} timed out after {} seconds",
                path.display(),
                PROBE_TIMEOUT.as_secs()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Find the first executable named `name` in absolute entries of a PATH value.
pub fn find_executable(name: &str, path_var: &OsStr) -> Option<PathBuf> {
    for entry in std::env::split_paths(path_var) {
        if !entry.is_absolute() {
            continue;
        }
        let candidate = entry.join(name);
        let Ok(metadata) = fs::metadata(&candidate) else {
            continue;
        };
        if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
            continue;
        }
        if let Ok(path) = fs::canonicalize(candidate) {
            return Some(path);
        }
    }
    None
}

fn search_path_value(path_var: Option<&OsStr>) -> Result<PathBuf> {
    let path_var = path_var
        .map(OsStr::to_os_string)
        .or_else(|| std::env::var_os("PATH"))
        .ok_or_else(|| anyhow!("PATH is not set; cannot find bubblewrap"))?;
    let path = PathBuf::from(path_var);
    if path.as_os_str().is_empty() {
        bail!("PATH is empty; cannot find bubblewrap");
    }
    Ok(path)
}

/// Resolve a bwrap executable from a daemon-owned PATH-style value.
pub fn resolve_bwrap(path_var: Option<&OsStr>) -> Result<PathBuf> {
    let path = search_path_value(path_var)?;
    if path.is_absolute() && path.is_file() {
        return fs::canonicalize(&path).context("canonicalizing bwrap path");
    }
    let candidate_name = if path.is_file() {
        path.clone()
    } else {
        PathBuf::from("bwrap")
    };
    if candidate_name.is_absolute() {
        return fs::canonicalize(&candidate_name).context("canonicalizing bwrap path");
    }
    let Some(candidate_name) = candidate_name.to_str() else {
        bail!("bubblewrap was not found in PATH");
    };
    if let Some(path) = find_executable(candidate_name, path.as_os_str()) {
        return Ok(path);
    }
    bail!("bubblewrap was not found in PATH")
}

fn preflight_args() -> Vec<String> {
    vec![
        "--die-with-parent".into(),
        "--unshare-user".into(),
        "--unshare-ipc".into(),
        "--unshare-pid".into(),
        "--ro-bind".into(),
        "/".into(),
        "/".into(),
        "--dev".into(),
        "/dev".into(),
        "--proc".into(),
        "/proc".into(),
        "--size".into(),
        "1073741824".into(),
        "--tmpfs".into(),
        "/tmp".into(),
        "--".into(),
        "true".into(),
    ]
}

/// Run version and a real base-mount probe.  A failure is fatal; callers must
/// not fall back to an unwrapped PTY after this returns an error.
pub fn preflight(bwrap: &Path) -> Result<()> {
    let bwrap = canonical_existing(bwrap, "bubblewrap")?;
    command_with_timeout(&bwrap, &["--version".into()])?;
    command_with_timeout(&bwrap, &preflight_args())
}

/// Check whether bubblewrap supports the temporary overlays used for harness files.
pub fn supports_tmp_overlay(bwrap: &Path, parents: &[PathBuf]) -> Result<(), String> {
    if parents.is_empty() {
        return Ok(());
    }
    let bwrap = canonical_existing(bwrap, "bubblewrap").map_err(|error| error.to_string())?;
    let mut probe = preflight_args();
    let Some(mut separator) = probe.iter().position(|argument| argument == "--") else {
        return Err("invalid preflight probe".into());
    };
    for parent in parents {
        probe.splice(
            separator..separator,
            [
                "--overlay-src".to_owned(),
                path_key(parent),
                "--tmp-overlay".to_owned(),
                path_key(parent),
            ],
        );
        separator += 4;
    }
    command_with_timeout(&bwrap, &probe).map_err(|error| {
        let text = error.to_string();
        text.split_once(": ")
            .map(|(_, stderr)| stderr.lines().next().unwrap_or(stderr))
            .unwrap_or_else(|| {
                text.lines()
                    .next()
                    .unwrap_or("bubblewrap overlay probe failed")
            })
            .to_owned()
    })
}

/// Verify that `env` supports the signal-disposition flags used for Generic sessions.
pub fn preflight_env(env: &Path) -> Result<()> {
    let env_path = env.to_string_lossy().into_owned();
    let probe = vec![
        "--ignore-signal=INT".into(),
        env_path.clone(),
        "--default-signal=INT".into(),
        env_path,
    ];
    let requirement = "confinement of a Generic session needs env with --ignore-signal and --default-signal: \
         GNU coreutils 8.31 or newer, or uutils";
    command_with_timeout(env, &probe).map_err(|error| anyhow!("{requirement}: {error:#}"))
}

/// Create the host-side empty regular file used for hidden files.
pub fn ensure_empty_file(layout: &crate::layout::Layout) -> Result<PathBuf> {
    let path = layout.confine_empty();
    if !path.exists() {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o400)
            .open(&path)?;
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o400))?;
    Ok(path)
}
