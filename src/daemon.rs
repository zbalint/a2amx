//! The host daemon: owns session runtimes and serves the local TCP protocol.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::{JoinHandle, JoinSet};

use crate::client::Framed;
use crate::emulator::Size;
use crate::session::{AttachmentSlot, Session, SessionId, SessionSpec};
use crate::wire::{ClientFrame, Request, Response, ServerFrame, SessionSummary};

pub struct DaemonConfig {
    /// State directory (`A2AMX_HOME` or `--home`).
    pub state_dir: PathBuf,
    /// Listen addresses; loopback by default. Port 0 lets the OS choose.
    pub listen: Vec<SocketAddr>,
}

pub struct Daemon {
    addrs: Vec<SocketAddr>,
    runtime: Arc<Runtime>,
    shutdown: watch::Sender<bool>,
    listeners: Vec<JoinHandle<()>>,
    state_dir: PathBuf,
    lock: Option<File>,
}

struct Runtime {
    sessions: Mutex<BTreeMap<u64, Arc<Session>>>,
    next_id: AtomicU64,
    token: String,
}

impl Daemon {
    /// Bind every listen address, write the chosen addresses to a file in the
    /// state dir, write the admin token file (mode 0600), and start serving.
    pub async fn start(config: DaemonConfig) -> anyhow::Result<Self> {
        let path = config.state_dir.clone();
        let lock = tokio::task::spawn_blocking(move || -> anyhow::Result<File> {
            let mut missing = Vec::new();
            let mut ancestor = path.as_path();
            loop {
                match std::fs::metadata(ancestor) {
                    Ok(metadata) if metadata.is_dir() => break,
                    Ok(_) => return Err(anyhow::anyhow!("state path is not a directory")),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
                missing.push(ancestor.to_owned());
                ancestor = ancestor
                    .parent()
                    .filter(|path| !path.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."));
            }
            for directory in missing.iter().rev() {
                match std::fs::DirBuilder::new().mode(0o700).create(directory) {
                    Ok(()) => {}
                    Err(error)
                        if error.kind() == std::io::ErrorKind::AlreadyExists
                            && directory.is_dir() => {}
                    Err(error) => return Err(error.into()),
                }
                // A restrictive umask must not prevent creation of the next child.
                std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
            }
            if missing.is_empty() {
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
            }
            let lock = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .open(path.join("daemon.lock"))?;
            rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive)
                .map_err(|_| {
                    anyhow::anyhow!("another a2amx daemon is running for this state dir")
                })?;
            Ok(lock)
        })
        .await??;
        let mut bound = Vec::new();
        let mut addrs = Vec::new();
        let listen = if config.listen.is_empty() {
            vec![SocketAddr::from(([127, 0, 0, 1], 0))]
        } else {
            config.listen
        };
        for address in listen {
            let listener = TcpListener::bind(address).await?;
            addrs.push(listener.local_addr()?);
            bound.push(listener);
        }
        let path = config.state_dir.clone();
        let addresses = addrs
            .iter()
            .map(|addr| format!("{addr}\n"))
            .collect::<String>();
        let token = tokio::task::spawn_blocking(move || -> anyhow::Result<String> {
            let mut random = [0; 32];
            File::open("/dev/urandom")?.read_exact(&mut random)?;
            let mut token = String::with_capacity(64);
            use std::fmt::Write as _;
            for byte in random {
                write!(&mut token, "{byte:02x}")?;
            }
            write_state_file(&path.join("admin.token"), token.as_bytes())?;
            write_state_file(&path.join("addr"), addresses.as_bytes())?;
            Ok(token)
        })
        .await??;
        // shortcut: sessions live only in this daemon; add persistence only with
        // a later lifecycle specification.
        let runtime = Arc::new(Runtime {
            sessions: Mutex::new(BTreeMap::new()),
            next_id: AtomicU64::new(1),
            token,
        });
        let (shutdown, _) = watch::channel(false);
        let listeners = bound.into_iter().map(|listener| {
            let runtime = runtime.clone();
            let mut stopping = shutdown.subscribe();
            tokio::spawn(async move {
                let mut connections = JoinSet::new();
                loop {
                    tokio::select! {
                        _ = stopping.changed() => break,
                        accepted = listener.accept() => {
                            match accepted {
                                Ok((socket, _)) => {
                                    let runtime = runtime.clone();
                                    let mut stopping = stopping.clone();
                                    connections.spawn(async move {
                                        tokio::select! {
                                            _ = stopping.changed() => {},
                                            result = serve(socket, runtime) => {
                                                if let Err(error) = result {
                                                    tracing::debug!(%error, "client connection closed");
                                                }
                                            }
                                        }
                                    });
                                }
                                Err(error) => tracing::warn!(%error, "accept failed"),
                            }
                        }
                        Some(result) = connections.join_next(), if !connections.is_empty() => {
                            if let Err(error) = result {
                                tracing::warn!(%error, "connection task failed");
                            }
                        }
                    }
                }
                connections.abort_all();
                while connections.join_next().await.is_some() {}
            })
        }).collect();
        Ok(Self {
            addrs,
            runtime,
            shutdown,
            listeners,
            state_dir: config.state_dir,
            lock: Some(lock),
        })
    }

    /// Addresses actually bound (resolves port 0).
    pub fn addrs(&self) -> &[SocketAddr] {
        &self.addrs
    }

    pub async fn shutdown(mut self) -> anyhow::Result<()> {
        let _ = self.shutdown.send(true);
        for listener in self.listeners.drain(..) {
            listener.await?;
        }
        let sessions = self
            .runtime
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut kills = JoinSet::new();
        for session in sessions {
            kills.spawn(async move { session.kill().await });
        }
        let mut failure = None;
        while let Some(result) = kills.join_next().await {
            if let Err(error) = result
                .context("session shutdown task")
                .and_then(|value| value)
            {
                failure.get_or_insert(error);
            }
        }
        self.runtime
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        let addr = self.state_dir.join("addr");
        let lock = self.lock.take();
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            match std::fs::remove_file(addr) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            drop(lock);
            Ok(())
        })
        .await??;
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
    }
}

fn write_state_file(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(bytes)?;
    Ok(())
}

async fn response(connection: &mut Framed, response: Response) -> anyhow::Result<()> {
    connection.send(&serde_json::to_vec(&response)?).await
}

async fn serve(socket: TcpStream, runtime: Arc<Runtime>) -> anyhow::Result<()> {
    let mut connection = Framed::new(socket);
    let hello = tokio::time::timeout(Duration::from_secs(5), connection.recv()).await;
    let authenticated = match hello {
        Ok(Ok(Some(bytes))) => match serde_json::from_slice::<Request>(&bytes) {
            Ok(Request::Hello { token }) => {
                let mut difference = token.len() ^ runtime.token.len();
                for (index, expected) in runtime.token.bytes().enumerate() {
                    difference |=
                        usize::from(expected ^ token.as_bytes().get(index).copied().unwrap_or(0));
                }
                difference == 0
            }
            _ => false,
        },
        _ => false,
    };
    if !authenticated {
        response(
            &mut connection,
            Response::Error {
                message: "authentication required; check admin.token".into(),
            },
        )
        .await?;
        connection.stream.shutdown().await?;
        return Ok(());
    }
    response(&mut connection, Response::Ok).await?;
    while let Some(bytes) = connection.recv().await? {
        let request = match serde_json::from_slice::<Request>(&bytes) {
            Ok(request) => request,
            Err(_) => {
                response(
                    &mut connection,
                    Response::Error {
                        message: "invalid control request".into(),
                    },
                )
                .await?;
                continue;
            }
        };
        let result = match request {
            Request::Hello { .. } => Response::Error {
                message: "already authenticated".into(),
            },
            Request::List => {
                let sessions = runtime
                    .sessions
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .values()
                    .map(|session| {
                        let state = session.lock();
                        SessionSummary {
                            id: session.id().0.clone(),
                            argv: session.argv.clone(),
                            cols: state.size.cols,
                            rows: state.size.rows,
                            attached: state.attachment.is_some(),
                            exit_code: state.exit_code,
                        }
                    })
                    .collect();
                Response::Sessions { sessions }
            }
            Request::NewSession {
                argv,
                cols,
                rows,
                cwd,
                env,
            } => {
                if argv.is_empty() || cols == 0 || rows == 0 {
                    Response::Error {
                        message: "command and nonzero dimensions are required".into(),
                    }
                } else {
                    let number = runtime
                        .next_id
                        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
                        .map_err(|_| anyhow::anyhow!("session id space exhausted"))?;
                    let id = SessionId(format!("s{number}"));
                    match tokio::task::spawn_blocking(move || {
                        Session::spawn(
                            id,
                            SessionSpec {
                                argv,
                                cwd: cwd.map(PathBuf::from),
                                size: Size { cols, rows },
                                env,
                            },
                        )
                    })
                    .await?
                    {
                        Ok(session) => {
                            let id = session.id().0.clone();
                            runtime
                                .sessions
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .insert(number, Arc::new(session));
                            Response::Created { session: id }
                        }
                        Err(error) => Response::Error {
                            message: error.to_string(),
                        },
                    }
                }
            }
            Request::Kill { session: id } => match lookup(&runtime, &id) {
                Some((number, session)) => match session.kill().await {
                    Ok(()) => {
                        runtime
                            .sessions
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .remove(&number);
                        Response::Ok
                    }
                    Err(error) => Response::Error {
                        message: error.to_string(),
                    },
                },
                None => Response::Error {
                    message: format!("unknown session {id}"),
                },
            },
            Request::Attach {
                session: id,
                force,
                cols,
                rows,
            } => {
                if cols == 0 || rows == 0 {
                    Response::Error {
                        message: "attachment dimensions must be nonzero".into(),
                    }
                } else if let Some((_, session)) = lookup(&runtime, &id) {
                    if attach(&mut connection, session, force, Size { cols, rows }).await? {
                        return Ok(());
                    }
                    continue;
                } else {
                    Response::Error {
                        message: format!("unknown session {id}"),
                    }
                }
            }
        };
        response(&mut connection, result).await?;
    }
    Ok(())
}

fn lookup(runtime: &Runtime, id: &str) -> Option<(u64, Arc<Session>)> {
    let number = id.strip_prefix('s')?.parse::<u64>().ok()?;
    runtime
        .sessions
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&number)
        .filter(|session| session.id().0 == id)
        .map(|session| (number, session.clone()))
}

struct AttachmentGuard {
    session: Arc<Session>,
    generation: u64,
    closed: watch::Sender<bool>,
}

impl Drop for AttachmentGuard {
    fn drop(&mut self) {
        let mut state = self.session.lock();
        if state
            .attachment
            .as_ref()
            .is_some_and(|slot| slot.generation == self.generation)
        {
            state.attachment = None;
        }
        if state.pending_attachment == Some(self.generation) {
            state.pending_attachment = None;
        }
        let _ = self.closed.send(true);
    }
}

async fn attach(
    connection: &mut Framed,
    session: Arc<Session>,
    force: bool,
    size: Size,
) -> anyhow::Result<bool> {
    let (takeover, mut taken_over) = watch::channel(false);
    let (closed, completion) = watch::channel(false);
    let reservation = {
        let mut state = session.lock();
        if (state.attachment.is_some() || state.pending_attachment.is_some()) && !force {
            None
        } else {
            let previous = state
                .attachment
                .as_ref()
                .map(|slot| (slot.takeover.clone(), slot.closed.clone()));
            state.generation += 1;
            let generation = state.generation;
            state.pending_attachment = Some(generation);
            Some((generation, previous))
        }
    };
    let Some((generation, previous)) = reservation else {
        response(
            connection,
            Response::Error {
                message: format!(
                    "session {} is attached elsewhere (use --force to take over)",
                    session.id().0
                ),
            },
        )
        .await?;
        return Ok(false);
    };
    let _guard = AttachmentGuard {
        session: session.clone(),
        generation,
        closed,
    };
    if let Some((previous, mut previous_closed)) = previous {
        let _ = previous.send(true);
        while !*previous_closed.borrow() {
            if previous_closed.changed().await.is_err() {
                break;
            }
        }
    }
    let snapshot = {
        let mut state = session.lock();
        if state.pending_attachment != Some(generation) {
            Err(anyhow::anyhow!("attachment was taken over"))
        } else {
            session.resize_locked(&mut state, size).map(|()| {
                let bytes = state.emulator.render_full();
                state.dirty = false;
                state.attachment = Some(AttachmentSlot {
                    generation,
                    takeover,
                    closed: completion,
                });
                state.pending_attachment = None;
                (bytes, state.exit_code)
            })
        }
    };
    let (bytes, exited) = match snapshot {
        Ok(snapshot) => snapshot,
        Err(error) => {
            response(
                connection,
                Response::Error {
                    message: error.to_string(),
                },
            )
            .await?;
            return Ok(false);
        }
    };
    response(connection, Response::Attached).await?;
    send_data(connection, &bytes).await?;
    if let Some(code) = exited {
        connection.send(&ServerFrame::Exit(code).encode()).await?;
        connection.stream.shutdown().await?;
        return Ok(true);
    }
    loop {
        if *taken_over.borrow() {
            connection
                .send(&ServerFrame::Detached("taken over by another attachment".into()).encode())
                .await?;
            connection.stream.shutdown().await?;
            return Ok(true);
        }
        tokio::select! {
            _ = taken_over.changed() => {},
            _ = session.notify().notified() => {
                let (bytes, exited) = {
                    let mut state = session.lock();
                    let bytes = state.emulator.render_update();
                    if !state.emulator.is_scrolled() { state.dirty = false; }
                    let exited = state.exit_code.filter(|_| !state.dirty);
                    (bytes, exited)
                };
                send_data(connection, &bytes).await?;
                if let Some(code) = exited {
                    connection.send(&ServerFrame::Exit(code).encode()).await?;
                    connection.stream.shutdown().await?;
                    return Ok(true);
                }
            },
            payload = connection.recv() => {
                let Some(payload) = payload? else { return Ok(true); };
                match ClientFrame::decode(&payload)? {
                    ClientFrame::Detach => return Ok(true),
                    ClientFrame::Input(bytes) => {
                        if !session.enqueue(bytes).await? {
                            session.notify().notify_one();
                        }
                    },
                    frame => {
                        let (bytes, exited) = {
                            let mut state = session.lock();
                            match frame {
                                ClientFrame::Resize { cols, rows } if state.exit_code.is_none() =>
                                    session.resize_locked(&mut state, Size { cols, rows })?,
                                ClientFrame::Resize { .. } => {},
                                ClientFrame::Scroll(scroll) => state.emulator.scroll(scroll),
                                ClientFrame::Redraw => {},
                                _ => unreachable!(),
                            }
                            state.dirty = false;
                            (state.emulator.render_full(), state.exit_code)
                        };
                        send_data(connection, &bytes).await?;
                        if let Some(code) = exited {
                            connection.send(&ServerFrame::Exit(code).encode()).await?;
                            connection.stream.shutdown().await?;
                            return Ok(true);
                        }
                    },
                }
            },
        }
    }
}

async fn send_data(connection: &mut Framed, bytes: &[u8]) -> anyhow::Result<()> {
    for chunk in bytes.chunks(64 * 1024) {
        connection.send_tagged(0x01, chunk).await?;
    }
    Ok(())
}
