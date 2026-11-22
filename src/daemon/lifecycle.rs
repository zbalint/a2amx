use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::net::SocketAddr;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use anyhow::Context;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::{JoinHandle, JoinSet};

use super::omp_usage::OmpUsage;
use super::registry::Sessions;
use super::remote::Leases;
use super::serve::serve;
use super::{Daemon, DaemonConfig, OMP_USAGE_CHECK, Runtime, current_exe, random_hex};
use crate::confine;
use crate::layout::Layout;
use crate::messaging;
use crate::session::mark_inherited_fds_cloexec;
use crate::store::{self, Store};

impl Daemon {
    /// Bind every listen address, write the chosen addresses to a file in the
    /// a2amx directory, write the admin token file (mode 0600), and start serving.
    pub async fn start(config: DaemonConfig) -> anyhow::Result<Self> {
        mark_inherited_fds_cloexec()?;
        let exe = tokio::task::spawn_blocking(current_exe).await??;
        let bwrap_path = config
            .bwrap_path
            .clone()
            .or_else(|| std::env::var_os("PATH"))
            .unwrap_or_default();
        let env_path = config
            .env_path
            .clone()
            .unwrap_or_else(|| PathBuf::from(confine::ENV_PATH));
        let layout = Layout::new(config.root.clone());
        layout.ensure()?;
        let lock = acquire_lock(&layout).await?;
        let host_name = config.host_name;
        let (token, gateway_token, host_name, boot) =
            tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
                let token = random_hex::<32>()?;
                let gateway_token = random_hex::<32>()?;
                let host = match host_name {
                    Some(host) => {
                        messaging::validate_host(&host).context("invalid host name")?;
                        host
                    }
                    None => messaging::default_host_name(),
                };
                Ok((token, gateway_token, host, random_hex::<8>()?))
            })
            .await??;
        let store = Store::open(&layout, config.limits).await?;
        let (bound, addrs) = match publish(&layout, &token, &gateway_token, config.listen).await {
            Ok(published) => published,
            Err(error) => return Err(rollback(&layout, error).await),
        };
        // shortcut: sessions live only in this daemon; add persistence only with
        // a later lifecycle specification.
        let (shutdown, _) = watch::channel(false);
        let runtime = Arc::new(Runtime {
            sessions: Sessions::new(),
            token,
            gateway_token,
            store,
            host_name,
            boot,
            limits: config.limits,
            address: addrs[0].to_string(),
            layout: layout.clone(),
            bwrap_path,
            env_path,
            exe,
            omp_usage: OmpUsage::new(),
            // shortcut: accepted-send timestamps are memory-only; persist them if
            // rate limits must survive daemon restarts.
            rate: tokio::sync::Mutex::new(HashMap::new()),
            presence: Arc::new(Mutex::new(HashMap::new())),
            leases: Leases::new(),
            gateway_connections: AtomicUsize::new(0),
            human_attempts: Mutex::new(HashMap::new()),
            deliveries: Mutex::new(Vec::new()),
            stop_requested: tokio::sync::Notify::new(),
            stop_now: AtomicBool::new(false),
        });
        let purge = spawn_purge(runtime.store.clone(), shutdown.subscribe());
        let omp_usage_poller = spawn_omp_usage(runtime.clone(), shutdown.subscribe());
        let presence = {
            let runtime = runtime.clone();
            let stopping = shutdown.subscribe();
            tokio::spawn(runtime.poll_presence(stopping))
        };
        let listeners = bound
            .into_iter()
            .map(|listener| spawn_listener(listener, runtime.clone(), shutdown.subscribe()))
            .collect();
        Ok(Self {
            addrs,
            runtime,
            shutdown,
            listeners,
            purge: Some(purge),
            omp_usage_poller: Some(omp_usage_poller),
            presence: Some(presence),
            layout,
            lock: Some(lock),
        })
    }

    /// Addresses actually bound (resolves port 0).
    pub fn addrs(&self) -> &[SocketAddr] {
        &self.addrs
    }

    pub fn host_name(&self) -> &str {
        &self.runtime.host_name
    }

    /// Completes when an admin client asked the daemon to shut down.
    pub async fn stop_requested(&self) {
        self.runtime.stop_requested.notified().await;
    }

    pub async fn shutdown(mut self) -> anyhow::Result<()> {
        let _ = self.shutdown.send(true);
        self.stop_tasks().await?;
        let mut failure = self.stop_deliveries().await;
        if let Some(error) = self.stop_sessions().await {
            failure.get_or_insert(error);
        }
        let addr = self.layout.addr();
        let link_addr = self.layout.link_addr();
        let lock = self.lock.take();
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            remove_state_files(&[&addr, &link_addr])?;
            drop(lock);
            Ok(())
        })
        .await??;
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    async fn stop_tasks(&mut self) -> anyhow::Result<()> {
        for listener in self.listeners.drain(..) {
            listener.await?;
        }
        if let Some(purge) = self.purge.take() {
            purge.await?;
        }
        if let Some(omp_usage_poller) = self.omp_usage_poller.take() {
            omp_usage_poller.await?;
        }
        if let Some(presence) = self.presence.take() {
            presence.await?;
        }
        Ok(())
    }

    async fn stop_deliveries(&self) -> Option<anyhow::Error> {
        let mut failure = None;
        let deliveries = std::mem::take(
            &mut *self
                .runtime
                .deliveries
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        // Stop injection before killing children so graceful shutdown, rather
        // than a racing recipient-exit observer, owns the remaining open rows.
        for delivery in &deliveries {
            delivery.abort();
        }
        for delivery in deliveries {
            if let Err(error) = delivery.await
                && !error.is_cancelled()
            {
                failure.get_or_insert(anyhow::Error::new(error).context("message delivery task"));
            }
        }
        failure
    }

    async fn stop_sessions(&self) -> Option<anyhow::Error> {
        let mut failure = None;
        let sessions = self.runtime.sessions.snapshot();
        let graceful = !self.runtime.stop_now.load(Ordering::Relaxed);
        let mut kills = JoinSet::new();
        for session in sessions {
            kills.spawn(async move { session.kill(graceful).await });
        }
        while let Some(result) = kills.join_next().await {
            if let Err(error) = result
                .context("session shutdown task")
                .and_then(|value| value)
            {
                failure.get_or_insert(error);
            }
        }
        self.runtime.sessions.clear();
        if let Err(error) = self
            .runtime
            .store
            .fail_open(&self.runtime.boot, None, "daemon_stopped")
            .await
        {
            failure.get_or_insert(error);
        }
        if let Err(error) = self.runtime.store.close().await {
            failure.get_or_insert(error);
        }
        failure
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        for delivery in self
            .runtime
            .deliveries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
        {
            delivery.abort();
        }
    }
}

async fn acquire_lock(layout: &Layout) -> anyhow::Result<File> {
    let lock_path = layout.daemon_lock();
    tokio::task::spawn_blocking(move || -> anyhow::Result<File> {
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(lock_path)?;
        rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive).map_err(
            |_| anyhow::anyhow!("another a2amx daemon is running for this a2amx directory"),
        )?;
        Ok(lock)
    })
    .await?
}

async fn publish(
    layout: &Layout,
    token: &str,
    gateway_token: &str,
    listen: Vec<SocketAddr>,
) -> anyhow::Result<(Vec<TcpListener>, Vec<SocketAddr>)> {
    // Written only after the steps that can fail without binding, so an early
    // startup error never leaves a token for a daemon that is not running.
    let gateway_path = layout.gateway_token();
    let link_gateway_path = layout.link_gateway_token();
    let token_path = layout.admin_token();
    let gateway_text = gateway_token.to_owned();
    let token_text = token.to_owned();
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        write_state_file(&gateway_path, gateway_text.as_bytes())?;
        write_state_file(&link_gateway_path, gateway_text.as_bytes())?;
        write_state_file(&token_path, token_text.as_bytes())?;
        Ok(())
    })
    .await??;
    let mut bound = Vec::new();
    let mut addrs = Vec::new();
    let listen = if listen.is_empty() {
        vec![SocketAddr::from(([127, 0, 0, 1], 0))]
    } else {
        listen
    };
    for address in listen {
        let listener = TcpListener::bind(address).await?;
        addrs.push(listener.local_addr()?);
        bound.push(listener);
    }
    let path = layout.addr();
    let link_path = layout.link_addr();
    let addresses = addrs
        .iter()
        .map(|addr| format!("{addr}\n"))
        .collect::<String>();
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        write_state_file(&path, addresses.as_bytes())?;
        write_state_file(&link_path, addresses.as_bytes())?;
        Ok(())
    })
    .await??;
    Ok((bound, addrs))
}

async fn rollback(layout: &Layout, error: anyhow::Error) -> anyhow::Error {
    let admin = layout.admin_token();
    let gateway = layout.gateway_token();
    let link_gateway = layout.link_gateway_token();
    let addr = layout.addr();
    let link_addr = layout.link_addr();
    let removed = tokio::task::spawn_blocking(move || {
        remove_state_files(&[&admin, &gateway, &link_gateway, &addr, &link_addr])
    })
    .await
    .map_err(anyhow::Error::from)
    .and_then(|result| result);
    match removed {
        Ok(()) => error,
        Err(cleanup) => {
            anyhow::anyhow!("{error}; failed to remove state files after a failed start: {cleanup}")
        }
    }
}

fn spawn_purge(store: Store, mut stopping: watch::Receiver<bool>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(3600));
        interval.tick().await;
        loop {
            tokio::select! {
                _ = stopping.changed() => break,
                _ = interval.tick() => {
                    if let Err(error) = store.purge(store::now()).await {
                        tracing::error!(%error, "message purge failed");
                    }
                }
            }
        }
    })
}

fn spawn_omp_usage(runtime: Arc<Runtime>, mut stopping: watch::Receiver<bool>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(OMP_USAGE_CHECK);
        loop {
            tokio::select! {
                _ = stopping.changed() => break,
                _ = interval.tick() => {
                    if runtime.omp_usage.poll(
                        runtime.omp_usage_command(),
                        runtime.bwrap_path.as_os_str(),
                        &mut stopping,
                    ).await
                    {
                        break;
                    }
                }
            }
        }
    })
}

fn spawn_listener(
    listener: TcpListener,
    runtime: Arc<Runtime>,
    mut stopping: watch::Receiver<bool>,
) -> JoinHandle<()> {
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
}

fn remove_state_files(paths: &[&Path]) -> anyhow::Result<()> {
    let mut first_error = None;
    for path in paths {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                first_error.get_or_insert_with(|| {
                    anyhow::anyhow!("cannot remove state file {}: {error}", path.display())
                });
            }
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
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
