//! Command-line shape.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::harness::{Deliver, Harness};
use crate::prefix::parse_prefix;

#[derive(Debug, Parser)]
#[command(name = "a2amx", version)]
pub struct Cli {
    /// State directory. Defaults to A2AMX_HOME, XDG_STATE_HOME, or $HOME.
    #[arg(long, global = true, env = "A2AMX_HOME")]
    pub home: Option<PathBuf>,

    /// Prefix key used by an attached client.
    #[arg(
        long,
        global = true,
        env = "A2AMX_PREFIX",
        default_value = "C-b",
        value_parser = parse_prefix
    )]
    pub prefix: u8,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the host daemon (in the foreground unless --background).
    Daemon {
        /// Listen address; repeatable. Defaults to loopback with an OS-chosen port.
        #[arg(long)]
        listen: Vec<std::net::SocketAddr>,
        #[arg(long)]
        host_name: Option<String>,
        /// Start detached from the terminal and print the listen address. Output goes to <state dir>/daemon.log.
        #[arg(long)]
        background: bool,
    },
    /// Start a session running COMMAND and attach to it.
    New {
        #[arg(long)]
        detach: bool,
        #[arg(long)]
        name: Option<String>,
        /// Harness profile. Inferred from the command when omitted: claude, codex and omp are recognised by executable name, anything else is generic. omp delivers through an OMP extension; without it, messages wait (channel_down). codex hosts a private app-server and delivers through it.
        #[arg(long, value_enum)]
        harness: Option<Harness>,
        #[arg(long, value_enum)]
        deliver: Option<Deliver>,
        #[arg(long)]
        no_authorize_peers: bool,
        /// Claude only: deliver by typing into the terminal instead of through a channel.
        #[arg(long)]
        no_channel: bool,
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
    /// Stop the daemon. This ends every session.
    Stop {
        /// Do not ask for confirmation.
        #[arg(long)]
        yes: bool,
    },
    /// List messages.
    Messages {
        #[arg(long)]
        session: Option<String>,
        #[arg(
            long,
            value_parser = [
                "pending",
                "delivering",
                "submitted",
                "unsubmitted",
                "cancelled",
                "undeliverable"
            ]
        )]
        state: Option<String>,
    },
    /// Cancel a pending message.
    Cancel { message: String },
    /// Run the OMP extension's bridge to the daemon (reads and writes JSON lines on stdio).
    OmpBridge,
    /// Run the MCP server; --channel also delivers messages as Claude Code channel events.
    Mcp {
        #[arg(long)]
        channel: bool,
    },
    /// Run the prompt-submit hook adapter (reads the harness payload on stdin).
    Hook,
}
