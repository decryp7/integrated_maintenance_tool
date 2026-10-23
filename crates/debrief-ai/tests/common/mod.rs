//! A tiny HTTP/1.1 mock server on a loopback port (std only), used to test the Hugging Face adapter
//! and downloader without any real network.
#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct Req {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

pub struct Resp {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Send only this many body bytes, then close (a dropped connection).
    pub cut_after: Option<usize>,
    /// Wait this long before answering (a slow server).
    pub delay_ms: u64,
    /// Send the body at about this many bytes per second (a per-connection rate limit).
    pub rate_bps: Option<u64>,
}

impl Resp {
    pub fn new(status: u16, body: impl Into<Vec<u8>>) -> Resp {
        Resp {
            status,
            headers: vec![],
            body: body.into(),
            cut_after: None,
            delay_ms: 0,
            rate_bps: None,
        }
    }
    pub fn json(status: u16, v: serde_json::Value) -> Resp {
        Resp::new(status, v.to_string()).header("content-type", "application/json")
    }
    pub fn header(mut self, k: &str, v: &str) -> Resp {
        self.headers.push((k.into(), v.into()));
        self
    }
    pub fn redirect(to: &str) -> Resp {
        Resp::new(302, "").header("location", to)
    }
}

pub type Handler = Arc<dyn Fn(&Req, u16) -> Resp + Send + Sync>;

pub struct Mock {
    pub port: u16,
    pub requests: Arc<Mutex<Vec<Req>>>,
    stop: Arc<AtomicBool>,
}

impl Mock {
    pub fn start(handler: impl Fn(&Req, u16) -> Resp + Send + Sync + 'static) -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let handler: Handler = Arc::new(handler);
        let (reqs, st) = (requests.clone(), stop.clone());
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                if st.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(conn) = conn else { continue };
                let (h, r) = (handler.clone(), reqs.clone());
                std::thread::spawn(move || serve(conn, h, r, port));
            }
        });
        Mock { port, requests, stop }
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn log(&self) -> Vec<Req> {
        self.requests.lock().unwrap().clone()
    }

    pub fn count(&self, path_prefix: &str) -> usize {
        self.log().iter().filter(|r| r.path.starts_with(path_prefix)).count()
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

fn serve(mut s: TcpStream, h: Handler, reqs: Arc<Mutex<Vec<Req>>>, port: u16) {
    let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(5)));
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let head_end = loop {
        match s.read(&mut tmp) {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.lines();
    let first = lines.next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let (method, path) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
    let headers: HashMap<String, String> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_string()))
        .collect();
    let want: usize = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut body = buf[head_end..].to_vec();
    while body.len() < want {
        match s.read(&mut tmp) {
            Ok(0) | Err(_) => break,
            Ok(n) => body.extend_from_slice(&tmp[..n]),
        }
    }
    let req = Req {
        method,
        path,
        headers,
        body,
    };
    reqs.lock().unwrap().push(req.clone());
    let resp = h(&req, port);
    if resp.delay_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(resp.delay_ms));
    }
    let mut out = format!(
        "HTTP/1.1 {} X\r\nconnection: close\r\ncontent-length: {}\r\n",
        resp.status,
        resp.body.len()
    );
    for (k, v) in &resp.headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str("\r\n");
    let _ = s.write_all(out.as_bytes());
    let body = match resp.cut_after {
        Some(n) => &resp.body[..n.min(resp.body.len())],
        None => &resp.body[..],
    };
    match resp.rate_bps {
        Some(rate) => {
            let slice = 16 * 1024;
            for part in body.chunks(slice) {
                if s.write_all(part).is_err() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_secs_f64(part.len() as f64 / rate as f64));
            }
        }
        None => {
            let _ = s.write_all(body);
        }
    }
    let _ = s.flush();
}

/// Parses `bytes=a-b`.
pub fn range_of(req: &Req) -> Option<(usize, usize)> {
    let r = req.headers.get("range")?.strip_prefix("bytes=")?;
    let (a, b) = r.split_once('-')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

/// A 206 answer for `data` honouring the Range header.
pub fn partial(req: &Req, data: &[u8]) -> Resp {
    match range_of(req) {
        Some((a, b)) if a < data.len() => {
            let b = b.min(data.len() - 1);
            Resp::new(206, data[a..=b].to_vec()).header("content-range", &format!("bytes {a}-{b}/{}", data.len()))
        }
        _ => Resp::new(200, data.to_vec()),
    }
}

pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

pub fn blob(len: usize) -> Vec<u8> {
    let mut x: u32 = 12345;
    (0..len)
        .map(|_| {
            x = x.wrapping_mul(1664525).wrapping_add(1013904223);
            (x >> 24) as u8
        })
        .collect()
}

// ---- the sample workspace ----

use debrief_core::ports::Store;
use debrief_core::workspace::WorkspaceStore;
use debrief_core::{Profile, Workspace};
use std::path::PathBuf;
use std::sync::OnceLock;

pub fn samples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples")
}

/// The generated incident in /samples, opened once behind the store ports.
pub fn store() -> &'static (dyn Store + 'static) {
    static STORE: OnceLock<Box<dyn Store>> = OnceLock::new();
    STORE
        .get_or_init(|| {
            let text = std::fs::read_to_string(samples().join("profiles/orderservice.debriefprofile.toml")).unwrap();
            let mut ws = Workspace::new(Profile::from_toml(&text).unwrap()).unwrap();
            ws.open(&[samples()]).unwrap();
            Box::new(WorkspaceStore::new(ws))
        })
        .as_ref()
}

pub fn ms(h: u32, m: u32, s: u32) -> i64 {
    chrono_free_ms(2026, 10, 14, h, m, s)
}

fn chrono_free_ms(y: i64, mo: i64, d: i64, h: u32, mi: u32, s: u32) -> i64 {
    // days from civil (Howard Hinnant)
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (mo + if mo > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    (days * 86400 + h as i64 * 3600 + mi as i64 * 60 + s as i64) * 1000
}

/// The first lines of the log4net sample, as a user would paste them.
pub fn billing_sample(n: usize) -> String {
    let text = std::fs::read_to_string(samples().join("log4net/legacy-billing/billing.log.2026-10-14")).unwrap();
    text.lines().take(n).collect::<Vec<_>>().join("\n")
}

pub fn first_id(ql: &str) -> u32 {
    use debrief_core::api::QueryRequest;
    use debrief_core::ql::QlResult;
    use debrief_core::query::Query;
    match store()
        .run_query(&QueryRequest {
            query: Query {
                ql: ql.into(),
                ..Default::default()
            },
        })
        .unwrap()
    {
        QlResult::Events { ids, .. } => ids[0],
        _ => panic!("expected events"),
    }
}
