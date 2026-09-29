//! A2AMX: terminal multiplexer and message exchange for agent harness sessions.
//!
//! Spec 1 (terminal core) modules live here. Bodies marked `todo!()` are
//! implemented test-first; the public signatures are fixed by the scaffold.

// shortcut: scaffold bodies are `todo!()`, so parameters and fields are unused.
// Remove this allow module by module as the bodies land (spec 1).
#![allow(unused_variables, dead_code)]

pub mod cli;
pub mod client;
pub mod daemon;
pub mod emulator;
pub mod prefix;
pub mod session;
pub mod wire;
