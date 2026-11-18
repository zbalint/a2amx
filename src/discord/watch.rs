use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::gateway::Event;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest, protocol::CloseFrame};
use tokio_tungstenite::{Connector, WebSocketStream, client_async_tls_with_config};

use super::{DiscordConfig, DiscordError, Fatal};

const FRAME_LIMIT: usize = 1024 * 1024;
const INTENTS: u64 = 1 | 512 | 32_768;

pub struct Watch {
    receiver: mpsc::Receiver<Event>,
    state: Arc<WatchState>,
    handle: Option<tokio::task::JoinHandle<()>>,
    terminal: Option<Fatal>,
}

impl Watch {
    pub(super) fn start(config: Arc<DiscordConfig>) -> Self {
        let capacity = config.queue.max(1);
        let (sender, receiver) = mpsc::channel(capacity);
        let state = Arc::new(WatchState {
            fatal: Mutex::new(None),
        });
        let task_state = Arc::clone(&state);
        let handle = tokio::spawn(async move {
            run_watch(config, sender, task_state).await;
        });
        Watch {
            receiver,
            state,
            handle: Some(handle),
            terminal: None,
        }
    }

    pub async fn recv(&mut self) -> Result<crate::gateway::Event, Fatal> {
        if let Some(error) = &self.terminal {
            return Err(error.clone());
        }
        match self.receiver.recv().await {
            Some(event) => Ok(event),
            None => {
                let error = self
                    .state
                    .fatal
                    .lock()
                    .ok()
                    .and_then(|value| value.clone())
                    .unwrap_or_else(|| Fatal("gateway task ended".into()));
                self.terminal = Some(error.clone());
                Err(error)
            }
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

struct WatchState {
    fatal: Mutex<Option<Fatal>>,
}

#[derive(Debug)]
enum Outcome {
    Fatal(String),
    Resume,
    Reidentify,
    Transient,
    TransientFresh,
}

#[derive(Clone)]
struct Session {
    id: String,
    resume_url: Option<String>,
    resume_seq: Option<u64>,
}

struct ConnectionState {
    session: Option<Session>,
    last_seq: Option<u64>,
    last_identify: Option<Instant>,
    backoff: u32,
}

async fn run_watch(
    config: Arc<DiscordConfig>,
    sender: mpsc::Sender<Event>,
    state: Arc<WatchState>,
) {
    let mut connection = ConnectionState {
        session: None,
        last_seq: None,
        last_identify: None,
        backoff: 0,
    };
    loop {
        let started = Instant::now();
        let outcome = run_connection(&config, &sender, &mut connection).await;
        let stable = started.elapsed() >= config.stable_after;
        let wait_exponent = match outcome {
            Outcome::Fatal(message) => {
                set_fatal(&state, Fatal(message));
                return;
            }
            Outcome::Reidentify => {
                connection.session = None;
                connection.last_seq = None;
                let exponent = connection.backoff;
                connection.backoff = if stable {
                    0
                } else {
                    exponent.saturating_add(1)
                };
                if stable { 0 } else { exponent }
            }
            Outcome::TransientFresh => {
                connection.session = None;
                connection.last_seq = None;
                let exponent = connection.backoff;
                connection.backoff = exponent.saturating_add(1);
                exponent
            }
            Outcome::Resume | Outcome::Transient => {
                let exponent = connection.backoff;
                if stable {
                    connection.backoff = 0;
                    0
                } else {
                    connection.backoff = exponent.saturating_add(1);
                    exponent
                }
            }
        };
        let wait = backoff_duration(config.reconnect_base, wait_exponent, config.reconnect_max);
        tokio::time::sleep(wait).await;
    }
}

fn set_fatal(state: &WatchState, fatal: Fatal) {
    if let Ok(mut slot) = state.fatal.lock() {
        *slot = Some(fatal);
    }
}

type GatewaySocket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

async fn run_connection(
    config: &DiscordConfig,
    sender: &mpsc::Sender<Event>,
    connection: &mut ConnectionState,
) -> Outcome {
    let target = connection
        .session
        .as_ref()
        .and_then(|session| session.resume_url.as_deref())
        .filter(|url| valid_resume_url(config.gateway_url.as_str(), url))
        .unwrap_or(config.gateway_url.as_str());
    let target = gateway_url(target);
    let attempt = tokio::time::timeout(config.request_timeout, connect_and_hello(&target)).await;
    let (mut socket, interval) = match attempt {
        Ok(Ok(value)) => value,
        Ok(Err(outcome)) => return outcome,
        Err(_) => return Outcome::Transient,
    };
    if let Err(outcome) = identify_or_resume(config, &mut socket, connection).await {
        return outcome;
    }
    connection_loop(config, sender, connection, &mut socket, interval).await
}

async fn connect_and_hello(target: &str) -> Result<(GatewaySocket, Duration), Outcome> {
    let host_port = url_host_port(target).ok_or(Outcome::Transient)?;
    let request = target
        .to_owned()
        .into_client_request()
        .map_err(|_| Outcome::Transient)?;
    let ws_config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(FRAME_LIMIT))
        .max_frame_size(Some(FRAME_LIMIT));
    let connector = rustls_connector().map_err(|_| Outcome::Transient)?;
    let stream = tokio::net::TcpStream::connect((host_port.0.as_str(), host_port.1))
        .await
        .map_err(|_| Outcome::Transient)?;
    let (mut socket, _) =
        client_async_tls_with_config(request, stream, Some(ws_config), Some(connector))
            .await
            .map_err(|_| Outcome::Transient)?;
    let interval = receive_hello(&mut socket).await?;
    Ok((socket, interval))
}

async fn receive_hello<S>(socket: &mut WebSocketStream<S>) -> Result<Duration, Outcome>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    while let Some(message) = socket.next().await {
        match message {
            Ok(Message::Text(text)) => {
                let Ok(value) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                if value.get("op").and_then(Value::as_u64) != Some(10) {
                    continue;
                }
                let Some(interval) = value
                    .get("d")
                    .and_then(|data| data.get("heartbeat_interval"))
                    .and_then(Value::as_u64)
                else {
                    return Err(Outcome::Transient);
                };
                return Ok(Duration::from_millis(interval.max(1)));
            }
            Ok(Message::Close(frame)) => {
                return Err(classify_close(frame.as_ref().map(close_code)));
            }
            Ok(_) => {}
            Err(_) => return Err(Outcome::Transient),
        }
    }
    Err(Outcome::Transient)
}

async fn identify_or_resume<S>(
    config: &DiscordConfig,
    socket: &mut WebSocketStream<S>,
    connection: &mut ConnectionState,
) -> Result<(), Outcome>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    if let Some(session) = connection.session.as_ref() {
        let payload = json!({
            "op": 6,
            "d": {"token": config.token, "session_id": session.id, "seq": session.resume_seq}
        });
        socket
            .send(Message::Text(payload.to_string().into()))
            .await
            .map_err(|_| Outcome::Transient)?;
        return Ok(());
    }
    if let Some(last) = connection.last_identify {
        let elapsed = last.elapsed();
        if elapsed < config.identify_gap {
            tokio::time::sleep(config.identify_gap - elapsed).await;
        }
    }
    let payload = json!({
        "op": 2,
        "d": {
            "token": config.token,
            "intents": INTENTS,
            "properties": {"os": "linux", "browser": "a2amx", "device": "a2amx"}
        }
    });
    socket
        .send(Message::Text(payload.to_string().into()))
        .await
        .map_err(|_| Outcome::Transient)?;
    connection.last_identify = Some(Instant::now());
    Ok(())
}

async fn connection_loop<S>(
    config: &DiscordConfig,
    sender: &mpsc::Sender<Event>,
    connection: &mut ConnectionState,
    socket: &mut WebSocketStream<S>,
    interval: Duration,
) -> Outcome
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let jitter = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| (duration.subsec_nanos() as f64) / 1_000_000_000.0)
        .unwrap_or(0.0);
    let mut heartbeat = Box::pin(tokio::time::sleep(interval.mul_f64(jitter)));
    let mut acknowledged = true;
    let mut waiting: VecDeque<(u64, crate::gateway::Event)> = VecDeque::new();
    loop {
        tokio::select! {
            permit = sender.reserve(), if !waiting.is_empty() => {
                let Ok(permit) = permit else { return Outcome::Transient; };
                let Some((sequence, event)) = waiting.pop_front() else { continue; };
                permit.send(event);
                if let Some(session) = connection.session.as_mut() {
                    session.resume_seq = Some(sequence);
                }
            }
            _ = &mut heartbeat => {
                if !acknowledged {
                    let _ = socket.send(Message::Close(Some(CloseFrame {
                        code: tungstenite_close_code(4000),
                        reason: "heartbeat not acknowledged".into(),
                    }))).await;
                    return Outcome::Resume;
                }
                if socket.send(Message::Text(heartbeat_payload(connection.last_seq).into())).await.is_err() {
                    return Outcome::Transient;
                }
                acknowledged = false;
                heartbeat.as_mut().reset(tokio::time::Instant::now() + interval);
            }
            message = socket.next() => {
                match message {
                    Some(Ok(Message::Text(text))) => {
                        let Ok(value) = serde_json::from_str::<Value>(&text) else { continue; };
                        match decode_op(config, &value) {
                            OpAction::Dispatch { sequence, action } => {
                                if let Some(sequence) = sequence {
                                    connection.last_seq = Some(sequence);
                                }
                                match action {
                                    DispatchAction::Ready { session_id, resume_url } => {
                                        if let Some(session_id) = session_id {
                                            connection.session = Some(Session {
                                                id: session_id,
                                                resume_url,
                                                resume_seq: connection.last_seq,
                                            });
                                        }
                                        if waiting.is_empty()
                                            && let Some(session) = connection.session.as_mut()
                                        {
                                            session.resume_seq = connection.last_seq;
                                        }
                                    }
                                    DispatchAction::Message(event) => {
                                        let Some(event) = event else {
                                            if waiting.is_empty()
                                                && let Some(session) = connection.session.as_mut()
                                            {
                                                session.resume_seq = connection.last_seq;
                                            }
                                            continue;
                                        };
                                        let sequence = sequence.unwrap_or_default();
                                        if waiting.is_empty() {
                                            match sender.try_send(event) {
                                                Ok(()) => {
                                                    if let Some(session) = connection.session.as_mut() {
                                                        session.resume_seq = Some(sequence);
                                                    }
                                                }
                                                Err(mpsc::error::TrySendError::Full(event)) => waiting.push_back((sequence, event)),
                                                Err(mpsc::error::TrySendError::Closed(_)) => return Outcome::Transient,
                                            }
                                        } else {
                                            if waiting.len() >= config.queue.max(1) {
                                                let _ = socket.send(Message::Close(Some(CloseFrame {
                                                    code: tungstenite_close_code(4000),
                                                    reason: "event queue full".into(),
                                                }))).await;
                                                waiting.clear();
                                                return Outcome::Resume;
                                            }
                                            waiting.push_back((sequence, event));
                                        }
                                    }
                                    DispatchAction::Other => {
                                        if waiting.is_empty()
                                            && let Some(session) = connection.session.as_mut()
                                        {
                                            session.resume_seq = connection.last_seq;
                                        }
                                    }
                                }
                            }
                            OpAction::Heartbeat => {
                                if socket.send(Message::Text(heartbeat_payload(connection.last_seq).into())).await.is_err() {
                                    return Outcome::Transient;
                                }
                                acknowledged = false;
                            }
                            OpAction::Reconnect => return Outcome::Resume,
                            OpAction::InvalidSession(resumable) => {
                                if resumable {
                                    return Outcome::Resume;
                                }
                                return Outcome::Reidentify;
                            }
                            OpAction::Ack => acknowledged = true,
                            OpAction::Ignore | OpAction::Hello => {}
                        }
                    }
                    Some(Ok(Message::Close(frame))) => return classify_close(frame.as_ref().map(close_code)),
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => return Outcome::Transient,
                }
            }
        }
    }
}

#[derive(Debug)]
enum OpAction {
    Hello,
    Dispatch {
        sequence: Option<u64>,
        action: DispatchAction,
    },
    Heartbeat,
    Reconnect,
    InvalidSession(bool),
    Ack,
    Ignore,
}

#[derive(Debug)]
enum DispatchAction {
    Ready {
        session_id: Option<String>,
        resume_url: Option<String>,
    },
    Message(Option<crate::gateway::Event>),
    Other,
}

fn decode_op(config: &DiscordConfig, value: &Value) -> OpAction {
    let sequence = value.get("s").and_then(Value::as_u64);
    match value.get("op").and_then(Value::as_u64) {
        Some(0) => OpAction::Dispatch {
            sequence,
            action: decode_dispatch(config, value),
        },
        Some(1) => OpAction::Heartbeat,
        Some(7) => OpAction::Reconnect,
        Some(9) => {
            OpAction::InvalidSession(value.get("d").and_then(Value::as_bool).unwrap_or(false))
        }
        Some(10) => OpAction::Hello,
        Some(11) => OpAction::Ack,
        _ => OpAction::Ignore,
    }
}

fn decode_dispatch(config: &DiscordConfig, value: &Value) -> DispatchAction {
    let event = value.get("t").and_then(Value::as_str);
    match event {
        Some("READY") => DispatchAction::Ready {
            session_id: value
                .get("d")
                .and_then(|data| data.get("session_id"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            resume_url: value
                .get("d")
                .and_then(|data| data.get("resume_gateway_url"))
                .and_then(Value::as_str)
                .map(str::to_owned),
        },
        Some("MESSAGE_CREATE") => {
            DispatchAction::Message(parse_message_event(config, value.get("d")))
        }
        _ => DispatchAction::Other,
    }
}

fn parse_message_event(
    config: &DiscordConfig,
    value: Option<&Value>,
) -> Option<crate::gateway::Event> {
    let value = value?.as_object()?;
    let guild_id = value.get("guild_id")?.as_str()?;
    if guild_id != config.guild_id {
        return None;
    }
    let channel = value.get("channel_id")?.as_str()?.to_owned();
    let message_id = value.get("id")?.as_str()?.parse::<u64>().ok()?;
    let author = value.get("author")?.get("id")?.as_str()?.to_owned();
    let text = value.get("content")?.as_str()?.to_owned();
    Some(crate::gateway::Event {
        channel,
        message_id,
        author,
        text,
    })
}

fn heartbeat_payload(sequence: Option<u64>) -> String {
    json!({"op": 1, "d": sequence}).to_string()
}

fn close_code(frame: &CloseFrame) -> u16 {
    u16::from(frame.code)
}

fn classify_close(code: Option<u16>) -> Outcome {
    match code {
        Some(4004) => Outcome::Fatal("gateway close 4004: authentication failed".into()),
        Some(4010) => Outcome::Fatal("gateway close 4010: invalid shard".into()),
        Some(4011) => Outcome::Fatal("gateway close 4011: sharding required".into()),
        Some(4012) => Outcome::Fatal("gateway close 4012: invalid API version".into()),
        Some(4013) => Outcome::Fatal("gateway close 4013: invalid intents".into()),
        Some(4014) => Outcome::Fatal("gateway close 4014: disallowed intents".into()),
        Some(4007 | 4009) => Outcome::TransientFresh,
        _ => Outcome::Transient,
    }
}

fn backoff_duration(base: Duration, exponent: u32, cap: Duration) -> Duration {
    let factor = 1_u32.checked_shl(exponent.min(30)).unwrap_or(u32::MAX);
    let value = base.checked_mul(factor).unwrap_or(cap);
    value.min(cap)
}

fn gateway_url(base: &str) -> String {
    if base.contains('?') {
        format!("{base}&v=10&encoding=json")
    } else if base.ends_with('/') {
        format!("{base}?v=10&encoding=json")
    } else {
        format!("{base}/?v=10&encoding=json")
    }
}

fn valid_resume_url(base: &str, candidate: &str) -> bool {
    let Some((base_scheme, base_host)) = url_parts(base) else {
        return false;
    };
    let Some((candidate_scheme, candidate_host)) = url_parts(candidate) else {
        return false;
    };
    candidate_scheme == base_scheme
        && (candidate_host == base_host || candidate_host.ends_with(".discord.gg"))
}

fn url_parts(url: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit('@').next()?;
    let host = if authority.starts_with('[') {
        authority.split(']').next()?.trim_start_matches('[')
    } else {
        authority.split(':').next()?
    };
    (!scheme.is_empty() && !host.is_empty()).then_some((scheme, host))
}
fn url_host_port(url: &str) -> Option<(String, u16)> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?.rsplit('@').next()?;
    let (host, port) = if authority.starts_with('[') {
        let end = authority.find(']')?;
        let host = authority[1..end].to_owned();
        let port = authority
            .get(end + 1..)?
            .strip_prefix(':')
            .and_then(|value| value.parse::<u16>().ok());
        (host, port)
    } else {
        let (host, port) = authority.rsplit_once(':').unwrap_or((authority, ""));
        let port = (!port.is_empty())
            .then(|| port.parse::<u16>().ok())
            .flatten();
        (host.to_owned(), port)
    };
    let port = port.unwrap_or(match scheme {
        "ws" => 80,
        "wss" => 443,
        _ => return None,
    });
    (!host.is_empty()).then_some((host, port))
}

fn rustls_connector() -> Result<Connector, DiscordError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|_| DiscordError::Transport("tls config"))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Connector::Rustls(Arc::new(config)))
}
fn tungstenite_close_code(
    code: u16,
) -> tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode {
    tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::from(code)
}
