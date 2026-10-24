//! A small HTTP/1.1 client for Pitwall on `127.0.0.1` only (ADR 0059). Pitwall's contract is plain HTTP on loopback, and
//! Debrief needs two things a general client does not give: Cancel that acts within a fraction of a second even while
//! the server is silent (the socket is read in short slices and closed on cancel, which is how Pitwall's contract
//! cancels a request), and separate limits for connecting, the first byte and the gap between stream chunks.
//!
//! Bounded everywhere: header block, line and body sizes. One request per connection (`Connection: close`); bodies are
//! `Content-Length`, chunked or close-delimited. No redirects, no proxies, no TLS, no other host than 127.0.0.1.

use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long one blocking read waits before Cancel and the deadlines are looked at again.
const SLICE: Duration = Duration::from_millis(50);
/// Largest status line plus headers.
const MAX_HEAD: usize = 64 * 1024;
/// Largest chunk-size line.
const MAX_LINE: usize = 1024;

/// The limits of one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeouts {
    pub connect: Duration,
    /// From sending the request to the first byte of the answer (a cold model load happens here).
    pub first_byte: Duration,
    /// Longest silence between two pieces of the body.
    pub idle: Duration,
}

impl Timeouts {
    /// Quick calls (`/x/v1/health`, pairing, `select`, model lists).
    pub const QUICK: Timeouts = Timeouts {
        connect: Duration::from_secs(1),
        first_byte: Duration::from_secs(3),
        idle: Duration::from_secs(3),
    };
    /// A chat answer: the model may have to be loaded first, then tokens stream.
    pub const CHAT: Timeouts = Timeouts {
        connect: Duration::from_secs(2),
        first_byte: Duration::from_secs(120),
        idle: Duration::from_secs(60),
    };
}

/// Why a request failed below HTTP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpError {
    /// Nothing listens on the port (or it refused): Pitwall is not running.
    Refused(String),
    /// A limit ran out: `what` names it ("connecting", "the first answer", "the next piece of the answer").
    Timeout(&'static str),
    Cancelled,
    /// The connection closed in the middle of an answer.
    Closed,
    /// The server answered something that is not HTTP as expected.
    Protocol(String),
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpError::Refused(e) => write!(f, "nothing answers on the port ({e})"),
            HttpError::Timeout(what) => write!(f, "no answer in time ({what})"),
            HttpError::Cancelled => f.write_str("cancelled"),
            HttpError::Closed => f.write_str("the connection closed in the middle of the answer"),
            HttpError::Protocol(e) => write!(f, "an unexpected answer ({e})"),
        }
    }
}

/// One request.
#[derive(Debug, Clone)]
pub struct Request<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub headers: Vec<(&'a str, String)>,
    /// A JSON body.
    pub body: Option<Vec<u8>>,
}

impl<'a> Request<'a> {
    pub fn get(path: &'a str) -> Request<'a> {
        Request {
            method: "GET",
            path,
            headers: vec![],
            body: None,
        }
    }

    pub fn post(path: &'a str, body: Vec<u8>) -> Request<'a> {
        Request {
            method: "POST",
            path,
            headers: vec![],
            body: Some(body),
        }
    }

    pub fn header(mut self, name: &'a str, value: impl Into<String>) -> Request<'a> {
        self.headers.push((name, value.into()));
        self
    }
}

enum Framing {
    Length(u64),
    Chunked { left: u64, done: bool },
    Close { done: bool },
}

/// An answer whose body is read as it arrives.
pub struct Response {
    pub status: u16,
    headers: Vec<(String, String)>,
    conn: Conn,
    framing: Framing,
}

impl Response {
    /// A header's value (names compare case-insensitively).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// The next decoded piece of the body (empty at the end).
    pub fn next_piece(&mut self) -> Result<Vec<u8>, HttpError> {
        loop {
            match &mut self.framing {
                Framing::Length(0) | Framing::Chunked { done: true, .. } | Framing::Close { done: true } => return Ok(vec![]),
                Framing::Length(left) => {
                    let piece = self.conn.some(*left)?;
                    if piece.is_empty() {
                        return Err(HttpError::Closed);
                    }
                    *left -= piece.len() as u64;
                    return Ok(piece);
                }
                Framing::Close { done } => {
                    let piece = self.conn.some(u64::MAX)?;
                    if piece.is_empty() {
                        *done = true;
                    }
                    return Ok(piece);
                }
                Framing::Chunked { left, done } => {
                    if *left == 0 {
                        let line = self.conn.line(MAX_LINE)?;
                        let size = line.split(';').next().unwrap_or("").trim();
                        let n = u64::from_str_radix(size, 16).map_err(|_| HttpError::Protocol("a bad chunk size".into()))?;
                        if n == 0 {
                            // Trailers end with an empty line.
                            while !self.conn.line(MAX_LINE)?.is_empty() {}
                            *done = true;
                            continue;
                        }
                        *left = n;
                    }
                    let piece = self.conn.some(*left)?;
                    if piece.is_empty() {
                        return Err(HttpError::Closed);
                    }
                    *left -= piece.len() as u64;
                    if *left == 0 && !self.conn.line(MAX_LINE)?.is_empty() {
                        return Err(HttpError::Protocol("a chunk without its line end".into()));
                    }
                    return Ok(piece);
                }
            }
        }
    }

    /// The whole body, at most `limit` bytes.
    pub fn read_all(mut self, limit: usize) -> Result<Vec<u8>, HttpError> {
        let mut out = Vec::new();
        loop {
            let piece = self.next_piece()?;
            if piece.is_empty() {
                return Ok(out);
            }
            if out.len() + piece.len() > limit {
                return Err(HttpError::Protocol(format!("the answer is larger than {limit} bytes")));
            }
            out.extend_from_slice(&piece);
        }
    }
}

struct Conn {
    stream: TcpStream,
    buf: Vec<u8>,
    pos: usize,
    cancel: Option<Arc<AtomicBool>>,
    idle: Duration,
}

impl Drop for Conn {
    fn drop(&mut self) {
        // Closing the connection is how Pitwall hears a cancel.
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

impl Conn {
    fn cancelled(&self) -> bool {
        self.cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed))
    }

    /// Reads more bytes into the buffer, waiting at most `limit`; false at the end of the stream.
    fn fill(&mut self, limit: Duration, what: &'static str) -> Result<bool, HttpError> {
        let deadline = Instant::now() + limit;
        let mut tmp = [0u8; 16 * 1024];
        loop {
            if self.cancelled() {
                return Err(HttpError::Cancelled);
            }
            match self.stream.read(&mut tmp) {
                Ok(0) => return Ok(false),
                Ok(n) => {
                    if self.pos > 0 {
                        self.buf.drain(..self.pos);
                        self.pos = 0;
                    }
                    self.buf.extend_from_slice(&tmp[..n]);
                    return Ok(true);
                }
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted) => {
                    if Instant::now() >= deadline {
                        return Err(HttpError::Timeout(what));
                    }
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted | ErrorKind::BrokenPipe
                    ) =>
                {
                    return Ok(false)
                }
                Err(e) => return Err(HttpError::Protocol(e.to_string())),
            }
        }
    }

    fn available(&self) -> &[u8] {
        &self.buf[self.pos..]
    }

    /// Up to `max` bytes that are already here or arrive next (empty at the end of the stream).
    fn some(&mut self, max: u64) -> Result<Vec<u8>, HttpError> {
        if self.available().is_empty() && !self.fill(self.idle, "the next piece of the answer")? {
            return Ok(vec![]);
        }
        let n = self.available().len().min(usize::try_from(max).unwrap_or(usize::MAX));
        let out = self.available()[..n].to_vec();
        self.pos += n;
        Ok(out)
    }

    /// One CRLF-terminated line (without it), at most `max` bytes.
    fn line(&mut self, max: usize) -> Result<String, HttpError> {
        loop {
            if let Some(i) = self.available().windows(2).position(|w| w == b"\r\n") {
                let line = String::from_utf8_lossy(&self.available()[..i]).into_owned();
                self.pos += i + 2;
                return Ok(line);
            }
            if self.available().len() > max {
                return Err(HttpError::Protocol("a line is too long".into()));
            }
            if !self.fill(self.idle, "the next piece of the answer")? {
                return Err(HttpError::Closed);
            }
        }
    }
}

/// Sends `req` to `127.0.0.1:port` and reads the status and headers. The body is read from the [`Response`].
pub fn send(port: u16, req: &Request<'_>, t: Timeouts, cancel: Option<Arc<AtomicBool>>) -> Result<Response, HttpError> {
    if cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed)) {
        return Err(HttpError::Cancelled);
    }
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let stream = TcpStream::connect_timeout(&addr, t.connect).map_err(|e| match e.kind() {
        ErrorKind::TimedOut | ErrorKind::WouldBlock => HttpError::Timeout("connecting"),
        _ => HttpError::Refused(e.to_string()),
    })?;
    let _ = stream.set_nodelay(true);
    stream
        .set_read_timeout(Some(SLICE))
        .and_then(|_| stream.set_write_timeout(Some(t.connect.max(Duration::from_secs(5)))))
        .map_err(|e| HttpError::Protocol(e.to_string()))?;
    let mut head = format!(
        "{} {} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nAccept-Encoding: identity\r\nUser-Agent: Debrief\r\n",
        req.method, req.path
    );
    for (n, v) in &req.headers {
        // Header values never carry line breaks (a key or a model name with one would split the request).
        if v.contains(['\r', '\n']) || n.contains(['\r', '\n', ':']) {
            return Err(HttpError::Protocol("a header with a line break".into()));
        }
        head.push_str(&format!("{n}: {v}\r\n"));
    }
    if let Some(b) = &req.body {
        head.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n", b.len()));
    }
    head.push_str("\r\n");
    let mut conn = Conn {
        stream,
        buf: Vec::new(),
        pos: 0,
        cancel,
        idle: t.idle,
    };
    let write = |s: &mut TcpStream, bytes: &[u8]| s.write_all(bytes).map_err(|_| HttpError::Closed);
    write(&mut conn.stream, head.as_bytes())?;
    if let Some(b) = &req.body {
        write(&mut conn.stream, b)?;
    }
    let _ = conn.stream.flush();
    // The head: wait for the first byte with the first-byte limit, then read the rest of the head.
    let started = Instant::now();
    loop {
        if conn.available().windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if conn.available().len() > MAX_HEAD {
            return Err(HttpError::Protocol("the answer's headers are too large".into()));
        }
        let left = if conn.available().is_empty() {
            t.first_byte.saturating_sub(started.elapsed())
        } else {
            t.idle
        };
        if !conn.fill(left.max(SLICE), "the first answer")? {
            return Err(if conn.available().is_empty() {
                HttpError::Closed
            } else {
                HttpError::Protocol("the headers were cut off".into())
            });
        }
    }
    let status_line = conn.line(MAX_HEAD)?;
    let mut parts = status_line.splitn(3, ' ');
    let (version, code) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    if !version.starts_with("HTTP/1.") {
        return Err(HttpError::Protocol("not an HTTP answer".into()));
    }
    let status: u16 = code.parse().map_err(|_| HttpError::Protocol("a bad status".into()))?;
    let mut headers = Vec::new();
    loop {
        let line = conn.line(MAX_HEAD)?;
        if line.is_empty() {
            break;
        }
        if headers.len() >= 128 {
            return Err(HttpError::Protocol("too many headers".into()));
        }
        if let Some((n, v)) = line.split_once(':') {
            headers.push((n.trim().to_string(), v.trim().to_string()));
        }
    }
    let find = |n: &str| headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.as_str());
    let framing = if find("transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked")) {
        Framing::Chunked { left: 0, done: false }
    } else if let Some(n) = find("content-length") {
        Framing::Length(n.parse().map_err(|_| HttpError::Protocol("a bad Content-Length".into()))?)
    } else if status == 204 || status == 304 || req.method == "HEAD" {
        Framing::Length(0)
    } else {
        Framing::Close { done: false }
    };
    Ok(Response {
        status,
        headers,
        conn,
        framing,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// Serves one connection with `answer` (written in the given pieces, `pause` apart) and returns the request it read.
    fn serve(pieces: Vec<Vec<u8>>, pause: Duration) -> (u16, std::thread::JoinHandle<String>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let h = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            s.set_read_timeout(Some(Duration::from_millis(300))).unwrap();
            let mut req = vec![0u8; 8192];
            let n = s.read(&mut req).unwrap_or(0);
            for p in pieces {
                if s.write_all(&p).is_err() {
                    break;
                }
                let _ = s.flush();
                std::thread::sleep(pause);
            }
            String::from_utf8_lossy(&req[..n]).into_owned()
        });
        (port, h)
    }

    #[test]
    fn content_length_chunked_and_close_delimited_bodies() {
        let (port, h) = serve(
            vec![b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nX-Pitwall-Model: m\r\n\r\nhello".to_vec()],
            Duration::ZERO,
        );
        let r = send(port, &Request::get("/x").header("Authorization", "Bearer k"), Timeouts::QUICK, None).unwrap();
        assert_eq!((r.status, r.header("x-pitwall-model")), (200, Some("m")));
        assert_eq!(r.read_all(100).unwrap(), b"hello");
        let req = h.join().unwrap();
        assert!(req.starts_with("GET /x HTTP/1.1\r\nHost: 127.0.0.1:"), "{req}");
        assert!(req.contains("Authorization: Bearer k\r\n") && req.contains("Connection: close"));

        let (port, _h) = serve(
            vec![
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n".to_vec(),
                b"2;x=y\r\nde\r\n0\r\nTrailer: t\r\n\r\n".to_vec(),
            ],
            Duration::from_millis(20),
        );
        let r = send(port, &Request::post("/c", b"{}".to_vec()), Timeouts::QUICK, None).unwrap();
        assert_eq!(r.read_all(100).unwrap(), b"abcde");

        let (port, _h) = serve(vec![b"HTTP/1.1 500 Oops\r\n\r\nbody".to_vec()], Duration::ZERO);
        let r = send(port, &Request::get("/"), Timeouts::QUICK, None).unwrap();
        assert_eq!(r.status, 500);
        assert_eq!(r.read_all(100).unwrap(), b"body");

        let (port, _h) = serve(vec![b"HTTP/1.1 200 OK\r\nContent-Length: 50\r\n\r\nshort".to_vec()], Duration::ZERO);
        let r = send(port, &Request::get("/"), Timeouts::QUICK, None).unwrap();
        assert_eq!(r.read_all(100).unwrap_err(), HttpError::Closed);

        let (port, _h) = serve(
            vec![b"HTTP/1.1 200 OK\r\nContent-Length: 500\r\n\r\n".to_vec(), vec![b'x'; 500]],
            Duration::ZERO,
        );
        let r = send(port, &Request::get("/"), Timeouts::QUICK, None).unwrap();
        assert!(matches!(r.read_all(100), Err(HttpError::Protocol(_))));
    }

    #[test]
    fn refused_garbage_and_header_injection() {
        let free = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        assert!(matches!(
            send(free, &Request::get("/"), Timeouts::QUICK, None),
            Err(HttpError::Refused(_))
        ));
        let (port, _h) = serve(vec![b"SSH-2.0-nope\r\n\r\n".to_vec()], Duration::ZERO);
        assert!(matches!(
            send(port, &Request::get("/"), Timeouts::QUICK, None),
            Err(HttpError::Protocol(_))
        ));
        let (port, _h) = serve(
            vec![b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n".to_vec()],
            Duration::ZERO,
        );
        let r = send(port, &Request::get("/"), Timeouts::QUICK, None).unwrap();
        assert!(matches!(r.read_all(10), Err(HttpError::Protocol(_))));
        let (port, _h) = serve(vec![], Duration::ZERO);
        let bad = Request::get("/").header("X", "a\r\nInjected: 1");
        assert!(matches!(send(port, &bad, Timeouts::QUICK, None), Err(HttpError::Protocol(_))));
        // A server that closes without answering.
        let (port, _h) = serve(vec![], Duration::ZERO);
        assert_eq!(send(port, &Request::get("/"), Timeouts::QUICK, None).err(), Some(HttpError::Closed));
        assert!(HttpError::Timeout("x").to_string().contains("in time"));
    }

    #[test]
    fn slow_first_byte_and_stalled_body_time_out_and_cancel_acts_at_once() {
        let quick = Timeouts {
            connect: Duration::from_secs(1),
            first_byte: Duration::from_millis(150),
            idle: Duration::from_millis(150),
        };
        let (port, _h) = serve(vec![b"HTTP/1.1 200 OK\r\n\r\n".to_vec()], Duration::from_millis(400));
        // Nothing arrives at first: the request is read but the answer comes too late.
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let silent = l.local_addr().unwrap().port();
        assert_eq!(
            send(silent, &Request::get("/"), quick, None).err(),
            Some(HttpError::Timeout("the first answer"))
        );
        let mut r = send(port, &Request::get("/"), quick, None).unwrap();
        assert_eq!(r.next_piece().unwrap_err(), HttpError::Timeout("the next piece of the answer"));

        let cancel = Arc::new(AtomicBool::new(false));
        let c = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            c.store(true, Ordering::Relaxed);
        });
        let started = Instant::now();
        let slow = Timeouts {
            first_byte: Duration::from_secs(30),
            ..quick
        };
        assert_eq!(
            send(silent, &Request::get("/"), slow, Some(cancel.clone())).err(),
            Some(HttpError::Cancelled)
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(
            send(silent, &Request::get("/"), slow, Some(cancel)).err(),
            Some(HttpError::Cancelled)
        );
        drop(l);
    }
}
