use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    root: PathBuf,
}

impl Layout {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn resolve(explicit: Option<PathBuf>, user_home: Option<PathBuf>) -> anyhow::Result<Self> {
        if let Some(root) = explicit {
            if root.as_os_str().is_empty() {
                anyhow::bail!("the a2amx directory must not be empty");
            }
            return Ok(Self::new(root));
        }
        user_home
            .map(|home| Self::new(home.join(".a2amx")))
            .ok_or_else(|| {
                anyhow::anyhow!("cannot determine the a2amx directory: set HOME or A2AMX_HOME")
            })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn config_dir(&self) -> PathBuf {
        self.root.join("config")
    }
    pub fn secrets_dir(&self) -> PathBuf {
        self.config_dir().join("secrets")
    }
    pub fn state_dir(&self) -> PathBuf {
        self.root.join("state")
    }
    pub fn gateway_state_dir(&self) -> PathBuf {
        self.state_dir().join("gateway")
    }
    pub fn run_dir(&self) -> PathBuf {
        self.root.join("run")
    }
    pub fn link_dir(&self) -> PathBuf {
        self.run_dir().join("link")
    }
    pub fn codex_dir(&self) -> PathBuf {
        self.run_dir().join("codex")
    }
    pub fn codex_session_dir(&self, id: &str) -> PathBuf {
        self.codex_dir().join(id)
    }
    pub fn omp_dir(&self) -> PathBuf {
        self.run_dir().join("omp")
    }
    pub fn log_dir(&self) -> PathBuf {
        self.root.join("log")
    }

    pub fn addr(&self) -> PathBuf {
        self.run_dir().join("addr")
    }
    pub fn admin_token(&self) -> PathBuf {
        self.run_dir().join("admin.token")
    }
    pub fn gateway_token(&self) -> PathBuf {
        self.run_dir().join("gateway.token")
    }
    pub fn daemon_lock(&self) -> PathBuf {
        self.run_dir().join("daemon.lock")
    }
    pub fn gateway_lock(&self) -> PathBuf {
        self.run_dir().join("gateway.lock")
    }
    pub fn gateway_pid(&self) -> PathBuf {
        self.run_dir().join("gateway.pid")
    }
    pub fn link_addr(&self) -> PathBuf {
        self.link_dir().join("addr")
    }
    pub fn link_gateway_token(&self) -> PathBuf {
        self.link_dir().join("gateway.token")
    }
    pub fn confine_empty(&self) -> PathBuf {
        self.run_dir().join("confine-empty")
    }
    pub fn messages_db(&self) -> PathBuf {
        self.state_dir().join("messages.db")
    }
    /// Accepted auto grants, keyed by canonical project directory.
    pub fn auto_grants(&self) -> PathBuf {
        self.state_dir().join("auto-grants.toml")
    }
    pub fn pairing(&self) -> PathBuf {
        self.gateway_state_dir().join("pairing.json")
    }
    pub fn seen(&self) -> PathBuf {
        self.gateway_state_dir().join("seen.json")
    }
    pub fn team_file(&self) -> PathBuf {
        self.config_dir().join("team.toml")
    }
    pub fn gateway_config(&self) -> PathBuf {
        self.config_dir().join("gateway.toml")
    }
    pub fn daemon_config(&self) -> PathBuf {
        self.config_dir().join("daemon.toml")
    }

    pub fn client_config(&self) -> PathBuf {
        self.config_dir().join("client.toml")
    }
    pub fn discord_token(&self) -> PathBuf {
        self.secrets_dir().join("discord.token")
    }
    pub fn daemon_log(&self) -> PathBuf {
        self.log_dir().join("daemon.log")
    }
    pub fn gateway_log(&self) -> PathBuf {
        self.log_dir().join("gateway.log")
    }

    pub fn ensure(&self) -> anyhow::Result<()> {
        let dirs = [
            self.root.clone(),
            self.config_dir(),
            self.secrets_dir(),
            self.state_dir(),
            self.gateway_state_dir(),
            self.run_dir(),
            self.link_dir(),
            self.log_dir(),
        ];
        for directory in dirs {
            match fs::metadata(&directory) {
                Ok(metadata) if metadata.is_dir() => {}
                Ok(_) => anyhow::bail!("a2amx path is not a directory: {}", directory.display()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let mut missing = Vec::new();
                    let mut current = directory.as_path();
                    while !current.exists() {
                        missing.push(current.to_owned());
                        current = current
                            .parent()
                            .ok_or_else(|| anyhow::anyhow!("a2amx path has no parent"))?;
                    }
                    for path in missing.iter().rev() {
                        fs::create_dir(path)?;
                        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
}
