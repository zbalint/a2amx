//! Prefix-key state machine for the attach client.
//!
//! Human input passes through untouched except for the configurable prefix
//! (default Ctrl-B, byte 0x02). Prefix twice sends a literal prefix. The
//! machine tolerates reads split at any byte and never interprets bytes inside
//! a bracketed paste (`ESC[200~` ... `ESC[201~`).

use anyhow::{Result, bail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Detach,
    SessionPicker,
    ScrollMode,
    Release,
    StatusLine,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Bytes to send to the hosted session unchanged.
    Forward(Vec<u8>),
    Command(Command),
}

const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";

/// Parse the restricted control-byte notation accepted by the attach client.
pub fn parse_prefix(spec: &str) -> Result<u8> {
    let normalized = spec.to_ascii_lowercase();
    if normalized == "c-space" {
        return Ok(0);
    }

    let Some(key) = normalized.strip_prefix("c-") else {
        bail!(
            "invalid prefix {spec:?}: expected C-space, C-a through C-z, C-\\\\, C-], C-^, or C-_"
        );
    };

    if key == "[" {
        bail!("invalid prefix {spec:?}: C-[ is ESC, which is reserved and not accepted");
    }

    if key.len() == 1 {
        let byte = key.as_bytes()[0];
        if byte.is_ascii_lowercase() {
            let control = byte - b'a' + 1;
            let reason = match byte {
                b'i' => Some("Tab"),
                b'j' => Some("LF"),
                b'm' => Some("CR"),
                _ => None,
            };
            if let Some(reason) = reason {
                let key_char = char::from(byte);
                bail!(
                    "invalid prefix {spec:?}: C-{key_char} is {reason}, which is reserved and not accepted"
                );
            }
            return Ok(control);
        }
    }

    if matches!(key, "\\" | "]" | "^" | "_") {
        return Ok(match key.as_bytes()[0] {
            b'\\' => 0x1c,
            b']' => 0x1d,
            b'^' => 0x1e,
            b'_' => 0x1f,
            _ => unreachable!(),
        });
    }

    if matches!(key, "@" | "?") {
        bail!("invalid prefix {spec:?}: C-{key} is not in the accepted control-byte range");
    }

    bail!(
        "invalid prefix {spec:?}: unsupported key; accepted controls are C-space, C-a..C-z, C-\\\\, C-], C-^, and C-_"
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PasteState {
    Outside { matched: usize },
    Inside { matched: usize },
}

pub struct PrefixMachine {
    prefix: u8,
    pending_prefix: bool,
    paste: PasteState,
}

impl PrefixMachine {
    pub fn new(prefix: u8) -> Self {
        Self {
            prefix,
            pending_prefix: false,
            paste: PasteState::Outside { matched: 0 },
        }
    }

    /// Consume one read of human input. Adjacent forwarded bytes are coalesced
    /// into a single `Forward`. State carries over to the next call.
    pub fn feed(&mut self, input: &[u8]) -> Vec<Action> {
        let mut actions = Vec::new();
        let mut forwarded = Vec::new();

        for &byte in input {
            match self.paste {
                PasteState::Inside { .. } => {
                    forwarded.push(byte);
                    self.advance_paste_marker(byte, PASTE_END);
                }
                PasteState::Outside { .. } => {
                    if self.pending_prefix {
                        self.pending_prefix = false;
                        if byte == self.prefix {
                            forwarded.push(byte);
                            self.advance_paste_marker(byte, PASTE_START);
                        } else {
                            match byte {
                                b'd' => {
                                    Self::flush_forward(&mut actions, &mut forwarded);
                                    actions.push(Action::Command(Command::Detach));
                                }
                                b'w' => {
                                    Self::flush_forward(&mut actions, &mut forwarded);
                                    actions.push(Action::Command(Command::SessionPicker));
                                }
                                b'[' => {
                                    Self::flush_forward(&mut actions, &mut forwarded);
                                    actions.push(Action::Command(Command::ScrollMode));
                                }
                                b's' => {
                                    Self::flush_forward(&mut actions, &mut forwarded);
                                    actions.push(Action::Command(Command::StatusLine));
                                }
                                b'r' => {
                                    Self::flush_forward(&mut actions, &mut forwarded);
                                    actions.push(Action::Command(Command::Release));
                                }
                                _ => {}
                            }
                        }
                    } else if byte == self.prefix {
                        self.pending_prefix = true;
                    } else {
                        forwarded.push(byte);
                        self.advance_paste_marker(byte, PASTE_START);
                    }
                }
            }
        }

        Self::flush_forward(&mut actions, &mut forwarded);
        actions
    }

    fn flush_forward(actions: &mut Vec<Action>, forwarded: &mut Vec<u8>) {
        if !forwarded.is_empty() {
            actions.push(Action::Forward(std::mem::take(forwarded)));
        }
    }

    fn advance_paste_marker(&mut self, byte: u8, marker: &[u8]) {
        let matched = match self.paste {
            PasteState::Outside { matched } | PasteState::Inside { matched } => matched,
        };

        let next = if byte == marker[matched] {
            matched + 1
        } else if byte == marker[0] {
            1
        } else {
            0
        };

        if next == marker.len() {
            self.paste = if marker == PASTE_START {
                PasteState::Inside { matched: 0 }
            } else {
                PasteState::Outside { matched: 0 }
            };
        } else {
            self.paste = match self.paste {
                PasteState::Outside { .. } => PasteState::Outside { matched: next },
                PasteState::Inside { .. } => PasteState::Inside { matched: next },
            };
        }
    }
}
