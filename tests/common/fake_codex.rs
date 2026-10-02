//! A fake Codex app-server: the WebSocket-over-unix-socket JSON-RPC subset the daemon
//! uses, with state the test can read and change.

use std::path::Path;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

#[derive(Clone, Debug, PartialEq)]
pub struct TurnStart {
    pub thread: String,
    pub client_id: String,
    pub text: String,
}

struct Thread {
    id: String,
    status: Value,
    updated_at: i64,
}

#[derive(Default)]
struct State {
    threads: Vec<Thread>,
    items: Vec<(String, Value)>,
    turn_starts: Vec<TurnStart>,
    auto_enter: bool,
    reject_turns: Option<String>,
    connections: usize,
    served: Vec<tokio::task::JoinHandle<()>>,
}

pub struct FakeCodex {
    state: Arc<Mutex<State>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for FakeCodex {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FakeCodex {
    pub fn bind(path: &Path) -> Self {
        let listener = UnixListener::bind(path).expect("bind fake app-server socket");
        let state = Arc::new(Mutex::new(State::default()));
        let shared = state.clone();
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let handle = tokio::spawn(serve(stream, shared.clone()));
                let mut state = shared.lock().unwrap();
                state.connections += 1;
                state.served.push(handle);
            }
        });
        Self { state, task }
    }

    /// Replaces the loaded threads; later entries are more recently updated.
    pub fn set_threads(&self, threads: &[(&str, Value)]) {
        self.state.lock().unwrap().threads = threads
            .iter()
            .enumerate()
            .map(|(index, (id, status))| Thread {
                id: (*id).to_owned(),
                status: status.clone(),
                updated_at: index as i64 + 1,
            })
            .collect();
    }

    pub fn set_status(&self, thread: &str, status: Value) {
        for known in &mut self.state.lock().unwrap().threads {
            if known.id == thread {
                known.status = status.clone();
            }
        }
    }

    /// Whether `turn/start` makes the message appear in the thread at once.
    pub fn auto_enter(&self, on: bool) {
        self.state.lock().unwrap().auto_enter = on;
    }

    /// Makes `turn/start` fail with this JSON-RPC error message.
    pub fn reject_turns(&self, message: &str) {
        self.state.lock().unwrap().reject_turns = Some(message.to_owned());
    }

    /// Puts a user message into the thread as the harness would after accepting it.
    pub fn enter(&self, thread: &str, client_id: &str, text: &str) {
        self.state
            .lock()
            .unwrap()
            .items
            .push((thread.to_owned(), user_message(client_id, text)));
    }

    pub fn turn_starts(&self) -> Vec<TurnStart> {
        self.state.lock().unwrap().turn_starts.clone()
    }

    /// Drops every connection and refuses new ones, as a crashed app-server would.
    pub fn stop(&self) {
        self.task.abort();
        for handle in self.state.lock().unwrap().served.drain(..) {
            handle.abort();
        }
    }

    pub fn connections(&self) -> usize {
        self.state.lock().unwrap().connections
    }
}

fn user_message(client_id: &str, text: &str) -> Value {
    json!({
        "type": "userMessage",
        "id": format!("item-{client_id}"),
        "clientId": client_id,
        "content": [{"type": "text", "text": text, "text_elements": []}],
    })
}

async fn serve(mut stream: UnixStream, state: Arc<Mutex<State>>) {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte).await {
            Ok(1) => head.push(byte[0]),
            _ => return,
        }
    }
    let reply = "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: unchecked\r\n\r\n";
    if stream.write_all(reply.as_bytes()).await.is_err() {
        return;
    }
    while let Some(request) = read_message(&mut stream).await {
        let Some(id) = request.get("id").cloned() else {
            continue;
        };
        let method = request["method"].as_str().unwrap_or_default().to_owned();
        let response = match answer(&state, &method, &request["params"]) {
            Ok(result) => json!({"id": id, "result": result}),
            Err(message) => json!({"id": id, "error": {"code": -32600, "message": message}}),
        };
        // A notification first proves the client skips what it did not ask for.
        let noise = json!({"method": "thread/status/changed", "params": {}});
        for value in [noise, response] {
            if write_message(&mut stream, &value).await.is_err() {
                return;
            }
        }
    }
}

fn answer(state: &Mutex<State>, method: &str, params: &Value) -> Result<Value, String> {
    let mut state = state.lock().unwrap();
    match method {
        "initialize" => Ok(json!({"userAgent": "fake", "codexHome": "/nowhere"})),
        "thread/loaded/list" => Ok(json!({
            "data": state.threads.iter().map(|t| t.id.clone()).collect::<Vec<_>>(),
            "nextCursor": null,
        })),
        "thread/read" => {
            let wanted = params["threadId"].as_str().unwrap_or_default();
            let thread = state
                .threads
                .iter()
                .find(|t| t.id == wanted)
                .ok_or("no such thread")?;
            Ok(json!({"thread": {
                "id": thread.id,
                "status": thread.status,
                "updatedAt": thread.updated_at,
            }}))
        }
        "thread/items/list" => {
            let wanted = params["threadId"].as_str().unwrap_or_default();
            let data: Vec<Value> = state
                .items
                .iter()
                .rev()
                .filter(|(thread, _)| thread == wanted)
                .map(|(_, item)| json!({"turnId": "turn-1", "item": item}))
                .collect();
            Ok(json!({"data": data, "nextCursor": null, "backwardsCursor": null}))
        }
        "turn/start" => {
            if let Some(message) = &state.reject_turns {
                return Err(message.clone());
            }
            let thread = params["threadId"].as_str().unwrap_or_default().to_owned();
            let client_id = params["clientUserMessageId"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let text = params["input"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            state.turn_starts.push(TurnStart {
                thread: thread.clone(),
                client_id: client_id.clone(),
                text: text.clone(),
            });
            if state.auto_enter {
                state.items.push((thread, user_message(&client_id, &text)));
            }
            Ok(json!({"turn": {"id": "turn-1", "items": [], "status": "inProgress"}}))
        }
        other => Err(format!("unsupported method {other}")),
    }
}

async fn read_message(stream: &mut UnixStream) -> Option<Value> {
    let mut header = [0u8; 2];
    stream.read_exact(&mut header).await.ok()?;
    let masked = header[1] & 0x80 != 0;
    let mut length = usize::from(header[1] & 0x7f);
    if length == 126 {
        let mut bytes = [0u8; 2];
        stream.read_exact(&mut bytes).await.ok()?;
        length = usize::from(u16::from_be_bytes(bytes));
    } else if length == 127 {
        let mut bytes = [0u8; 8];
        stream.read_exact(&mut bytes).await.ok()?;
        length = usize::try_from(u64::from_be_bytes(bytes)).ok()?;
    }
    let mut mask = [0u8; 4];
    if masked {
        stream.read_exact(&mut mask).await.ok()?;
    }
    let mut payload = vec![0u8; length];
    stream.read_exact(&mut payload).await.ok()?;
    if masked {
        for (index, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask[index % 4];
        }
    }
    if header[0] & 0x0f == 0x8 {
        return None;
    }
    serde_json::from_slice(&payload).ok()
}

async fn write_message(stream: &mut UnixStream, value: &Value) -> std::io::Result<()> {
    let payload = serde_json::to_vec(value)?;
    let mut frame = vec![0x81];
    if payload.len() < 126 {
        frame.push(payload.len() as u8);
    } else if payload.len() <= usize::from(u16::MAX) {
        frame.push(126);
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        frame.push(127);
        frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    frame.extend_from_slice(&payload);
    stream.write_all(&frame).await
}
