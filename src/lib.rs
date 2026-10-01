//! A2AMX: terminal multiplexer and message exchange for agent harness sessions.
//!
//! Terminal-core modules expose the screen model, PTY sessions, local daemon,
//! framing, prefix handling, and human client.

pub mod bridge;
pub mod cli;
pub mod client;
pub mod daemon;
mod delivery;
pub mod emulator;
pub mod harness;
pub mod hook;
pub mod mcp;
pub mod messaging;
pub mod prefix;
pub mod session;
mod store;
pub mod wire;
