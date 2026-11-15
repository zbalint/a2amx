//! Embedded `agy` plugin and its launch-time installation.

use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions, Permissions};
use std::io::{self, Read};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

pub const PLUGIN: &str = "a2amx";
pub const SERVER: &str = "tools";
const MAX_STDERR: usize = 2 * 1024;
const MAX_STDOUT: usize = 64 * 1024;
const MAX_FILE: usize = 64 * 1024;

pub fn tool_rule() -> String {
    format!("mcp({PLUGIN}_{SERVER}/*)")
}

fn manifest() -> String {
    format!(
        "{{\"name\":\"{PLUGIN}\",\"version\":\"{}\",\"description\":\"a2amx messaging tools for agents started by a2amx\"}}",
        env!("CARGO_PKG_VERSION")
    )
}

fn mcp_config(binary: &Path) -> Result<String> {
    Ok(serde_json::json!({
        "mcpServers": {
            SERVER: {
                "command": binary,
                "args": ["mcp", "--optional"]
            }
        }
    })
    .to_string())
}

pub fn install(layout: &crate::layout::Layout, agy_program: &OsStr) -> Result<()> {
    install_with_timeout(layout, agy_program, Duration::from_secs(30))
}

pub fn install_with_timeout(
    layout: &crate::layout::Layout,
    agy_program: &OsStr,
    timeout: Duration,
) -> Result<()> {
    let binary = layout.installed_binary();
    let metadata = fs::metadata(&binary).with_context(|| {
        format!(
            "a2amx is not installed at {}; the agy plugin needs the installed binary there",
            binary.display()
        )
    })?;
    if !metadata.is_file() {
        bail!(
            "a2amx is not installed at {}; the agy plugin needs the installed binary there",
            binary.display()
        );
    }
    let program = resolve_program(agy_program)?;
    let support = run_command_clear(
        &binary,
        &[OsString::from("mcp"), OsString::from("--optional")],
        timeout.min(Duration::from_secs(5)),
        MAX_STDOUT,
        MAX_STDERR,
    )?;
    if !support.status.success() {
        bail!(
            "the installed a2amx at {} does not support mcp --optional; reinstall it",
            binary.display()
        );
    }

    let agy_dir = layout.agy_dir();
    fs::create_dir_all(&agy_dir)?;
    fs::set_permissions(&agy_dir, Permissions::from_mode(0o700))?;
    let lock_path = agy_dir.join("install.lock");
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&lock_path)?;
    fs::set_permissions(&lock_path, Permissions::from_mode(0o600))?;
    lock.lock()?;

    let plugin = agy_dir.join("plugin");
    fs::create_dir_all(&plugin)?;
    fs::set_permissions(&plugin, Permissions::from_mode(0o700))?;
    for entry in fs::read_dir(&plugin)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with(".install-") {
            let path = entry.path();
            if entry.file_type()?.is_file() {
                fs::remove_file(path)?;
            }
        }
    }
    let manifest_path = plugin.join("plugin.json");
    let config_path = plugin.join("mcp_config.json");
    crate::omp::write_if_changed(&manifest_path, manifest().as_bytes())?;
    crate::omp::write_if_changed(&config_path, mcp_config(&binary)?.as_bytes())?;

    let validated = run_command(
        &program,
        &[
            OsString::from("plugin"),
            OsString::from("validate"),
            plugin.clone().into_os_string(),
            OsString::from("--strict"),
            OsString::from("--json"),
        ],
        timeout,
        MAX_STDOUT,
        MAX_STDERR,
    )?;
    if !validated.status.success() {
        bail!(
            "agy plugin validate failed: {}",
            String::from_utf8_lossy(&validated.stderr.bytes)
        );
    }

    let list = list_plugins(&program, timeout)?;
    let ours = env!("CARGO_PKG_VERSION");
    let ours_version =
        parse_version(ours).ok_or_else(|| anyhow::anyhow!("invalid crate version"))?;
    let install_needed = match list.iter().find(|entry| entry.name == PLUGIN) {
        None => true,
        Some(entry) => match entry.version.as_deref().and_then(parse_version) {
            None => true,
            Some(version) if version > ours_version => false,
            Some(version) => {
                version < ours_version
                    || entry.version.as_deref() != Some(ours)
                    || !entry_contents_match(entry, &manifest_path, &config_path)
            }
        },
    };
    if install_needed {
        let installed = run_command(
            &program,
            &[
                OsString::from("plugin"),
                OsString::from("install"),
                plugin.clone().into_os_string(),
            ],
            timeout,
            MAX_STDOUT,
            MAX_STDERR,
        )?;
        if !installed.status.success() {
            bail!(
                "agy plugin install failed: {}",
                String::from_utf8_lossy(&installed.stderr.bytes)
            );
        }
        let after = list_plugins(&program, timeout)?;
        let Some(entry) = after.iter().find(|entry| entry.name == PLUGIN) else {
            bail!("agy plugin install did not install {PLUGIN}");
        };
        if entry.version.as_deref() != Some(ours)
            || !entry_contents_match(entry, &manifest_path, &config_path)
        {
            bail!("agy plugin install produced an invalid {PLUGIN} plugin");
        }
    }
    Ok(())
}

fn resolve_program(command: &OsStr) -> Result<PathBuf> {
    if command.is_empty() {
        bail!("empty command");
    }
    let path = Path::new(command);
    if path.file_name() == Some(OsStr::new("agy")) {
        Ok(path.to_owned())
    } else {
        Ok(PathBuf::from("agy"))
    }
}

#[derive(Deserialize)]
struct PluginEntry {
    name: String,
    version: Option<String>,
    path: PathBuf,
}

fn list_plugins(program: &Path, timeout: Duration) -> Result<Vec<PluginEntry>> {
    let output = run_command_list(
        program,
        &[
            OsString::from("plugin"),
            OsString::from("list"),
            OsString::from("--json"),
        ],
        timeout,
    )?;
    if !output.status.success() {
        bail!(
            "agy plugin list failed: {}",
            String::from_utf8_lossy(&output.stderr.bytes)
        );
    }
    if output.stdout.exceeded {
        bail!("agy plugin list output exceeds 64 KiB");
    }
    serde_json::from_slice(&output.stdout.bytes).context("agy plugin list returned invalid JSON")
}

fn parse_version(value: &str) -> Option<(u64, u64, u64)> {
    let mut parts = value.split('.');
    let version = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    (parts.next().is_none()).then_some(version)
}

fn entry_contents_match(entry: &PluginEntry, manifest: &Path, config: &Path) -> bool {
    let installed = &entry.path;
    bounded_read(&installed.join("plugin.json"), MAX_FILE).as_deref()
        == bounded_read(manifest, MAX_FILE).as_deref()
        && bounded_read(&installed.join("mcp_config.json"), MAX_FILE).as_deref()
            == bounded_read(config, MAX_FILE).as_deref()
}

fn bounded_read(path: &Path, limit: usize) -> Option<Vec<u8>> {
    let mut file = File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > limit {
        None
    } else {
        Some(bytes)
    }
}

struct CommandOutput {
    status: std::process::ExitStatus,
    stdout: CapturedOutput,
    stderr: CapturedOutput,
}

struct CapturedOutput {
    bytes: Vec<u8>,
    exceeded: bool,
}

fn run_command(
    program: &Path,
    args: &[OsString],
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
) -> Result<CommandOutput> {
    run_command_inner(program, args, timeout, stdout_limit, stderr_limit, false)
}

fn run_command_clear(
    program: &Path,
    args: &[OsString],
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
) -> Result<CommandOutput> {
    run_command_inner(program, args, timeout, stdout_limit, stderr_limit, true)
}

fn run_command_list(program: &Path, args: &[OsString], timeout: Duration) -> Result<CommandOutput> {
    run_command_inner(program, args, timeout, MAX_STDOUT, MAX_STDERR, false)
}

fn run_command_inner(
    program: &Path,
    args: &[OsString],
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
    clear_env: bool,
) -> Result<CommandOutput> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if clear_env {
        command.env_clear();
    } else {
        command
            .env_remove("A2AMX_ADDR")
            .env_remove("A2AMX_TOKEN")
            .env_remove("A2AMX_SESSION");
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("run {}", program.display()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("agy command has no stdout pipe"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("agy command has no stderr pipe"))?;
    let (stdout_tx, stdout_rx) = mpsc::channel();
    let (stderr_tx, stderr_rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = stdout_tx.send(read_limited(stdout, stdout_limit));
    });
    thread::spawn(move || {
        let _ = stderr_tx.send(read_limited(stderr, stderr_limit));
    });
    let started = Instant::now();
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= timeout {
            timed_out = true;
            let _ = child.kill();
            let kill_deadline = Instant::now() + Duration::from_secs(1);
            loop {
                if child.try_wait()?.is_some() || Instant::now() >= kill_deadline {
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
            break child.try_wait()?.ok_or_else(|| {
                anyhow::anyhow!(
                    "agy command timed out after {:.3} seconds",
                    timeout.as_secs_f64()
                )
            })?;
        }
        thread::sleep(Duration::from_millis(10));
    };
    let deadline = Instant::now() + Duration::from_secs(1);
    let stdout_result = receive_output(stdout_rx, deadline);
    let stderr_result = receive_output(stderr_rx, deadline);
    if timed_out {
        bail!(
            "agy command timed out after {:.3} seconds",
            timeout.as_secs_f64()
        );
    }
    let stdout = stdout_result?;
    let stderr = stderr_result?;
    Ok(CommandOutput {
        status,
        stdout,
        stderr,
    })
}

fn receive_output(
    receiver: Receiver<io::Result<CapturedOutput>>,
    deadline: Instant,
) -> Result<CapturedOutput> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    match receiver.recv_timeout(remaining) {
        Ok(output) => output.map_err(|error| anyhow::anyhow!("agy output read failed: {error}")),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            bail!("agy command left its output pipe open")
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => bail!("agy output reader failed"),
    }
}

fn read_limited<R: Read>(mut reader: R, limit: usize) -> io::Result<CapturedOutput> {
    let mut bytes = Vec::with_capacity(limit.min(8192));
    let mut exceeded = false;
    let mut buffer = [0_u8; 8192];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        if bytes.len() < limit {
            let kept = count.min(limit - bytes.len());
            bytes.extend_from_slice(&buffer[..kept]);
            if kept < count {
                exceeded = true;
            }
        } else {
            exceeded = true;
        }
    }
    Ok(CapturedOutput { bytes, exceeded })
}
