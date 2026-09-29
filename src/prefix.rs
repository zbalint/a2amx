//! Prefix-key state machine for the attach client.
//!
//! Human input passes through untouched except for the configurable prefix
//! (default Ctrl-Space, byte 0x00). Prefix twice sends a literal prefix. The
//! machine tolerates reads split at any byte and never interprets bytes inside
//! a bracketed paste (`ESC[200~` ... `ESC[201~`).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Detach,
    SessionPicker,
    ScrollMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Bytes to send to the hosted session unchanged.
    Forward(Vec<u8>),
    Command(Command),
}

pub struct PrefixMachine {
    prefix: u8,
}

impl PrefixMachine {
    pub fn new(prefix: u8) -> Self {
        todo!()
    }

    /// Consume one read of human input. Adjacent forwarded bytes are coalesced
    /// into a single `Forward`. State carries over to the next call.
    pub fn feed(&mut self, input: &[u8]) -> Vec<Action> {
        todo!()
    }
}
