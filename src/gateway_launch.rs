use std::env;
use std::fs;
use std::io;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail};
use rustix::io::{FdFlags, fcntl_getfd, fcntl_setfd};

const BWRAP_REQUIRED: &str = "bubblewrap is required to run the gateway; it never runs unconfined";
const DAEMON_REQUIRED: &str =
    "the a2amx daemon is not running in this a2amx directory; start it first";
const RESOLV_REQUIRED: &str = "cannot resolve /etc/resolv.conf; the gateway needs DNS";

#[derive(Debug, Clone)]
pub struct LaunchPlan {
    pub bwrap: PathBuf,
    pub layout: crate::layout::Layout,
    pub exe: PathBuf,
    pub rust_log: Option<String>,
    pub resolv_target: Option<PathBuf>,
    pub die_with_parent: bool,
}

pub fn sandbox_argv(layout: &LaunchPlan, inner: &[String]) -> Vec<String> {
    let mut argv = vec![path_string(&layout.bwrap)];
    if layout.die_with_parent {
        argv.push("--die-with-parent".into());
    }
    argv.extend([
        "--unshare-user".into(),
        "--unshare-ipc".into(),
        "--unshare-pid".into(),
    ]);
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
    let link = layout.layout.link_dir();
    let gateway = layout.layout.gateway_state_dir();
    let gateway_config = layout.layout.gateway_config();
    let discord_token = layout.layout.discord_token();
    let link = path_string(&link);
    let gateway = path_string(&gateway);
    let gateway_config = path_string(&gateway_config);
    let discord_token = path_string(&discord_token);
    argv.extend([
        "--ro-bind".into(),
        link.clone(),
        link,
        "--ro-bind".into(),
        gateway_config.clone(),
        gateway_config,
        "--ro-bind".into(),
        discord_token.clone(),
        discord_token,
        "--bind".into(),
        gateway.clone(),
        gateway.clone(),
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
    argv.extend(["--chdir".into(), gateway.clone(), "--".into()]);
    argv.extend_from_slice(inner);
    argv
}

pub fn up(home: &Path, pair: bool, foreground: bool) -> Result<()> {
    crate::layout::Layout::new(home.to_owned()).ensure()?;
    crate::config::ensure_gateway_file(&crate::layout::Layout::new(home.to_owned()))?;
    let loaded = crate::relay_files::load(home)?;
    if !pair && loaded.pairing.is_none() {
        bail!("not paired; run with --pair");
    }
    let root = fs::canonicalize(home).context("cannot resolve the a2amx directory")?;
    let layout = crate::layout::Layout::new(root.clone());
    check_link_files(&layout)?;
    let bwrap = crate::confine::resolve_bwrap(None).map_err(|_| anyhow!(BWRAP_REQUIRED))?;
    crate::confine::preflight(&bwrap).map_err(|_| anyhow!(BWRAP_REQUIRED))?;
    let resolv_target = resolve_resolv_target(Path::new("/etc/resolv.conf"))?;
    let exe = fs::canonicalize(env::current_exe()?).context("cannot resolve a2amx executable")?;
    let state_text = path_string(&root);
    let mut inner = vec![
        path_string(&exe),
        "--home".into(),
        state_text.clone(),
        "gateway".into(),
        "run".into(),
    ];
    let lock = acquire_or_explain(&root)?;
    if pair && !foreground {
        inner.push("--pair-only".into());
        let layout = LaunchPlan {
            bwrap: bwrap.clone(),
            layout: crate::layout::Layout::new(root.clone()),
            exe: exe.clone(),
            rust_log: env::var("RUST_LOG").ok(),
            resolv_target: resolv_target.clone(),
            die_with_parent: true,
        };
        let pair_argv = sandbox_argv(&layout, &inner);
        run_pair_step(&pair_argv, &root)?;
        let _ = fs::remove_file(layout.layout.gateway_pid());
        inner.pop();
        let background = LaunchPlan {
            bwrap,
            layout: crate::layout::Layout::new(root.clone()),
            exe,
            rust_log: Some(env::var("RUST_LOG").unwrap_or_else(|_| "warn".into())),
            resolv_target,
            die_with_parent: false,
        };
        let argv = sandbox_argv(&background, &inner);
        return start_background(lock, &root, &argv).map_err(|error| {
            anyhow!(
                "{error}; the pairing is saved; run a2amx gateway up (without --pair) to start the gateway"
            )
        });
    }
    if pair {
        inner.push("--pair".into());
    }
    let background = !foreground;
    let layout = LaunchPlan {
        bwrap,
        layout: crate::layout::Layout::new(root.clone()),
        exe,
        rust_log: if background {
            Some(env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()))
        } else {
            env::var("RUST_LOG").ok()
        },
        resolv_target,
        die_with_parent: !background,
    };
    let argv = sandbox_argv(&layout, &inner);
    if foreground || pair {
        exec_gateway(lock, &root, &argv)
    } else {
        start_background(lock, &root, &argv)
    }
}

fn run_pair_step(argv: &[String], root: &Path) -> Result<()> {
    let mut child = match Command::new(&argv[0]).args(&argv[1..]).spawn() {
        Ok(child) => child,
        Err(error) => bail!("pairing did not complete; the gateway was not started: {error}"),
    };
    if let Err(error) = crate::gateway_proc::write_pid(root, child.id()) {
        let _ = child.kill();
        let _ = child.wait();
        bail!("pairing did not complete; the gateway was not started: {error}");
    }
    let status = match child.wait() {
        Ok(status) => status,
        Err(error) => {
            let _ = fs::remove_file(crate::layout::Layout::new(root.to_owned()).gateway_pid());
            bail!("pairing did not complete; the gateway was not started: {error}");
        }
    };
    if !status.success() {
        let _ = fs::remove_file(crate::layout::Layout::new(root.to_owned()).gateway_pid());
        bail!("pairing did not complete; the gateway was not started");
    }
    Ok(())
}

fn acquire_or_explain(root: &Path) -> Result<crate::gateway_proc::GatewayLock> {
    match crate::gateway_proc::acquire(root) {
        Ok(lock) => Ok(lock),
        Err(crate::gateway_proc::LockError::Held) => {
            let suffix = match crate::gateway_proc::state(root)? {
                crate::gateway_proc::State::Running { pid: Some(pid) } => format!(" (pid {pid})"),
                _ => String::new(),
            };
            bail!("the gateway is already running{suffix}; stop it with: a2amx gateway stop");
        }
        Err(crate::gateway_proc::LockError::Other(error)) => Err(error),
    }
}

fn exec_gateway(
    lock: crate::gateway_proc::GatewayLock,
    root: &Path,
    argv: &[String],
) -> Result<()> {
    clear_cloexec(lock.as_fd())?;
    crate::gateway_proc::write_pid(root, std::process::id())?;
    let error = Command::new(&argv[0]).args(&argv[1..]).exec();
    Err(error.into())
}

fn start_background(
    lock: crate::gateway_proc::GatewayLock,
    root: &Path,
    argv: &[String],
) -> Result<()> {
    let layout = crate::layout::Layout::new(root.to_owned());
    let (log, offset) = crate::gateway_proc::open_log(root)?;
    clear_cloexec(lock.as_fd())?;
    let stdout = log.try_clone()?;
    let stderr = log.try_clone()?;
    let mut command = Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr);
    // SAFETY: the closure only calls setsid, which is async-signal-safe.
    unsafe {
        command.pre_exec(|| {
            rustix::process::setsid()
                .map(|_| ())
                .map_err(io::Error::from)
        });
    }
    let mut child = command.spawn().context("cannot start the gateway")?;
    let pid = child.id();
    crate::gateway_proc::write_pid(root, pid)?;
    wait_until_ready(&mut child, &log, offset, root)?;
    println!(
        "gateway running in the background (pid {pid}); log: {}; stop it with: a2amx gateway stop",
        layout.gateway_log().display()
    );
    drop(lock);
    Ok(())
}

fn wait_until_ready(
    child: &mut std::process::Child,
    log: &std::fs::File,
    offset: u64,
    root: &Path,
) -> Result<()> {
    let log_path = crate::layout::Layout::new(root.to_owned()).gateway_log();
    let deadline = Instant::now() + crate::gateway_proc::READY_WAIT;
    loop {
        if let Some(status) = child.try_wait()? {
            let progress = crate::gateway_proc::log_progress(log, offset)?;
            let _ = fs::remove_file(crate::layout::Layout::new(root.to_owned()).gateway_pid());
            bail!(
                "the gateway exited during startup ({status}); log: {}\n{}",
                log_path.display(),
                progress.tail
            );
        }
        let progress = crate::gateway_proc::log_progress(log, offset)?;
        if progress.ready {
            return Ok(());
        }
        if Instant::now() >= deadline {
            // shortcut: a slow start is reported, not reaped; add a kill if a half-started gateway ever lingers.
            bail!(
                "the gateway did not become ready within 10s; log: {} (it may still start; check with: a2amx gateway status)",
                log_path.display()
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

fn clear_cloexec(fd: std::os::fd::BorrowedFd<'_>) -> Result<()> {
    let flags = fcntl_getfd(fd)?;
    fcntl_setfd(fd, flags & !FdFlags::CLOEXEC)?;
    Ok(())
}

fn check_link_files(layout: &crate::layout::Layout) -> Result<()> {
    for path in [layout.link_addr(), layout.link_gateway_token()] {
        let metadata = fs::symlink_metadata(&path).map_err(|_| anyhow!(DAEMON_REQUIRED))?;
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
