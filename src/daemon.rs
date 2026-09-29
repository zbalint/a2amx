//! The host daemon: owns session runtimes and serves the local TCP protocol.

use std::net::SocketAddr;
use std::path::PathBuf;

pub struct DaemonConfig {
    /// State directory (`A2AMX_HOME` or `--home`).
    pub state_dir: PathBuf,
    /// Listen addresses; loopback by default. Port 0 lets the OS choose.
    pub listen: Vec<SocketAddr>,
}

pub struct Daemon {
    addrs: Vec<SocketAddr>,
}

impl Daemon {
    /// Bind every listen address, write the chosen addresses to a file in the
    /// state dir, write the admin token file (mode 0600), and start serving.
    pub async fn start(config: DaemonConfig) -> anyhow::Result<Self> {
        todo!()
    }

    /// Addresses actually bound (resolves port 0).
    pub fn addrs(&self) -> &[SocketAddr] {
        &self.addrs
    }

    pub async fn shutdown(self) -> anyhow::Result<()> {
        todo!()
    }
}
