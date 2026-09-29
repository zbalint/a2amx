//! Session runtime: one PTY, its child, and its emulator.
//!
//! Blocking PTY reads, PTY writes, and the drop of the PTY (which blocks on the
//! child) stay off the async runtime. The PTY master is non-blocking and ends with
//! EIO when the child closes the slave. Resize uses our own ioctl, never
//! `Pty::on_resize`, which would exit the process on failure.

use crate::emulator::Size;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(pub String);

pub struct SessionSpec {
    pub argv: Vec<String>,
    pub cwd: Option<std::path::PathBuf>,
    pub size: Size,
}

pub struct Session {
    id: SessionId,
}

impl Session {
    pub fn spawn(id: SessionId, spec: SessionSpec) -> anyhow::Result<Self> {
        todo!()
    }

    pub fn id(&self) -> &SessionId {
        &self.id
    }

    pub fn write_input(&self, bytes: &[u8]) -> anyhow::Result<()> {
        todo!()
    }

    pub fn resize(&self, size: Size) -> anyhow::Result<()> {
        todo!()
    }

    /// Exit code once the child has exited.
    pub fn exit_code(&self) -> Option<i32> {
        todo!()
    }
}
