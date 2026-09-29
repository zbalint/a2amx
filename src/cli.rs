//! Command-line shape.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "a2amx", version)]
pub struct Cli {
    /// State directory. Defaults to `A2AMX_HOME`.
    #[arg(long, global = true, env = "A2AMX_HOME")]
    pub home: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the host daemon in the foreground.
    Daemon {
        /// Listen address; repeatable. Defaults to loopback with an OS-chosen port.
        #[arg(long)]
        listen: Vec<std::net::SocketAddr>,
    },
    /// Start a session running COMMAND and attach to it.
    New {
        #[arg(long)]
        detach: bool,
        #[arg(trailing_var_arg = true, required = true)]
        command: Vec<String>,
    },
    /// List sessions.
    List,
    /// Attach to a session.
    Attach {
        session: String,
        /// Take control from another attachment.
        #[arg(long)]
        force: bool,
    },
    /// Terminate a session.
    Kill { session: String },
}
