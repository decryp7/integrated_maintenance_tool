//! A minimal synchronous DevTools-protocol client: one thread owns the WebSocket, callers send a command and wait
//! for its reply; events go to a bounded log that callers wait on.

use crate::{Error, Result};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tungstenite::client::IntoClientRequest;
use tungstenite::protocol::WebSocketConfig;
use tungstenite::Message;

const MAX_MESSAGE: usize = 32 << 20;
const MAX_EVENTS: usize = 1024;

#[derive(Debug, Clone)]
pub struct Event {
    pub method: String,
    pub session: Option<String>,
    pub params: Value,
}

#[derive(Default)]
struct Events {
    seq: u64,
    log: VecDeque<(u64, Event)>,
}

struct Inner {
    pending: Mutex<HashMap<u64, Sender<std::result::Result<Value, String>>>>,
    events: Mutex<Events>,
    cv: Condvar,
    closed: AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub struct Connection {
    out: Sender<String>,
    inner: Arc<Inner>,
    next: AtomicU64,
}

impl Connection {
    /// Connects to `ws://127.0.0.1:<port><path>` (loopback only).
    pub fn connect(port: u16, path: &str) -> Result<Connection> {
        let stream = TcpStream::connect(("127.0.0.1", port)).map_err(|e| Error::Connection(e.to_string()))?;
        let _ = stream.set_nodelay(true);
        let req = format!("ws://127.0.0.1:{port}{path}")
            .into_client_request()
            .map_err(|e| Error::Connection(e.to_string()))?;
        let mut cfg = WebSocketConfig::default();
        cfg.max_message_size = Some(MAX_MESSAGE);
        cfg.max_frame_size = Some(MAX_MESSAGE);
        let (ws, _) = tungstenite::client::client_with_config(req, stream, Some(cfg)).map_err(|e| Error::Connection(e.to_string()))?;
        let _ = ws.get_ref().set_read_timeout(Some(Duration::from_millis(20)));
        let inner = Arc::new(Inner {
            pending: Mutex::new(HashMap::new()),
            events: Mutex::new(Events::default()),
            cv: Condvar::new(),
            closed: AtomicBool::new(false),
        });
        let (out, rx) = channel();
        let i2 = inner.clone();
        std::thread::Builder::new()
            .name("cdp-io".into())
            .spawn(move || io_loop(ws, rx, i2))
            .map_err(|e| Error::Connection(e.to_string()))?;
        Ok(Connection {
            out,
            inner,
            next: AtomicU64::new(1),
        })
    }

    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::Acquire)
    }

    /// Sends a command and waits for its reply.
    pub fn call(&self, session: Option<&str>, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        if self.is_closed() {
            return Err(Error::Closed);
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = channel();
        lock(&self.inner.pending).insert(id, tx);
        let mut msg = json!({"id": id, "method": method, "params": params});
        if let Some(s) = session {
            msg["sessionId"] = json!(s);
        }
        if self.out.send(msg.to_string()).is_err() {
            lock(&self.inner.pending).remove(&id);
            return Err(Error::Closed);
        }
        match rx.recv_timeout(timeout) {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(m)) => Err(Error::Protocol(m)),
            Err(_) => {
                lock(&self.inner.pending).remove(&id);
                if self.is_closed() {
                    Err(Error::Closed)
                } else {
                    Err(Error::Timeout("a command"))
                }
            }
        }
    }

    /// The sequence number to pass to [`Connection::wait_event`] so only later events count.
    pub fn mark(&self) -> u64 {
        lock(&self.inner.events).seq
    }

    /// Waits for an event after `since` that `pred` accepts.
    pub fn wait_event(&self, since: u64, timeout: Duration, pred: impl Fn(&Event) -> bool) -> Option<Event> {
        let end = Instant::now() + timeout;
        let mut g = lock(&self.inner.events);
        loop {
            if let Some((_, e)) = g.log.iter().find(|(s, e)| *s > since && pred(e)) {
                return Some(e.clone());
            }
            if self.is_closed() {
                return None;
            }
            let left = end.checked_duration_since(Instant::now())?;
            g = self.inner.cv.wait_timeout(g, left).unwrap_or_else(|e| e.into_inner()).0;
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.inner.closed.store(true, Ordering::Release);
    }
}

type Ws = tungstenite::WebSocket<TcpStream>;

fn io_loop(mut ws: Ws, rx: Receiver<String>, inner: Arc<Inner>) {
    'outer: while !inner.closed.load(Ordering::Acquire) {
        while let Ok(m) = rx.try_recv() {
            if ws.send(Message::text(m)).is_err() {
                break 'outer;
            }
        }
        match ws.read() {
            Ok(Message::Text(t)) => dispatch(&inner, t.as_str()),
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(_) => break,
        }
    }
    inner.closed.store(true, Ordering::Release);
    lock(&inner.pending).clear();
    inner.cv.notify_all();
    let _ = ws.close(None);
}

fn dispatch(inner: &Inner, text: &str) {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return };
    if let Some(id) = v.get("id").and_then(Value::as_u64) {
        if let Some(tx) = lock(&inner.pending).remove(&id) {
            let r = match v.get("error") {
                Some(e) => Err(e.get("message").and_then(Value::as_str).unwrap_or("error").to_string()),
                None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
            };
            let _ = tx.send(r);
        }
    } else if let Some(m) = v.get("method").and_then(Value::as_str) {
        let mut g = lock(&inner.events);
        g.seq += 1;
        let seq = g.seq;
        g.log.push_back((
            seq,
            Event {
                method: m.to_string(),
                session: v.get("sessionId").and_then(Value::as_str).map(str::to_string),
                params: v.get("params").cloned().unwrap_or(Value::Null),
            },
        ));
        if g.log.len() > MAX_EVENTS {
            g.log.pop_front();
        }
        drop(g);
        inner.cv.notify_all();
    }
}
