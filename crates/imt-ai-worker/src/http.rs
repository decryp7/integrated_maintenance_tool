//! A deliberately small HTTP/1.1 client for talking to the local llama-server on 127.0.0.1: one request
//! per connection, JSON bodies, `Content-Length` or chunked responses. Hand-written rather than a library
//! because every read must be interruptible: the socket polls with a short read timeout so a cancelled
//! chat closes the connection at once (which is how llama-server learns to stop) and a dead server is
//! noticed. Every read is bounded (header size, body size, line length) and nothing here can panic on
//! what the server sends.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

/// How often a blocked read wakes up to check cancellation and deadlines.
pub const TICK: Duration = Duration::from_millis(50);
const MAX_HEADER_BYTES: usize = 64 * 1024;

#[derive(Debug)]
pub enum HttpError {
    /// Nothing listens on the port (the server is not up, or has gone).
    Connect(io::Error),
    Io(io::Error),
    TimedOut,
    Cancelled,
    TooLarge(&'static str),
    Protocol(String),
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpError::Connect(e) => write!(f, "cannot connect to llama-server: {e}"),
            HttpError::Io(e) => write!(f, "the connection to llama-server failed: {e}"),
            HttpError::TimedOut => write!(f, "llama-server did not answer in time"),
            HttpError::Cancelled => write!(f, "cancelled"),
            HttpError::TooLarge(what) => write!(f, "llama-server sent a {what} that is too large"),
            HttpError::Protocol(m) => write!(f, "llama-server sent a malformed answer: {m}"),
        }
    }
}

/// A complete (non-streamed) response.
#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Response {
    pub fn json(&self) -> Option<serde_json::Value> {
        serde_json::from_slice(&self.body).ok()
    }
}

/// The address and key of one running llama-server.
#[derive(Debug, Clone)]
pub struct Client {
    pub port: u16,
    pub key: String,
}

/// Reads from the socket in short slices so cancellation and deadlines are honoured mid-read.
struct Ticking<'a> {
    stream: TcpStream,
    /// When no byte has arrived for this long, the read fails with `TimedOut`.
    idle: Duration,
    last: Instant,
    cancelled: &'a dyn Fn() -> bool,
}

const CANCELLED_MARK: &str = "imt:cancelled";
const TIMEOUT_MARK: &str = "imt:timeout";

impl Read for Ticking<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if (self.cancelled)() {
                return Err(io::Error::other(CANCELLED_MARK));
            }
            match self.stream.read(buf) {
                Ok(n) => {
                    self.last = Instant::now();
                    return Ok(n);
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
                    ) =>
                {
                    if self.last.elapsed() >= self.idle {
                        return Err(io::Error::other(TIMEOUT_MARK));
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }
}

fn classify(e: io::Error) -> HttpError {
    match e.get_ref().map(|i| i.to_string()) {
        Some(m) if m == CANCELLED_MARK => HttpError::Cancelled,
        Some(m) if m == TIMEOUT_MARK => HttpError::TimedOut,
        _ if e.kind() == io::ErrorKind::InvalidData => HttpError::Protocol(e.to_string()),
        _ => HttpError::Io(e),
    }
}

/// How the body is delimited.
enum Framing {
    Length(u64),
    Chunked { left: u64, done: bool },
    UntilClose,
}

/// The response body, decoded from its framing, over the buffered connection `R`.
pub struct Body<R> {
    inner: R,
    framing: Framing,
}

fn bad(m: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, m.into())
}

fn read_crlf_line(r: &mut impl BufRead, limit: usize) -> io::Result<String> {
    let mut line = Vec::new();
    r.take(limit as u64 + 1).read_until(b'\n', &mut line)?;
    if line.len() > limit {
        return Err(bad("line too long"));
    }
    while matches!(line.last(), Some(b'\n' | b'\r')) {
        line.pop();
    }
    String::from_utf8(line).map_err(|_| bad("not UTF-8"))
}

impl<R: BufRead> Read for Body<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        match &mut self.framing {
            Framing::Length(left) => {
                if *left == 0 {
                    return Ok(0);
                }
                let want = buf.len().min(usize::try_from(*left).unwrap_or(usize::MAX));
                let n = self.inner.read(&mut buf[..want])?;
                if n == 0 {
                    return Err(bad("the body ended early"));
                }
                *left -= n as u64;
                Ok(n)
            }
            Framing::UntilClose => self.inner.read(buf),
            Framing::Chunked { left, done } => {
                if *done {
                    return Ok(0);
                }
                if *left == 0 {
                    let line = read_crlf_line(&mut self.inner, 1024)?;
                    let size = line.split(';').next().unwrap_or("").trim();
                    let size = u64::from_str_radix(size, 16).map_err(|_| bad(format!("bad chunk size {size:?}")))?;
                    if size == 0 {
                        // trailers, up to the empty line
                        while !read_crlf_line(&mut self.inner, 8192)?.is_empty() {}
                        *done = true;
                        return Ok(0);
                    }
                    *left = size;
                }
                let want = buf.len().min(usize::try_from(*left).unwrap_or(usize::MAX));
                let n = self.inner.read(&mut buf[..want])?;
                if n == 0 {
                    return Err(bad("the chunk ended early"));
                }
                *left -= n as u64;
                if *left == 0 {
                    read_crlf_line(&mut self.inner, 2)?;
                }
                Ok(n)
            }
        }
    }
}

impl<R: BufRead> Body<R> {
    /// The next line of the body without its line ending; `None` at the end. Longer lines are an error.
    pub fn next_line(&mut self, limit: usize) -> Result<Option<String>, HttpError> {
        let mut line = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            // Body is not buffered itself, but its inner reader is, so byte reads are cheap.
            match self.read(&mut byte).map_err(classify)? {
                0 if line.is_empty() => return Ok(None),
                0 => break,
                _ if byte[0] == b'\n' => break,
                _ => {
                    if line.len() >= limit {
                        return Err(HttpError::TooLarge("line"));
                    }
                    line.push(byte[0]);
                }
            }
        }
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        Ok(Some(String::from_utf8_lossy(&line).into_owned()))
    }

    /// The whole body, at most `limit` bytes.
    pub fn read_all(&mut self, limit: usize) -> Result<Vec<u8>, HttpError> {
        let mut out = Vec::new();
        self.take(limit as u64 + 1).read_to_end(&mut out).map_err(classify)?;
        if out.len() > limit {
            return Err(HttpError::TooLarge("response"));
        }
        Ok(out)
    }
}

/// Options of one request.
pub struct Call<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub body: Option<&'a [u8]>,
    /// Fails when no byte arrives for this long.
    pub idle: Duration,
    pub cancelled: &'a dyn Fn() -> bool,
}

impl Client {
    /// Sends the request and returns the status and the body reader.
    pub fn open<'a>(&self, call: &Call<'a>) -> Result<(u16, Body<impl BufRead + 'a>), HttpError> {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, self.port));
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(HttpError::Connect)?;
        stream.set_read_timeout(Some(TICK)).map_err(HttpError::Io)?;
        stream.set_write_timeout(Some(Duration::from_secs(30))).map_err(HttpError::Io)?;
        let _ = stream.set_nodelay(true);
        let body = call.body.unwrap_or_default();
        let mut head = format!(
            "{} {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\nAccept-Encoding: identity\r\nConnection: close\r\n",
            call.method, call.path, self.port, self.key
        );
        if call.body.is_some() {
            head.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n", body.len()));
        }
        head.push_str("\r\n");
        let mut w = &stream;
        w.write_all(head.as_bytes())
            .and_then(|_| w.write_all(body))
            .map_err(HttpError::Io)?;
        let inner = BufReader::new(Ticking {
            stream,
            idle: call.idle,
            last: Instant::now(),
            cancelled: call.cancelled,
        });
        read_head(inner)
    }

    /// A complete request: status and body (at most `limit` bytes).
    pub fn send(&self, call: &Call<'_>, limit: usize) -> Result<Response, HttpError> {
        let (status, mut body) = self.open(call)?;
        let body = body.read_all(limit)?;
        Ok(Response { status, body })
    }
}

/// Reads the status line and the headers of a response from `inner` and returns the status and the body reader
/// (the parser of everything the server sends; fuzzed by `fuzz_worker_http`).
pub fn read_head<R: BufRead>(mut inner: R) -> Result<(u16, Body<R>), HttpError> {
    let status_line = read_crlf_line(&mut inner, 1024).map_err(classify)?;
    let status = parse_status(&status_line).ok_or_else(|| HttpError::Protocol(format!("bad status line {status_line:?}")))?;
    let (mut length, mut chunked, mut seen) = (None, false, 0usize);
    loop {
        let line = read_crlf_line(&mut inner, 8192).map_err(classify)?;
        seen += line.len() + 2;
        if seen > MAX_HEADER_BYTES {
            return Err(HttpError::TooLarge("header"));
        }
        if line.is_empty() {
            break;
        }
        let Some((k, v)) = line.split_once(':') else { continue };
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
        if k == "content-length" {
            length = Some(v.parse::<u64>().map_err(|_| HttpError::Protocol("bad Content-Length".into()))?);
        } else if k == "transfer-encoding" && v.to_ascii_lowercase().contains("chunked") {
            chunked = true;
        }
    }
    let framing = match (chunked, length) {
        (true, _) => Framing::Chunked { left: 0, done: false },
        (false, Some(n)) => Framing::Length(n),
        (false, None) => Framing::UntilClose,
    };
    Ok((status, Body { inner, framing }))
}

fn parse_status(line: &str) -> Option<u16> {
    let mut parts = line.split_whitespace();
    parts.next().filter(|v| v.starts_with("HTTP/1."))?;
    parts.next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn never() -> bool {
        false
    }

    #[test]
    fn head_parser_decodes_chunks_and_refuses_bad_input() {
        let ok = b"HTTP/1.1 200 OK
Transfer-Encoding: chunked

3
ab

2
cd
0

";
        let (status, mut body) = read_head(&ok[..]).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body.next_line(16).unwrap().as_deref(), Some("ab"));
        assert_eq!(body.read_all(16).unwrap(), b"cd");
        assert!(matches!(
            read_head(
                &b"garbage

"[..]
            ),
            Err(HttpError::Protocol(_))
        ));
        let short = b"HTTP/1.1 200 OK
Content-Length: 10

abc";
        assert!(read_head(&short[..]).unwrap().1.read_all(64).is_err());
        let bad_chunk = b"HTTP/1.1 200 OK
Transfer-Encoding: chunked

zz
";
        assert!(read_head(&bad_chunk[..]).unwrap().1.read_all(64).is_err());
    }

    /// Serves one canned response (written in pieces) and returns the request it received.
    fn serve(resp: Vec<u8>, pause: Duration) -> (u16, std::thread::JoinHandle<String>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let h = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut req = Vec::new();
            let mut buf = [0u8; 4096];
            s.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
            // Read the whole request (headers, then exactly Content-Length body bytes): closing a socket
            // with unread data makes Windows reset the connection, and the client would see a reset.
            while let Ok(n) = s.read(&mut buf) {
                if n == 0 {
                    break;
                }
                req.extend_from_slice(&buf[..n]);
                if let Some(end) = req.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&req[..end]).to_ascii_lowercase();
                    let body_len = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    if req.len() >= end + 4 + body_len {
                        break;
                    }
                }
            }
            for piece in resp.chunks(7) {
                if s.write_all(piece).is_err() {
                    break;
                }
                std::thread::sleep(pause);
            }
            String::from_utf8_lossy(&req).into_owned()
        });
        (port, h)
    }

    fn client(port: u16) -> Client {
        Client { port, key: "k3y".into() }
    }

    fn call<'a>(path: &'a str, body: Option<&'a [u8]>, cancelled: &'a dyn Fn() -> bool) -> Call<'a> {
        Call {
            method: if body.is_some() { "POST" } else { "GET" },
            path,
            body,
            idle: Duration::from_secs(5),
            cancelled,
        }
    }

    #[test]
    fn content_length_body_and_request_headers() {
        let (port, h) = serve(
            b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nX: y\r\n\r\n{\"ok\":true}".to_vec(),
            Duration::ZERO,
        );
        let r = client(port).send(&call("/tokenize", Some(b"{\"a\":1}"), &never), 1024).unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.json().unwrap()["ok"], true);
        let req = h.join().unwrap();
        assert!(req.starts_with("POST /tokenize HTTP/1.1\r\n"));
        assert!(req.contains("Authorization: Bearer k3y\r\n") && req.contains("Content-Length: 7\r\n"));
    }

    #[test]
    fn chunked_lines_are_decoded() {
        let body = "data: one\n\ndata: two\r\n";
        let mut resp = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
        for part in [&body[..5], &body[5..]] {
            resp.extend(format!("{:x};ext=1\r\n{part}\r\n", part.len()).as_bytes());
        }
        resp.extend(b"0\r\nTrailer: x\r\n\r\n");
        let (port, _h) = serve(resp, Duration::from_millis(1));
        let (status, mut b) = client(port).open(&call("/completion", Some(b"{}"), &never)).unwrap();
        assert_eq!(status, 200);
        let mut lines = vec![];
        while let Some(l) = b.next_line(100).unwrap() {
            lines.push(l);
        }
        assert_eq!(lines, ["data: one", "", "data: two"]);
    }

    #[test]
    fn until_close_and_limits() {
        let (port, _h) = serve(b"HTTP/1.1 500 Oops\r\n\r\nabcdefghij".to_vec(), Duration::ZERO);
        let r = client(port).send(&call("/x", None, &never), 64).unwrap();
        assert_eq!((r.status, r.body.as_slice()), (500, b"abcdefghij".as_slice()));
        assert!(r.json().is_none());
        let (port, _h) = serve(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nabcdefghij".to_vec(), Duration::ZERO);
        assert!(matches!(
            client(port).send(&call("/x", None, &never), 5),
            Err(HttpError::TooLarge(_))
        ));
        let (port, _h) = serve(b"HTTP/1.1 200 OK\r\n\r\naaaaaaaaaaaaaaaaaaaa\n".to_vec(), Duration::ZERO);
        let (_, mut b) = client(port).open(&call("/x", None, &never)).unwrap();
        assert!(matches!(b.next_line(4), Err(HttpError::TooLarge("line"))));
    }

    #[test]
    fn malformed_answers_are_errors_not_panics() {
        for resp in [
            b"garbage\r\n\r\n".to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Length: nope\r\n\r\n".to_vec(),
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n".to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 50\r\n\r\nshort".to_vec(),
        ] {
            let (port, _h) = serve(resp, Duration::ZERO);
            let r = client(port).send(&call("/x", None, &never), 1024);
            assert!(r.is_err(), "{r:?}");
            assert!(!r.unwrap_err().to_string().is_empty());
        }
        let big = format!("HTTP/1.1 200 OK\r\n{}\r\n", "X: y\r\n".repeat(12_000));
        let (port, _h) = serve(big.into_bytes(), Duration::ZERO);
        assert!(matches!(
            client(port).send(&call("/x", None, &never), 10),
            Err(HttpError::TooLarge("header"))
        ));
    }

    #[test]
    fn cancellation_and_idle_timeout_interrupt_a_blocked_read() {
        let (port, _h) = serve(b"HTTP/1.1 200 OK\r\n\r\nslow".to_vec(), Duration::from_millis(400));
        let start = Instant::now();
        let cancel = move || start.elapsed() > Duration::from_millis(100);
        let r = client(port).send(&call("/x", None, &cancel), 1024);
        assert!(matches!(r, Err(HttpError::Cancelled)), "{r:?}");
        assert!(start.elapsed() < Duration::from_millis(390));

        let (port, _h) = serve(b"HTTP/1.1 200 OK\r\n\r\nslow".to_vec(), Duration::from_millis(400));
        let mut c = call("/x", None, &never);
        c.idle = Duration::from_millis(100);
        assert!(matches!(client(port).send(&c, 1024), Err(HttpError::TimedOut)));
    }

    #[test]
    fn nobody_listening_is_a_connect_error() {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let e = client(port).send(&call("/health", None, &never), 10).unwrap_err();
        assert!(matches!(e, HttpError::Connect(_)));
        assert!(e.to_string().contains("cannot connect"));
        for e in [HttpError::Io(io::Error::other("x")), HttpError::TimedOut, HttpError::Cancelled] {
            assert!(!e.to_string().is_empty());
        }
    }
}
