//! A2AMX: terminal multiplexer and message exchange for agent harness sessions.
//!
//! Terminal-core modules expose the screen model, PTY sessions, local daemon,
//! framing, prefix handling, and human client.

/// Git description of this build, or the package version when git metadata is unavailable.
pub const VERSION: &str = env!("A2AMX_VERSION");

pub mod bridge;
mod channel;
pub mod cli;
pub mod client;
pub mod codex;
pub mod daemon;
mod delivery;
pub mod emulator;
pub mod harness;
pub mod hook;
pub mod mcp;
pub mod messaging;
pub mod names;
pub mod omp;
pub mod picker;
pub mod prefix;
pub mod quota;
pub mod session;
pub mod status;
mod store;
pub mod team;
pub mod wire;
