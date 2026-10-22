//! A fake symbol server on the loopback address, for tests of downloads (this crate's and the app's). Not used by
//! IMT itself.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

pub use crate::msf::synthetic as synthetic_pdb;

/// A server on `127.0.0.1` answering each request (one per connection) with `answer(path)`; it records the paths.
/// Its thread ends with the test process.
pub struct FakeServer {
    /// `http://127.0.0.1:<port>`.
    pub url: String,
    seen: Arc<Mutex<Vec<String>>>,
}

impl FakeServer {
    pub fn start(answer: impl Fn(&str) -> Vec<u8> + Send + 'static) -> FakeServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
        let url = format!("http://{}", listener.local_addr().expect("its address"));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { continue };
                let mut buf = [0u8; 8192];
                let n = s.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = request.split_whitespace().nth(1).unwrap_or_default().to_string();
                log.lock().unwrap_or_else(|p| p.into_inner()).push(path.clone());
                let _ = s.write_all(&answer(&path));
            }
        });
        FakeServer { url, seen }
    }

    /// The paths asked for so far, in order.
    pub fn requests(&self) -> Vec<String> {
        self.seen.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

/// An HTTP/1.1 answer with `status` (`200 OK`) and `body`.
pub fn http(status: &str, body: &[u8]) -> Vec<u8> {
    let mut r = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
    r.extend_from_slice(body);
    r
}
