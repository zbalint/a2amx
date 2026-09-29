//! Client API used by the human CLI. Reads the daemon address and admin token from
//! the state dir; errors with a hint when no daemon is running.

use std::path::Path;

use crate::wire::{Request, Response};

pub struct Client {
    _private: (),
}

impl Client {
    pub async fn connect(state_dir: &Path) -> anyhow::Result<Self> {
        todo!()
    }

    pub async fn request(&mut self, request: Request) -> anyhow::Result<Response> {
        todo!()
    }
}
