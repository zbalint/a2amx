use std::env;
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};

const BWRAP_REQUIRED: &str = "bubblewrap is required to run the gateway; it never runs unconfined";
const DAEMON_REQUIRED: &str =
    "the a2amx daemon is not running in this state directory; start it first";
const RESOLV_REQUIRED: &str = "cannot resolve /etc/resolv.conf; the gateway needs DNS";

#[derive(Debug, Clone)]
pub struct Layout {
    pub bwrap: PathBuf,
    pub state_dir: PathBuf,
    pub exe: PathBuf,
    pub rust_log: Option<String>,
    pub resolv_target: Option<PathBuf>,
}

pub fn sandbox_argv(layout: &Layout, inner: &[String]) -> Vec<String> {
    let mut argv = vec![
        path_string(&layout.bwrap),
        "--die-with-parent".into(),
        "--unshare-user".into(),
        "--unshare-ipc".into(),
        "--unshare-pid".into(),
    ];
    argv.extend(["--tmpfs".into(), "/".into()]);
    push_bind(&mut argv, "--ro-bind", Path::new("/usr"), Path::new("/usr"));
    for path in ["/bin", "/sbin", "/lib", "/lib64"] {
        push_bind(&mut argv, "--ro-bind-try", Path::new(path), Path::new(path));
    }
    push_bind(&mut argv, "--ro-bind", Path::new("/etc"), Path::new("/etc"));
    if let Some(path) = &layout.resolv_target {
        let path = path_string(path);
        argv.extend(["--ro-bind".into(), path.clone(), path]);
    }
    argv.extend([
        "--proc".into(),
        "/proc".into(),
        "--dev".into(),
        "/dev".into(),
        "--size".into(),
        "1073741824".into(),
        "--tmpfs".into(),
        "/tmp".into(),
    ]);
    let exe = path_string(&layout.exe);
    push_bind(&mut argv, "--ro-bind", Path::new(&exe), Path::new(&exe));
    let link = layout.state_dir.join("link");
    let gateway = layout.state_dir.join("gateway");
    let link = path_string(&link);
    let gateway = path_string(&gateway);
    argv.extend([
        "--ro-bind".into(),
        link.clone(),
        link,
        "--bind".into(),
        gateway.clone(),
        gateway,
        "--remount-ro".into(),
        "/".into(),
        "--clearenv".into(),
        "--setenv".into(),
        "PATH".into(),
        "/usr/local/bin:/usr/bin:/bin".into(),
    ]);
    if let Some(value) = &layout.rust_log {
        argv.extend(["--setenv".into(), "RUST_LOG".into(), value.clone()]);
    }
    argv.extend([
        "--chdir".into(),
        path_string(&layout.state_dir.join("gateway")),
        "--".into(),
    ]);
    argv.extend_from_slice(inner);
    argv
}

pub fn up(home: &Path, pair: bool) -> Result<()> {
    let loaded = crate::relay_files::load(home)?;
    if !pair && loaded.pairing.is_none() {
        bail!("not paired; run with --pair");
    }
    let state_dir = fs::canonicalize(home).context("cannot resolve state directory")?;
    check_link_files(&state_dir)?;
    let bwrap = crate::confine::resolve_bwrap(None).map_err(|_| anyhow!(BWRAP_REQUIRED))?;
    crate::confine::preflight(&bwrap).map_err(|_| anyhow!(BWRAP_REQUIRED))?;
    let resolv_target = resolve_resolv_target(Path::new("/etc/resolv.conf"))?;
    let exe = fs::canonicalize(env::current_exe()?).context("cannot resolve a2amx executable")?;
    let state_text = path_string(&state_dir);
    let exe_text = path_string(&exe);
    let mut inner = vec![
        exe_text,
        "--home".into(),
        state_text,
        "gateway".into(),
        "run".into(),
    ];
    if pair {
        inner.push("--pair".into());
    }
    let layout = Layout {
        bwrap,
        state_dir,
        exe,
        rust_log: env::var("RUST_LOG").ok(),
        resolv_target,
    };
    let argv = sandbox_argv(&layout, &inner);
    let error = Command::new(&argv[0]).args(&argv[1..]).exec();
    Err(error.into())
}

fn check_link_files(state_dir: &Path) -> Result<()> {
    let link_dir = state_dir.join("link");
    let directory = fs::symlink_metadata(&link_dir).map_err(|_| anyhow!(DAEMON_REQUIRED))?;
    if !directory.file_type().is_dir() {
        bail!(DAEMON_REQUIRED);
    }
    for name in ["addr", "gateway.token"] {
        let metadata =
            fs::symlink_metadata(link_dir.join(name)).map_err(|_| anyhow!(DAEMON_REQUIRED))?;
        if !metadata.file_type().is_file() {
            bail!(DAEMON_REQUIRED);
        }
    }
    Ok(())
}

fn resolve_resolv_target(path: &Path) -> Result<Option<PathBuf>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => bail!(RESOLV_REQUIRED),
    };
    if !metadata.file_type().is_symlink() {
        return Ok(None);
    }
    let target = fs::canonicalize(path).map_err(|_| anyhow!(RESOLV_REQUIRED))?;
    let target_metadata = fs::metadata(&target).map_err(|_| anyhow!(RESOLV_REQUIRED))?;
    if !target_metadata.file_type().is_file() {
        bail!(RESOLV_REQUIRED);
    }
    if target.starts_with("/etc") || target.starts_with("/usr") {
        Ok(None)
    } else {
        Ok(Some(target))
    }
}

fn push_bind(argv: &mut Vec<String>, option: &str, source: &Path, target: &Path) {
    argv.extend([option.into(), path_string(source), path_string(target)]);
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::{RESOLV_REQUIRED, resolve_resolv_target};
    use std::os::unix::fs::symlink;

    #[test]
    fn resolve_resolv_target_classifies_regular_and_safe_links() {
        let dir = tempfile::tempdir().unwrap();
        let regular = dir.path().join("regular");
        std::fs::write(&regular, b"nameserver 127.0.0.1\n").unwrap();
        assert_eq!(resolve_resolv_target(&regular).unwrap(), None);

        let under_etc = dir.path().join("under-etc");
        symlink("/etc/hosts", &under_etc).unwrap();
        assert_eq!(resolve_resolv_target(&under_etc).unwrap(), None);

        let external_dir = dir.path().join("mnt");
        std::fs::create_dir(&external_dir).unwrap();
        let external_target = external_dir.join("resolv.conf");
        std::fs::write(&external_target, b"nameserver 127.0.0.1\n").unwrap();
        let external = dir.path().join("external");
        symlink(&external_target, &external).unwrap();
        assert_eq!(
            resolve_resolv_target(&external).unwrap(),
            Some(std::fs::canonicalize(external_target).unwrap())
        );
    }

    #[test]
    fn resolve_resolv_target_rejects_dangling_links() {
        let dir = tempfile::tempdir().unwrap();
        let dangling = dir.path().join("dangling");
        symlink("missing", &dangling).unwrap();
        assert_eq!(
            resolve_resolv_target(&dangling).unwrap_err().to_string(),
            RESOLV_REQUIRED
        );
    }
}
