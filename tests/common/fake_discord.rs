#![allow(clippy::expect_used, clippy::unwrap_used, dead_code)]

use a2amx::discord::{Discord, DiscordConfig};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{Arc, Mutex, mpsc as std_mpsc};
use std::thread;
use std::time::Duration;
use tokio::net::TcpListener as TokioTcpListener;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{WebSocketStream, accept_async};

#[derive(Clone)]
pub struct HttpReply {
    pub status: u16,
    pub body: String,
    pub headers: Vec<(String, String)>,
    pub silent: bool,
}

impl HttpReply {
    pub fn json(status: u16, body: Value) -> Self {
        Self {
            status,
            body: body.to_string(),
            headers: Vec::new(),
            silent: false,
        }
    }

    pub fn text(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            body: body.into(),
            headers: Vec::new(),
            silent: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: String,
}

pub struct HttpFake {
    pub base: String,
    pub requests: Arc<Mutex<Vec<HttpRequest>>>,
    stop: Option<std_mpsc::Sender<()>>,
    join: Option<thread::JoinHandle<()>>,
}

impl Drop for HttpFake {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let _ = TcpStream::connect(self.base.trim_start_matches("http://"));
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

pub fn http_fake(replies: Vec<HttpReply>) -> HttpFake {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{addr}");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let records = Arc::clone(&requests);
    let (stop, stop_rx) = std_mpsc::channel();
    let join = thread::spawn(move || {
        let mut index = 0;
        loop {
            if stop_rx.try_recv().is_ok() {
                break;
            }
            let (mut stream, _) = match listener.accept() {
                Ok(value) => value,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2));
                    continue;
                }
                Err(_) => break,
            };
            let request = read_http_request(&mut stream);
            if let Some(request) = request {
                records.lock().unwrap().push(request);
            }
            let reply = replies
                .get(index)
                .cloned()
                .unwrap_or_else(|| HttpReply::json(200, Value::Null));
            index += 1;
            if reply.silent {
                thread::sleep(Duration::from_secs(2));
                let _ = stream.shutdown(Shutdown::Both);
                continue;
            }
            let mut response = format!(
                "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\n",
                reply.status,
                if reply.status == 429 {
                    "Too Many Requests"
                } else {
                    "OK"
                },
                reply.body.len()
            );
            for (key, value) in &reply.headers {
                response.push_str(&format!("{key}: {value}\r\n"));
            }
            response.push_str("\r\n");
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.write_all(reply.body.as_bytes());
            let _ = stream.shutdown(Shutdown::Both);
        }
    });
    HttpFake {
        base,
        requests,
        stop: Some(stop),
        join: Some(join),
    }
}

pub fn read_http_request(stream: &mut TcpStream) -> Option<HttpRequest> {
    stream.set_read_timeout(Some(Duration::from_secs(1))).ok()?;
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut chunk = [0_u8; 4096];
        let count = stream.read(&mut chunk).ok()?;
        if count == 0 {
            return None;
        }
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
        if bytes.len() > 64 * 1024 {
            return None;
        }
    };
    let header = std::str::from_utf8(&bytes[..header_end]).ok()?;
    let mut lines = header.split("\r\n");
    let mut request_line = lines.next()?.split_whitespace();
    let method = request_line.next()?.to_owned();
    let path = request_line.next()?.to_owned();
    let mut headers = HashMap::new();
    let mut content_length = 0;
    for line in lines.filter(|line| !line.is_empty()) {
        let (key, value) = line.split_once(':')?;
        let value = value.trim().to_owned();
        if key.eq_ignore_ascii_case("content-length") {
            content_length = value.parse().ok()?;
        }
        headers.insert(key.to_ascii_lowercase(), value);
    }
    while bytes.len() < header_end + content_length {
        let mut chunk = [0_u8; 4096];
        let count = stream.read(&mut chunk).ok()?;
        if count == 0 {
            return None;
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    let body = String::from_utf8(bytes[header_end..header_end + content_length].to_vec()).ok()?;
    Some(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

pub fn discord_for_http(fake: &HttpFake) -> Discord {
    let mut config = DiscordConfig::new("test-token".into(), "123".into());
    config.api_base = fake.base.clone();
    config.request_timeout = Duration::from_secs(1);
    Discord::new(config)
}

pub struct WsFake {
    pub url: String,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for WsFake {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub async fn ws_fake<F, Fut>(connections: usize, handler: F) -> WsFake
where
    F: Fn(usize, WebSocketStream<tokio::net::TcpStream>) -> Fut + Send + Sync + Clone + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let listener = TokioTcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("ws://{addr}");
    let task = tokio::spawn(async move {
        for index in 0..connections {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let Ok(socket) = accept_async(stream).await else {
                return;
            };
            let handler = handler.clone();
            tokio::spawn(async move {
                handler(index, socket).await;
            });
        }
    });
    WsFake { url, task }
}

pub async fn send_json<S>(socket: &mut WebSocketStream<S>, value: Value)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .unwrap();
}

pub async fn next_json<S>(socket: &mut WebSocketStream<S>) -> Option<Value>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    while let Some(message) = socket.next().await {
        match message.ok()? {
            Message::Text(text) => return serde_json::from_str(&text).ok(),
            Message::Close(_) => return None,
            _ => {}
        }
    }
    None
}

pub async fn next_close_code<S>(socket: &mut WebSocketStream<S>) -> Option<u16>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    while let Some(message) = socket.next().await {
        if let Message::Close(frame) = message.ok()? {
            return frame.map(|close| u16::from(close.code));
        }
    }
    None
}

pub fn discord_for_ws(url: &str, guild: &str) -> Discord {
    let mut config = DiscordConfig::new("bot-token".into(), guild.into());
    config.gateway_url = url.into();
    config.reconnect_base = Duration::from_millis(1);
    config.identify_gap = Duration::from_millis(1);
    config.stable_after = Duration::from_secs(30);
    Discord::new(config)
}

pub async fn hello<S>(socket: &mut WebSocketStream<S>, interval: u64)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    send_json(socket, json!({"op":10,"d":{"heartbeat_interval":interval}})).await;
}

#[derive(Clone)]
struct FakeChannel {
    id: String,
    kind: u8,
    name: String,
    parent_id: Option<String>,
}

struct FakeState {
    guild_id: String,
    member_count: u64,
    member_count_requests: u64,
    next_channel_id: u64,
    next_message_id: u64,
    fail_channel_creates: bool,
    fail_posts: bool,
    channels: Vec<FakeChannel>,
    posts: HashMap<String, Vec<Value>>,
}
#[derive(Clone)]
struct GatewaySender {
    generation: u64,
    sender: tokio::sync::mpsc::UnboundedSender<Value>,
}
type GatewayClose = (u64, tokio::sync::mpsc::UnboundedSender<()>);

pub struct FakeDiscord {
    pub api_base: String,
    pub gateway_url: String,
    state: Arc<Mutex<FakeState>>,
    gateway_tx: Arc<tokio::sync::Mutex<Option<GatewaySender>>>,
    gateway_close: Arc<tokio::sync::Mutex<Option<GatewayClose>>>,
    gateway_task: tokio::task::JoinHandle<()>,
    http_stop: Option<std_mpsc::Sender<()>>,
    http_join: Option<thread::JoinHandle<()>>,
}

impl FakeDiscord {
    pub async fn new(guild_id: impl Into<String>, member_count: u64) -> Self {
        let guild_id = guild_id.into();
        let state = Arc::new(Mutex::new(FakeState {
            guild_id: guild_id.clone(),
            member_count,
            member_count_requests: 0,
            next_channel_id: 100,
            next_message_id: 1,
            fail_channel_creates: false,
            fail_posts: false,
            channels: Vec::new(),
            posts: HashMap::new(),
        }));
        let (api_base, http_stop, http_join) = start_fake_http(Arc::clone(&state));
        let listener = TokioTcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let gateway_url = format!("ws://{address}");
        let gateway_tx = Arc::new(tokio::sync::Mutex::new(None));
        let gateway_close = Arc::new(tokio::sync::Mutex::new(None));
        let task_tx = Arc::clone(&gateway_tx);
        let task_close = Arc::clone(&gateway_close);
        let task = tokio::spawn(async move {
            let mut generation = 0_u64;
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let Ok(socket) = accept_async(stream).await else {
                    continue;
                };
                let (mut sink, mut source) = socket.split();
                let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
                let (close_tx, mut close_rx) = tokio::sync::mpsc::unbounded_channel();
                let hello = Message::Text(
                    json!({"op": 10, "d": {"heartbeat_interval": 50}})
                        .to_string()
                        .into(),
                );
                if sink.send(hello).await.is_err() {
                    continue;
                }
                let Some(Ok(Message::Text(_identify))) = source.next().await else {
                    continue;
                };
                let ready = Message::Text(
                    json!({
                        "op": 0,
                        "s": 1,
                        "t": "READY",
                        "d": {"session_id": "fake-session"}
                    })
                    .to_string()
                    .into(),
                );
                if sink.send(ready).await.is_err() {
                    continue;
                }
                generation += 1;
                let current = GatewaySender {
                    generation,
                    sender: out_tx,
                };
                *task_tx.lock().await = Some(current);
                *task_close.lock().await = Some((generation, close_tx));
                loop {
                    tokio::select! {
                        Some(value) = out_rx.recv() => {
                            if sink.send(Message::Text(value.to_string().into())).await.is_err() {
                                break;
                            }
                        }
                        Some(()) = close_rx.recv() => {
                            let _ = sink.send(Message::Close(None)).await;
                            break;
                        }
                        message = source.next() => {
                            match message {
                                Some(Ok(Message::Text(text))) => {
                                    if let Ok(value) = serde_json::from_str::<Value>(&text)
                                        && value["op"] == 1
                                        && sink.send(Message::Text(
                                            json!({"op": 11, "d": null}).to_string().into(),
                                        )).await.is_err()
                                    {
                                        break;
                                    }
                                }
                                Some(Ok(Message::Ping(payload))) => {
                                    if sink.send(Message::Pong(payload)).await.is_err() {
                                        break;
                                    }
                                }
                                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                                Some(Ok(_)) => {}
                            }
                        }
                    }
                }
                let mut slot = task_tx.lock().await;
                if slot
                    .as_ref()
                    .is_some_and(|current| current.generation == generation)
                {
                    *slot = None;
                }
                drop(slot);
                let mut close = task_close.lock().await;
                if close
                    .as_ref()
                    .is_some_and(|(current, _)| *current == generation)
                {
                    *close = None;
                }
            }
        });
        Self {
            api_base: api_base.clone(),
            gateway_url: gateway_url.clone(),
            state,
            gateway_tx,
            gateway_close,
            gateway_task: task,
            http_stop: Some(http_stop),
            http_join: Some(http_join),
        }
    }

    pub fn discord_config(&self) -> DiscordConfig {
        let guild_id = self.state.lock().unwrap().guild_id.clone();
        let mut config = DiscordConfig::new("fake-token".into(), guild_id);
        config.api_base = self.api_base.clone();
        config.gateway_url = self.gateway_url.clone();
        config.request_timeout = Duration::from_secs(1);
        config.reconnect_base = Duration::from_millis(5);
        config.identify_gap = Duration::from_millis(1);
        config.stable_after = Duration::from_secs(30);
        config
    }

    pub async fn set_member_count(&self, count: u64) -> u64 {
        let mut state = self.state.lock().unwrap();
        state.member_count = count;
        state.member_count_requests
    }

    pub fn member_count_requests(&self) -> u64 {
        self.state.lock().unwrap().member_count_requests
    }

    pub async fn set_fail_posts(&self, fail: bool) {
        self.state.lock().unwrap().fail_posts = fail;
    }
    pub async fn set_fail_channel_creates(&self, fail: bool) {
        self.state.lock().unwrap().fail_channel_creates = fail;
    }

    pub fn channels(&self) -> Vec<(String, u8, String, Option<String>)> {
        self.state
            .lock()
            .unwrap()
            .channels
            .iter()
            .map(|channel| {
                (
                    channel.id.clone(),
                    channel.kind,
                    channel.name.clone(),
                    channel.parent_id.clone(),
                )
            })
            .collect()
    }

    pub fn posts(&self, channel: &str) -> Vec<Value> {
        self.state
            .lock()
            .unwrap()
            .posts
            .get(channel)
            .cloned()
            .unwrap_or_default()
    }

    pub async fn push_message(
        &self,
        channel: &str,
        author: &str,
        text: &str,
    ) -> anyhow::Result<u64> {
        let id = {
            let mut state = self.state.lock().unwrap();
            let id = state.next_message_id;
            state.next_message_id += 1;
            id
        };
        let value = json!({
            "op": 0,
            "s": id,
            "t": "MESSAGE_CREATE",
            "d": {
                "guild_id": self.state.lock().unwrap().guild_id.clone(),
                "id": id.to_string(),
                "channel_id": channel,
                "author": {"id": author},
                "content": text,
            }
        });
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let sender = self
                .gateway_tx
                .lock()
                .await
                .as_ref()
                .map(|value| value.sender.clone());
            if let Some(sender) = sender
                && sender.send(value.clone()).is_ok()
            {
                return Ok(id);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow::anyhow!("fake Discord gateway is not connected"));
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    pub async fn close_gateway(&self) -> bool {
        let Some((generation, sender)) = self.gateway_close.lock().await.take() else {
            return false;
        };
        if sender.send(()).is_err() {
            return false;
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let active = self
                .gateway_tx
                .lock()
                .await
                .as_ref()
                .map(|value| value.generation);
            if active != Some(generation) {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
}

impl Drop for FakeDiscord {
    fn drop(&mut self) {
        self.gateway_task.abort();
        if let Some(stop) = self.http_stop.take() {
            let _ = stop.send(());
        }
        let _ = TcpStream::connect(self.api_base.trim_start_matches("http://"));
        if let Some(join) = self.http_join.take() {
            let _ = join.join();
        }
    }
}

fn start_fake_http(
    state: Arc<Mutex<FakeState>>,
) -> (String, std_mpsc::Sender<()>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let base = format!("http://{address}");
    let (stop, stop_rx) = std_mpsc::channel();
    let join = thread::spawn(move || {
        loop {
            if stop_rx.try_recv().is_ok() {
                break;
            }
            let (mut stream, _) = match listener.accept() {
                Ok(value) => value,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2));
                    continue;
                }
                Err(_) => break,
            };
            let Some(request) = read_http_request(&mut stream) else {
                continue;
            };
            let (status, body) = fake_http_route(&state, &request);
            let response = format!(
                "HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.shutdown(Shutdown::Both);
        }
    });
    (base, stop, join)
}

fn fake_http_route(state: &Arc<Mutex<FakeState>>, request: &HttpRequest) -> (u16, String) {
    let mut state = state.lock().unwrap();
    let guild_path = format!("/guilds/{}/channels", state.guild_id);
    if request.method == "GET" && request.path == guild_path {
        let channels = state
            .channels
            .iter()
            .map(|channel| {
                json!({
                    "id": channel.id,
                    "type": channel.kind,
                    "name": channel.name,
                    "parent_id": channel.parent_id,
                })
            })
            .collect::<Vec<_>>();
        return (200, Value::Array(channels).to_string());
    }
    let member_path = format!("/guilds/{}?with_counts=true", state.guild_id);
    if request.method == "GET" && request.path == member_path {
        state.member_count_requests += 1;
        return (
            200,
            json!({"approximate_member_count": state.member_count}).to_string(),
        );
    }
    if request.method == "POST" && request.path == guild_path {
        if state.fail_channel_creates {
            return (500, "{}".into());
        }
        let Ok(value) = serde_json::from_str::<Value>(&request.body) else {
            return (400, "{}".into());
        };
        let kind = value["type"].as_u64().unwrap_or_default() as u8;
        let name = value["name"].as_str().unwrap_or_default().to_owned();
        let parent_id = value["parent_id"].as_str().map(str::to_owned);
        let id = state.next_channel_id.to_string();
        state.next_channel_id += 1;
        state.channels.push(FakeChannel {
            id: id.clone(),
            kind,
            name,
            parent_id,
        });
        return (200, json!({"id": id}).to_string());
    }
    if request.method == "POST" && request.path.starts_with("/channels/") {
        if state.fail_posts {
            return (404, "{}".into());
        }
        let channel = request
            .path
            .strip_prefix("/channels/")
            .and_then(|path| path.strip_suffix("/messages"))
            .unwrap_or_default()
            .to_owned();
        let Ok(value) = serde_json::from_str::<Value>(&request.body) else {
            return (400, "{}".into());
        };
        state.posts.entry(channel).or_default().push(value);
        return (200, json!({"id":"posted"}).to_string());
    }
    (404, "{}".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    use a2amx::discord::Discord;

    #[tokio::test]
    async fn push_message_survives_gateway_reconnect() {
        let fake = FakeDiscord::new("123", 2).await;
        let mut watch = Discord::new(fake.discord_config()).watch();

        let first_id = fake.push_message("100", "456", "first").await.unwrap();
        let first = tokio::time::timeout(Duration::from_secs(2), watch.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.message_id, first_id);

        assert!(fake.close_gateway().await);

        let second_id = fake.push_message("100", "456", "second").await.unwrap();
        let second = tokio::time::timeout(Duration::from_secs(2), watch.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second.message_id, second_id);
    }
}
