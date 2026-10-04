//! Embedded OMP extension and owner-only per-session launch files.

use std::fs::{self, OpenOptions, Permissions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub struct Installed {
    pub extension: PathBuf,
    pub overlay: PathBuf,
}

/// The embedded extension with the tool schemas rendered in.
pub fn render_extension() -> anyhow::Result<String> {
    let schemas = serde_json::to_string(&crate::mcp::tool_schemas())?;
    Ok(include_str!("../extension/omp.ts").replace("[/* a2amx:tools */]", &schemas))
}

/// The per-session overlay: native tools for this session only.
pub fn overlay() -> &'static str {
    "tools:\n  xdev: false\n"
}

/// Writes both files under `<home>/omp`; call from a blocking thread.
pub fn install(home: &Path) -> anyhow::Result<Installed> {
    let home = std::path::absolute(home)?;
    let dir = home.join("omp");
    fs::create_dir_all(&dir)?;
    fs::set_permissions(&dir, Permissions::from_mode(0o700))?;
    let installed = Installed {
        extension: dir.join(concat!("extension-", env!("CARGO_PKG_VERSION"), ".ts")),
        overlay: dir.join("overlay.yml"),
    };
    write_if_changed(&installed.extension, render_extension()?.as_bytes())?;
    write_if_changed(&installed.overlay, overlay().as_bytes())?;
    Ok(installed)
}

fn write_if_changed(path: &Path, content: &[u8]) -> anyhow::Result<()> {
    let changed = match fs::read(path) {
        Ok(old) => old != content,
        Err(error) if error.kind() == ErrorKind::NotFound => true,
        Err(error) => return Err(error.into()),
    };
    if changed {
        static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
        let (temp, mut file) = loop {
            let temp = path.with_file_name(format!(
                ".install-{}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temp)
            {
                Ok(file) => break (temp, file),
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        };
        let result = file
            .write_all(content)
            .and_then(|()| fs::rename(&temp, path));
        if let Err(error) = result {
            // Preserve the write error: it is the one the operator needs.
            let _ = fs::remove_file(&temp);
            return Err(error.into());
        }
    }
    fs::set_permissions(path, Permissions::from_mode(0o600))?;
    Ok(())
}
