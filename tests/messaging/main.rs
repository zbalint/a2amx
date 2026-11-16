use a2amx::emulator::{Emulator, Size};
use std::path::Path;

use a2amx::harness::{self, Deliver, Harness};
use a2amx::messaging::{
    COOLDOWN, Limits, MAX_HUMAN_BODY_BYTES, MAX_MESSAGE_BYTES, MAX_SUBJECT_BYTES, MessageState,
    Origin, PASTE_GAP, address, default_host_name, input_is_typing, local_part, origin_of,
    paste_bytes, render_envelope, validate_host, validate_message, validate_name,
};
use tempfile::TempDir;

mod agy_readiness;
mod claude_readiness;
mod harness_wiring;
mod limits_and_envelopes;
mod paste_and_teams;
