//! Wire format: `u32` big-endian length followed by the payload.
//!
//! The control channel carries JSON. An attach opens a new connection that
//! switches to binary stream frames (server: `Data`, `Exit`; client: `Input`,
//! `Resize`, `Detach`).

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::emulator::Scroll;

/// Largest accepted frame payload. Decoding rejects longer frames.
pub const MAX_FRAME_LEN: usize = 1 << 20;

/// Largest client input payload in one stream frame.
pub const MAX_INPUT_LEN: usize = 8 * 1024;

/// Largest server data payload in one stream frame.
pub const MAX_SERVER_DATA_LEN: usize = 64 * 1024;

/// Prefix `payload` with its big-endian `u32` length. Errors above `MAX_FRAME_LEN`.
pub fn encode_frame(payload: &[u8]) -> Result<Vec<u8>> {
    if payload.len() > MAX_FRAME_LEN {
        bail!(
            "frame payload length {} exceeds maximum {}",
            payload.len(),
            MAX_FRAME_LEN
        );
    }

    let mut encoded = Vec::with_capacity(4 + payload.len());
    encoded.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    encoded.extend_from_slice(payload);
    Ok(encoded)
}

/// Incremental frame decoder that tolerates arbitrary read boundaries.
#[derive(Default)]
pub struct FrameDecoder {
    buf: Vec<u8>,
    expected: Option<usize>,
    header: [u8; 4],
    header_len: usize,
}

impl FrameDecoder {
    /// Append bytes and return every complete frame payload, in order. Errors on a
    /// declared length above `MAX_FRAME_LEN`, before buffering the payload.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>> {
        let mut frames = Vec::new();
        let mut cursor = 0;

        loop {
            if self.expected.is_none() {
                while self.header_len < self.header.len() && cursor < bytes.len() {
                    self.header[self.header_len] = bytes[cursor];
                    self.header_len += 1;
                    cursor += 1;
                }
                if self.header_len < self.header.len() {
                    break;
                }

                let length = u32::from_be_bytes(self.header) as usize;
                self.header_len = 0;
                self.header = [0; 4];
                if length > MAX_FRAME_LEN {
                    self.buf.clear();
                    self.expected = None;
                    bail!(
                        "declared frame length {} exceeds maximum {}",
                        length,
                        MAX_FRAME_LEN
                    );
                }
                self.expected = Some(length);
                self.buf.clear();
                if length == 0 {
                    self.expected = None;
                    frames.push(Vec::new());
                    continue;
                }
            }

            let Some(expected) = self.expected else {
                continue;
            };
            let needed = expected - self.buf.len();
            let available = bytes.len() - cursor;
            if available == 0 {
                break;
            }
            let take = needed.min(available);
            self.buf.extend_from_slice(&bytes[cursor..cursor + take]);
            cursor += take;
            if self.buf.len() == expected {
                self.expected = None;
                frames.push(std::mem::take(&mut self.buf));
            }
        }

        Ok(frames)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Hello {
        token: String,
    },
    NewSession {
        argv: Vec<String>,
        cols: u16,
        rows: u16,
        cwd: Option<String>,
        env: Vec<(String, String)>,
    },
    List,
    /// Followed by a switch of this connection to stream frames.
    Attach {
        session: String,
        force: bool,
        cols: u16,
        rows: u16,
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
    Detached(String),
}

/// Client-to-server stream frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientFrame {
    Input(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    Detach,
    Redraw,
    Scroll(Scroll),
}
impl ServerFrame {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Self::Data(data) => {
                let mut payload = Vec::with_capacity(1 + data.len());
                payload.push(0x01);
                payload.extend_from_slice(data);
                payload
            }
            Self::Exit(code) => {
                let mut payload = Vec::with_capacity(5);
                payload.push(0x02);
                payload.extend_from_slice(&code.to_be_bytes());
                payload
            }
            Self::Detached(reason) => {
                let mut payload = Vec::with_capacity(1 + reason.len());
                payload.push(0x03);
                payload.extend_from_slice(reason.as_bytes());
                payload
            }
        }
    }

    pub fn decode(payload: &[u8]) -> Result<Self> {
        let Some((&tag, body)) = payload.split_first() else {
            bail!("empty server stream frame payload");
        };
        match tag {
            0x01 => {
                if body.len() > MAX_SERVER_DATA_LEN {
                    bail!(
                        "server data payload length {} exceeds maximum {}",
                        body.len(),
                        MAX_SERVER_DATA_LEN
                    );
                }
                Ok(Self::Data(body.to_vec()))
            }
            0x02 => {
                if body.len() != 4 {
                    bail!("server exit frame body must be 4 bytes");
                }
                let code = i32::from_be_bytes([body[0], body[1], body[2], body[3]]);
                Ok(Self::Exit(code))
            }
            0x03 => Ok(Self::Detached(
                String::from_utf8(body.to_vec()).map_err(anyhow::Error::new)?,
            )),
            _ => bail!("unknown server stream frame tag 0x{tag:02x}"),
        }
    }
}

impl ClientFrame {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Self::Input(data) => {
                let mut payload = Vec::with_capacity(1 + data.len());
                payload.push(0x01);
                payload.extend_from_slice(data);
                payload
            }
            Self::Resize { cols, rows } => {
                let mut payload = Vec::with_capacity(5);
                payload.push(0x02);
                payload.extend_from_slice(&cols.to_be_bytes());
                payload.extend_from_slice(&rows.to_be_bytes());
                payload
            }
            Self::Detach => vec![0x03],
            Self::Redraw => vec![0x04],
            Self::Scroll(scroll) => vec![0x05, scroll_to_byte(*scroll)],
        }
    }

    pub fn decode(payload: &[u8]) -> Result<Self> {
        let Some((&tag, body)) = payload.split_first() else {
            bail!("empty client stream frame payload");
        };
        match tag {
            0x01 => {
                if body.len() > MAX_INPUT_LEN {
                    bail!(
                        "client input payload length {} exceeds maximum {}",
                        body.len(),
                        MAX_INPUT_LEN
                    );
                }
                Ok(Self::Input(body.to_vec()))
            }
            0x02 => {
                if body.len() != 4 {
                    bail!("client resize frame body must be 4 bytes");
                }
                Ok(Self::Resize {
                    cols: u16::from_be_bytes([body[0], body[1]]),
                    rows: u16::from_be_bytes([body[2], body[3]]),
                })
            }
            0x03 => {
                if !body.is_empty() {
                    bail!("client detach frame body must be empty");
                }
                Ok(Self::Detach)
            }
            0x04 => {
                if !body.is_empty() {
                    bail!("client redraw frame body must be empty");
                }
                Ok(Self::Redraw)
            }
            0x05 => {
                if body.len() != 1 {
                    bail!("client scroll frame body must be one byte");
                }
                Ok(Self::Scroll(byte_to_scroll(body[0])?))
            }
            _ => bail!("unknown client stream frame tag 0x{tag:02x}"),
        }
    }
}

fn scroll_to_byte(scroll: Scroll) -> u8 {
    match scroll {
        Scroll::LineUp => 0,
        Scroll::LineDown => 1,
        Scroll::PageUp => 2,
        Scroll::PageDown => 3,
        Scroll::Top => 4,
        Scroll::Bottom => 5,
    }
}

fn byte_to_scroll(byte: u8) -> Result<Scroll> {
    match byte {
        0 => Ok(Scroll::LineUp),
        1 => Ok(Scroll::LineDown),
        2 => Ok(Scroll::PageUp),
        3 => Ok(Scroll::PageDown),
        4 => Ok(Scroll::Top),
        5 => Ok(Scroll::Bottom),
        _ => bail!("unknown client scroll value {byte}"),
    }
}
