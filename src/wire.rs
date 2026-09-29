//! Wire format: `u32` big-endian length followed by the payload.
//!
//! The control channel carries JSON. An attach opens a new connection that
//! switches to binary stream frames (server: `Data`, `Exit`; client: `Input`,
//! `Resize`, `Detach`).

use serde::{Deserialize, Serialize};

/// Largest accepted frame payload. Decoding rejects longer frames.
pub const MAX_FRAME_LEN: usize = 1 << 20;

/// Prefix `payload` with its big-endian `u32` length. Errors above `MAX_FRAME_LEN`.
pub fn encode_frame(payload: &[u8]) -> anyhow::Result<Vec<u8>> {
    todo!()
}

/// Incremental frame decoder that tolerates arbitrary read boundaries.
#[derive(Default)]
pub struct FrameDecoder {
    buf: Vec<u8>,
}

impl FrameDecoder {
    /// Append bytes and return every complete frame payload, in order. Errors on a
    /// declared length above `MAX_FRAME_LEN`, before buffering the payload.
    pub fn push(&mut self, bytes: &[u8]) -> anyhow::Result<Vec<Vec<u8>>> {
        todo!()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    NewSession {
        argv: Vec<String>,
        cols: u16,
        rows: u16,
        cwd: Option<String>,
    },
    List,
    /// Followed by a switch of this connection to stream frames.
    Attach {
        session: String,
        force: bool,
    },
    Kill {
        session: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Created { session: String },
    Sessions { sessions: Vec<SessionSummary> },
    Attached,
    Ok,
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub argv: Vec<String>,
    pub cols: u16,
    pub rows: u16,
    /// `None` while running, the exit code once the child exited.
    pub exit_code: Option<i32>,
    pub attached: bool,
}

/// Server-to-client stream frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerFrame {
    Data(Vec<u8>),
    Exit(i32),
}

/// Client-to-server stream frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientFrame {
    Input(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    Detach,
}

impl ServerFrame {
    pub fn encode(&self) -> Vec<u8> {
        todo!()
    }
    pub fn decode(payload: &[u8]) -> anyhow::Result<Self> {
        todo!()
    }
}

impl ClientFrame {
    pub fn encode(&self) -> Vec<u8> {
        todo!()
    }
    pub fn decode(payload: &[u8]) -> anyhow::Result<Self> {
        todo!()
    }
}
