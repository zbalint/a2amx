//! A2AMX: terminal multiplexer and message exchange for agent harness sessions.
//!
//! Terminal-core modules expose the screen model, PTY sessions, local daemon,
//! framing, prefix handling, and human client.

pub mod cli;
pub mod client;
pub mod daemon;
pub mod emulator;
pub mod prefix;
pub mod session;
pub mod wire;
