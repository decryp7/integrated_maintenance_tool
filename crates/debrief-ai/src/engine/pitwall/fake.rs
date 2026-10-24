//! A fake Pitwall server for tests (ADR 0059): contract-shaped answers on an ephemeral 127.0.0.1 port, scriptable
//! (pairing approved or denied, another `apiVersion`, a server that dies mid-stream, slow answers, typed errors). It
//! never uses Pitwall's real port, data folder or credential store. Not part of the app's behaviour; tests only.

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// Request headers, names lowercased.
pub type Headers = Vec<(String, String)>;
/// One request the fake saw: method, path, headers, body.
pub type Seen = (String, String, Headers, String);

/// How the fake answers a chat.
#[derive(Debug, Clone)]
pub enum ChatScript {
    /// Streams these pieces, then `finish_reason`.
    Answer { pieces: Vec<String>, finish: String },
    /// Streams these pieces, then drops the connection (Pitwall stopped).
    DieAfter(Vec<String>),
    /// Answers with this status and an OpenAI-shaped error.
    Fail { status: u16, code: String, message: String },
    /// Sends an error event in the middle of the stream.
    ErrorEvent(String),
}

/// How the fake answers pairing polls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairMode {
    Approve(String),
    Deny,
    Pending,
}

/// The fake's script and what it saw.
#[derive(Debug, Clone)]
pub struct FakeState {
    pub version: String,
    pub api_version: u32,
    pub serving: String,
    /// Keys it accepts.
    pub keys: Vec<String>,
    pub pair: PairMode,
    /// Polls answered `pending` before the outcome.
    pub pending_polls: usize,
    /// (id, capabilities, contextMax)
    pub models: Vec<(String, Vec<String>, u64)>,
    /// Whether the `debrief` preset is listed.
    pub preset: bool,
    pub chat: ChatScript,
    /// Pause before the first byte of a chat answer and between pieces.
    pub chat_delay: Duration,
    pub piece_delay: Duration,
    /// Every request: (method, path, headers lowercased, body).
    pub seen: Vec<Seen>,
}

impl Default for FakeState {
    fn default() -> Self {
        FakeState {
            version: "1.0.4".into(),
            api_version: 1,
            serving: "serving".into(),
            keys: vec![],
            pair: PairMode::Approve("pw_fakekey".into()),
            pending_polls: 1,
            models: vec![("qwen3-8b-q4km".into(), vec!["chat".into(), "json".into(), "tools".into()], 32768)],
            preset: false,
            chat: ChatScript::Answer {
                pieces: vec!["Hello".into(), " from".into(), " Pitwall.".into()],
                finish: "stop".into(),
            },
            chat_delay: Duration::ZERO,
            piece_delay: Duration::ZERO,
            seen: vec![],
        }
    }
}

/// A running fake server.
pub struct FakePitwall {
    pub port: u16,
    state: Arc<Mutex<FakeState>>,
    stop: Arc<AtomicBool>,
}

impl Drop for FakePitwall {
    fn drop(&mut self) {
        self.kill();
    }
}

impl FakePitwall {
    pub fn start(state: FakeState) -> FakePitwall {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let port = listener.local_addr().expect("local address").port();
        listener.set_nonblocking(true).expect("non-blocking");
        let state = Arc::new(Mutex::new(state));
        let stop = Arc::new(AtomicBool::new(false));
        let (s, st) = (state.clone(), stop.clone());
        std::thread::spawn(move || {
            while !st.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((conn, _)) => {
                        let (s, st) = (s.clone(), st.clone());
                        std::thread::spawn(move || serve(conn, &s, &st));
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(5)),
                }
            }
        });
        FakePitwall { port, state, stop }
    }

    pub fn state(&self) -> MutexGuard<'_, FakeState> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Writes a `discovery.json` for this server into `dir` with process id `pid`.
    pub fn write_discovery(&self, dir: &Path, pid: u32, api_version: u32) {
        let port = self.port;
        std::fs::create_dir_all(dir).expect("discovery folder");
        let text = json!({"format": 1, "version": self.state().version, "apiVersion": api_version, "pid": pid, "port": port,
            "baseUrl": format!("http://127.0.0.1:{port}"), "startedAt": 1});
        std::fs::write(dir.join(super::discovery::FILE_NAME), text.to_string()).expect("discovery file");
    }

    /// Stops listening and drops the connections it is serving (Pitwall quit).
    pub fn kill(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// The requests to `path` it saw.
    pub fn requests_to(&self, path: &str) -> Vec<(String, Headers, String)> {
        self.state()
            .seen
            .iter()
            .filter(|(_, p, _, _)| p == path)
            .map(|(m, _, h, b)| (m.clone(), h.clone(), b.clone()))
            .collect()
    }
}

fn read_request(conn: &mut TcpStream) -> Option<Seen> {
    conn.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        let n = conn.read(&mut tmp).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let (method, path) = (first.next()?.to_string(), first.next()?.to_string());
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let len: usize = headers
        .iter()
        .find(|(n, _)| n == "content-length")
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < len {
        let n = conn.read(&mut tmp).ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    Some((method, path, headers, String::from_utf8_lossy(&body).into_owned()))
}

fn respond(conn: &mut TcpStream, status: u16, extra: &[(&str, String)], body: &Value) {
    let text = body.to_string();
    let mut head = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
        text.len()
    );
    for (n, v) in extra {
        head.push_str(&format!("{n}: {v}\r\n"));
    }
    head.push_str("Connection: close\r\n\r\n");
    let _ = conn.write_all(head.as_bytes());
    let _ = conn.write_all(text.as_bytes());
}

fn error(conn: &mut TcpStream, status: u16, code: &str, message: &str) {
    respond(
        conn,
        status,
        &[],
        &json!({"error": {"message": message, "type": "x", "code": code}}),
    );
}

fn chunk(conn: &mut TcpStream, data: &str) -> bool {
    let frame = format!("data: {data}\n\n");
    conn.write_all(format!("{:x}\r\n{frame}\r\n", frame.len()).as_bytes()).is_ok() && conn.flush().is_ok()
}

fn serve(mut conn: TcpStream, state: &Mutex<FakeState>, stop: &AtomicBool) {
    let Some((method, path, headers, body)) = read_request(&mut conn) else {
        return;
    };
    let lock = || state.lock().unwrap_or_else(|p| p.into_inner());
    lock().seen.push((method.clone(), path.clone(), headers.clone(), body.clone()));
    let key = headers
        .iter()
        .find(|(n, _)| n == "authorization")
        .and_then(|(_, v)| v.strip_prefix("Bearer "))
        .map(str::to_string);
    let authed = key.as_ref().is_some_and(|k| lock().keys.contains(k));
    let s = lock().clone();
    match (method.as_str(), path.as_str()) {
        ("GET", "/x/v1/health") => respond(
            &mut conn,
            200,
            &[],
            &json!({"ok": true, "name": "pitwall", "version": s.version, "apiVersion": s.api_version, "serving": s.serving, "offline": false, "loadedModels": 0}),
        ),
        ("POST", "/x/v1/pair") => respond(
            &mut conn,
            202,
            &[],
            &json!({"pairingId": "pr1-abc", "code": "482913", "expiresInSecs": 300}),
        ),
        ("GET", p) if p.starts_with("/x/v1/pair/") => {
            let answer = {
                let mut st = lock();
                if st.pending_polls > 0 {
                    st.pending_polls -= 1;
                    json!({"status": "pending"})
                } else {
                    match st.pair.clone() {
                        PairMode::Approve(k) => {
                            st.keys.push(k.clone());
                            // Delivered once.
                            st.pair = PairMode::Pending;
                            json!({"status": "approved", "key": k})
                        }
                        PairMode::Deny => json!({"status": "denied"}),
                        PairMode::Pending => json!({"status": "pending"}),
                    }
                }
            };
            respond(&mut conn, 200, &[], &answer);
        }
        _ if !authed => error(&mut conn, 401, "unauthorized", "a valid Pitwall API key is required"),
        ("GET", "/v1/models") => {
            let mut data: Vec<Value> = s.models.iter().map(|(id, _, _)| json!({"id": id, "object": "model"})).collect();
            if s.preset {
                data.push(json!({"id": "debrief", "object": "model"}));
            }
            respond(&mut conn, 200, &[], &json!({"object": "list", "data": data}));
        }
        ("GET", "/x/v1/models") => {
            let models: Vec<Value> = s
                .models
                .iter()
                .map(|(id, caps, ctx)| json!({"id": id, "capabilities": caps, "unavailable": [], "contextMax": ctx, "sizeBytes": 1, "loaded": false}))
                .collect();
            respond(&mut conn, 200, &[], &json!({ "models": models }));
        }
        ("POST", "/x/v1/select") => {
            let spec = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| v.get("model").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_default();
            let vision = spec.contains("vision");
            let pick = s.models.iter().find(|(_, caps, _)| !vision || caps.iter().any(|c| c == "vision"));
            match pick {
                Some((id, _, _)) => respond(
                    &mut conn,
                    200,
                    &[],
                    &json!({"model": id, "chosenBy": "capabilities", "loaded": false}),
                ),
                None => error(&mut conn, 404, "not_found", "no installed model has chat"),
            }
        }
        ("POST", "/v1/chat/completions") => {
            std::thread::sleep(s.chat_delay);
            if stop.load(Ordering::Relaxed) {
                return;
            }
            let model = s.models.first().map(|m| m.0.clone()).unwrap_or_default();
            let pieces = match &s.chat {
                ChatScript::Fail { status, code, message } => return error(&mut conn, *status, code, message),
                ChatScript::Answer { pieces, .. } | ChatScript::DieAfter(pieces) => pieces.clone(),
                ChatScript::ErrorEvent(_) => vec!["partial".into()],
            };
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nX-Pitwall-Model: {model}\r\nX-Pitwall-Chosen-By: capabilities\r\nConnection: close\r\n\r\n"
            );
            let _ = conn.write_all(head.as_bytes());
            let role = json!({"choices": [{"index": 0, "delta": {"role": "assistant", "content": ""}, "finish_reason": null}]});
            if !chunk(&mut conn, &role.to_string()) {
                return;
            }
            for p in pieces {
                std::thread::sleep(s.piece_delay);
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                let c = json!({"choices": [{"index": 0, "delta": {"content": p}, "finish_reason": null}]});
                if !chunk(&mut conn, &c.to_string()) {
                    return;
                }
            }
            match &s.chat {
                ChatScript::DieAfter(_) => {
                    let _ = conn.shutdown(std::net::Shutdown::Both);
                }
                ChatScript::ErrorEvent(m) => {
                    chunk(
                        &mut conn,
                        &json!({"error": {"message": m, "type": "server_error", "code": "engine_error"}}).to_string(),
                    );
                    let _ = conn.write_all(b"0\r\n\r\n");
                }
                ChatScript::Answer { finish, .. } => {
                    let end = json!({"choices": [{"index": 0, "delta": {}, "finish_reason": finish}],
                        "usage": {"prompt_tokens": 11, "completion_tokens": 3, "total_tokens": 14}});
                    chunk(&mut conn, &end.to_string());
                    chunk(&mut conn, "[DONE]");
                    let _ = conn.write_all(b"0\r\n\r\n");
                }
                ChatScript::Fail { .. } => {}
            }
        }
        _ => error(&mut conn, 404, "not_found", "no such route"),
    }
}
