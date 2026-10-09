//! Client API used by the human CLI. Reads the daemon address and admin token from
//! the state dir; errors with a hint when no daemon is running.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::path::Path;

use anyhow::{Context, bail};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::wire::{
    BridgeDown, BridgeUp, ClientFrame, FrameDecoder, Request, Response, ServerFrame,
};

pub const UNREACHABLE: &str = "cannot reach the a2amx daemon: start it with `a2amx daemon start`";

/// Returns a warning when the daemon's build differs from the running CLI build.
pub fn version_warning(cli: &str, daemon: Option<&str>) -> Option<String> {
    match daemon {
        Some(version) if version == cli => None,
        Some(version) => Some(format!(
            "warning: daemon version {version} differs from this binary {cli}; restart the daemon to use this binary (sessions end)"
        )),
        None => Some(
            "warning: daemon predates version reporting; restart it to use this binary (sessions end)"
                .to_owned(),
        ),
    }
}

fn validate_payload(payload: &[u8]) -> anyhow::Result<()> {
    if payload.len() > crate::wire::MAX_FRAME_LEN {
        bail!("frame exceeds maximum length");
    }
    Ok(())
}

pub struct Client {
    connection: Framed,
}

pub(crate) struct PreparedRequest(Vec<u8>);

impl Client {
    pub async fn connect(state_dir: &Path) -> anyhow::Result<Self> {
        let path = state_dir.to_owned();
        let (addr, token) = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            let addresses = match std::fs::read_to_string(path.join("addr")) {
                Ok(value) => value,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => bail!(UNREACHABLE),
                Err(error) => return Err(error.into()),
            };
            let addr = addresses
                .lines()
                .next()
                .context("daemon addr file is empty")?
                .parse()?;
            let token = std::fs::read_to_string(path.join("admin.token"))
                .context("cannot read the daemon admin.token file")?;
            Ok((addr, token))
        })
        .await??;
        Self::connect_addr(addr, token.trim()).await
    }

    pub async fn connect_addr(addr: SocketAddr, token: &str) -> anyhow::Result<Self> {
        let stream = match TcpStream::connect(addr).await {
            Ok(stream) => stream,
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => {
                bail!(UNREACHABLE)
            }
            Err(error) => return Err(error.into()),
        };
        let mut client = Self {
            connection: Framed::new(stream),
        };
        match client
            .request(Request::Hello {
                token: token.to_owned(),
            })
            .await?
        {
            Response::Ok => Ok(client),
            _ => bail!("daemon rejected authentication; check the admin.token file"),
        }
    }

    pub async fn request(&mut self, request: Request) -> anyhow::Result<Response> {
        self.request_prepared(Self::prepare_request(&request)?)
            .await
    }

    /// Finish serialization and size validation before any request bytes can be
    /// written, so MCP can distinguish a known pre-write failure from uncertainty.
    pub(crate) fn prepare_request(request: &Request) -> anyhow::Result<PreparedRequest> {
        let payload = serde_json::to_vec(request)?;
        validate_payload(&payload)?;
        Ok(PreparedRequest(payload))
    }

    pub(crate) async fn request_prepared(
        &mut self,
        request: PreparedRequest,
    ) -> anyhow::Result<Response> {
        self.connection.send_payload(&request.0).await?;
        let payload = self
            .connection
            .recv()
            .await?
            .context("daemon closed the control connection")?;
        Ok(serde_json::from_slice(&payload)?)
    }

    pub async fn attach(
        self,
        session: &str,
        force: bool,
        cols: u16,
        rows: u16,
    ) -> anyhow::Result<Attachment> {
        self.attach_with_status(session, force, cols, rows, false)
            .await
    }

    pub async fn attach_status(
        self,
        session: &str,
        force: bool,
        cols: u16,
        rows: u16,
    ) -> anyhow::Result<Attachment> {
        self.attach_with_status(session, force, cols, rows, true)
            .await
    }

    async fn attach_with_status(
        mut self,
        session: &str,
        force: bool,
        cols: u16,
        rows: u16,
        status: bool,
    ) -> anyhow::Result<Attachment> {
        match self
            .request(Request::Attach {
                session: session.to_owned(),
                force,
                cols,
                rows,
                status,
            })
            .await?
        {
            Response::Attached => Ok(Attachment {
                connection: self.connection,
            }),
            Response::Error { message } => bail!(message),
            _ => bail!("unexpected daemon attach response"),
        }
    }

    /// Sends `bridge_attach`; on `attached` switches this connection to bridge frames.
    pub async fn bridge(mut self) -> anyhow::Result<BridgeLink> {
        match self.request(Request::BridgeAttach).await? {
            Response::Attached => Ok(BridgeLink {
                connection: self.connection,
            }),
            Response::Error { message } => bail!(message),
            _ => bail!("unexpected daemon bridge response"),
        }
    }
}

pub struct Attachment {
    connection: Framed,
}

impl Attachment {
    pub async fn send(&mut self, frame: ClientFrame) -> anyhow::Result<()> {
        match frame {
            ClientFrame::Input(bytes) => {
                for chunk in bytes.chunks(8192) {
                    self.connection.send_tagged(0x01, chunk).await?;
                }
                Ok(())
            }
            frame => self.connection.send(&frame.encode()).await,
        }
    }

    pub async fn recv(&mut self) -> anyhow::Result<Option<ServerFrame>> {
        self.connection
            .recv()
            .await?
            .map(|payload| ServerFrame::decode(&payload))
            .transpose()
    }
}

pub struct BridgeLink {
    connection: Framed,
}

impl BridgeLink {
    pub async fn send(&mut self, frame: BridgeUp) -> anyhow::Result<()> {
        self.connection.send(&serde_json::to_vec(&frame)?).await
    }

    /// `Ok(None)` when the daemon closed the connection. Cancellation safe.
    pub async fn recv(&mut self) -> anyhow::Result<Option<BridgeDown>> {
        self.connection
            .recv()
            .await?
            .map(|payload| serde_json::from_slice(&payload).map_err(Into::into))
            .transpose()
    }
}

/// Persistent decoder state makes `recv` safe to cancel in a `select!`.
pub(crate) struct Framed {
    pub stream: TcpStream,
    decoder: FrameDecoder,
    pending: VecDeque<Vec<u8>>,
    incomplete: usize,
}

impl Framed {
    pub fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            decoder: FrameDecoder::default(),
            pending: VecDeque::new(),
            incomplete: 0,
        }
    }

    pub async fn recv(&mut self) -> anyhow::Result<Option<Vec<u8>>> {
        loop {
            if let Some(payload) = self.pending.pop_front() {
                return Ok(Some(payload));
            }
            let mut bytes = [0; 8192];
            let count = self.stream.read(&mut bytes).await?;
            if count == 0 {
                if self.incomplete != 0 {
                    bail!("connection closed within a frame");
                }
                return Ok(None);
            }
            self.incomplete += count;
            for payload in self.decoder.push(&bytes[..count])? {
                self.incomplete -= 4 + payload.len();
                self.pending.push_back(payload);
            }
        }
    }

    pub async fn send(&mut self, payload: &[u8]) -> anyhow::Result<()> {
        validate_payload(payload)?;
        self.send_payload(payload).await
    }

    async fn send_payload(&mut self, payload: &[u8]) -> anyhow::Result<()> {
        self.stream
            .write_all(&(payload.len() as u32).to_be_bytes())
            .await?;
        self.stream.write_all(payload).await?;
        Ok(())
    }

    pub async fn send_tagged(&mut self, tag: u8, body: &[u8]) -> anyhow::Result<()> {
        if body.len() >= crate::wire::MAX_FRAME_LEN {
            bail!("frame exceeds maximum length");
        }
        let mut header = [0; 5];
        header[..4].copy_from_slice(&((body.len() + 1) as u32).to_be_bytes());
        header[4] = tag;
        self.stream.write_all(&header).await?;
        self.stream.write_all(body).await?;
        Ok(())
    }
}
