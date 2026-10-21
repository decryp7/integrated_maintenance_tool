//! A fake child for the launcher and supervisor tests (never bundled). It runs [`imt_ipc::child::child_main`]
//! and answers text requests:
//!
//! - `echo <text>` → `<text>`; `env <NAME>` → the variable or `<unset>`; `exit <code>` → exits at once (a crash);
//!   `hang` → never answers; `panic` → panics in the handler; `big <n>` → n bytes; `stream <n>` → n items;
//!   `alloc <n>` → allocates and touches n bytes, `ok …` or `denied: …`;
//!   `write <path>` → tries to create the file, answers `ok` or the error; `spawn` → tries to start a program;
//!   `ask <text>` → calls the host back with `<text>` and answers with the host's reply; `gaps` → the notes of its
//!   lockdown (JSON), so tests know what this system enforces; `read <path>` → `ok <bytes>` or the error;
//!   `connect <ip:port>` → opens a TCP connection, `ok` or the error.
//!
//! Flags: `--exit-now` (dies before connecting), `--no-connect` (reads the bootstrap, never connects),
//! `--bad-token` (connects with a wrong token), `--relay` (starts a copy of itself that connects instead: a
//! process the host did not start), `--garbage` (sends bytes that are not a frame after the handshake),
//! `--oversize` (announces a 1 GiB frame after the handshake), `--say=<text>` (a line on stderr first),
//! `--protocol=<major.minor>` (speaks that protocol version in the handshake), `--skip-lockdown` (serves without
//! locking itself down: a binary that ignores the sandbox, to test what the launcher enforces).

use imt_ipc::child::{connect_speaking, lifeline, read_bootstrap};
use imt_ipc::frame::ProtocolVersion;
use imt_ipc::peer::{code, CallCtx, Handler};
use imt_ipc::RemoteError;
use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

fn handle(ctx: &CallCtx, body: &[u8]) -> Result<Vec<u8>, RemoteError> {
    let text = String::from_utf8_lossy(body).into_owned();
    let (cmd, arg) = text.split_once(' ').unwrap_or((text.as_str(), ""));
    let fail = |m: String| RemoteError::new(code::INTERNAL, m);
    match cmd {
        "echo" => Ok(arg.as_bytes().to_vec()),
        "gaps" => Ok(serde_json::to_vec(imt_ipc::sandbox::applied_notes()).map_err(|e| fail(e.to_string()))?),
        "env" => Ok(std::env::var(arg).unwrap_or_else(|_| "<unset>".into()).into_bytes()),
        "exit" => std::process::exit(arg.parse().unwrap_or(3)),
        "hang" => loop {
            std::thread::sleep(Duration::from_secs(3600));
        },
        "panic" => panic!("handler panicked"),
        "big" => Ok(vec![b'x'; arg.parse().unwrap_or(0)]),
        "alloc" => {
            // Allocates and touches n bytes without aborting when it cannot (`denied`).
            let n: usize = arg.parse().unwrap_or(0);
            let mut v: Vec<u8> = Vec::new();
            Ok(match v.try_reserve_exact(n) {
                Ok(()) => {
                    v.resize(n, 1);
                    format!("ok {}", v.iter().step_by(4096).map(|b| *b as usize).sum::<usize>()).into_bytes()
                }
                Err(e) => format!("denied: {e}").into_bytes(),
            })
        }
        "stream" => {
            for i in 0..arg.parse::<u32>().unwrap_or(0) {
                ctx.send_item(i.to_le_bytes().to_vec())?;
            }
            Ok(b"done".to_vec())
        }
        "read" => Ok(match std::fs::read(arg) {
            Ok(b) => format!("ok {}", b.len()).into_bytes(),
            Err(e) => format!("denied: {e}").into_bytes(),
        }),
        "connect" => Ok(
            match arg
                .parse()
                .map(|a| std::net::TcpStream::connect_timeout(&a, Duration::from_secs(3)))
            {
                Ok(Ok(_)) => b"ok".to_vec(),
                Ok(Err(e)) => format!("denied: {e}").into_bytes(),
                Err(e) => format!("bad address: {e}").into_bytes(),
            },
        ),
        "write" => Ok(match std::fs::write(arg, b"x") {
            Ok(()) => {
                let _ = std::fs::remove_file(arg);
                b"ok".to_vec()
            }
            Err(e) => format!("denied: {e}").into_bytes(),
        }),
        "spawn" => {
            #[allow(clippy::disallowed_methods)]
            let r = std::process::Command::new(std::env::current_exe().map_err(|e| fail(e.to_string()))?)
                .arg("--exit-now")
                .status();
            Ok(match r {
                Ok(_) => b"ok".to_vec(),
                Err(e) => format!("denied: {e}").into_bytes(),
            })
        }
        "ask" => ctx
            .peer()
            .call(arg.as_bytes().to_vec(), Some(Duration::from_secs(10)))
            .map_err(|e| fail(e.to_string())),
        _ => Err(RemoteError::new(code::UNIMPLEMENTED, format!("unknown request {cmd}"))),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    for say in args.iter().filter_map(|a| a.strip_prefix("--say=")) {
        eprintln!("{say}");
    }
    if args.iter().any(|a| a == "--exit-now") {
        std::process::exit(3);
    }
    if args.iter().any(|a| a == "--relay") {
        // Hand the bootstrap to a copy of itself, which connects with the right token but another pid.
        let mut line = String::new();
        let _ = std::io::BufRead::read_line(&mut std::io::BufReader::new(std::io::stdin().take(64 * 1024)), &mut line);
        #[allow(clippy::disallowed_methods)]
        let mut c = std::process::Command::new(std::env::current_exe().expect("exe"))
            .stdin(std::process::Stdio::piped())
            .spawn()
            .expect("spawn");
        if let Some(mut i) = c.stdin.take() {
            use std::io::Write;
            let _ = i.write_all(line.as_bytes());
            let _ = i.flush();
            std::thread::sleep(Duration::from_secs(30));
        }
        let _ = c.kill();
        let _ = c.wait();
        return;
    }
    let mut bootstrap = match read_bootstrap(std::io::stdin().lock()) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("fake child: {e}");
            std::process::exit(1);
        }
    };
    if args.iter().any(|a| a == "--no-connect") {
        lifeline();
        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    }
    if args.iter().any(|a| a == "--bad-token") {
        bootstrap.token = "ab".repeat(32);
    }
    if args.iter().any(|a| a == "--garbage" || a == "--oversize") {
        let token = imt_ipc::link::Token::from_hex(&bootstrap.token).expect("token");
        let address = bootstrap.address.clone();
        let bytes: Vec<u8> = if args.iter().any(|a| a == "--garbage") {
            vec![5, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0xff]
        } else {
            (1u32 << 30).to_le_bytes().to_vec()
        };
        lifeline();
        let _ = imt_ipc::runtime::run(async move {
            use tokio::io::AsyncWriteExt;
            if let Ok(mut io) = imt_ipc::link::handshake(&address, &token, "IMT").await {
                let _ = io.write_all(&bytes).await;
                let _ = io.flush().await;
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
        });
        return;
    }
    let handler: Arc<dyn Handler> = Arc::new(handle);
    if args.iter().any(|a| a == "--skip-lockdown") {
        let token = imt_ipc::link::Token::from_hex(&bootstrap.token).expect("token");
        let address = bootstrap.address.clone();
        let rt = imt_ipc::runtime::shared().expect("runtime");
        let io = rt
            .block_on(async move { imt_ipc::link::handshake(&address, &token, "IMT").await })
            .expect("handshake");
        let peer = imt_ipc::Peer::start(io, handler, imt_ipc::PeerOptions::new("IMT"), rt.handle());
        peer.wait_closed(None);
        return;
    }
    let protocol = args
        .iter()
        .find_map(|a| a.strip_prefix("--protocol="))
        .and_then(|p| p.parse().ok())
        .unwrap_or(ProtocolVersion::UNVERSIONED);
    match connect_speaking(&bootstrap, false, "IMT", protocol, |_| handler) {
        Ok(peer) => {
            // As child_main does: after the lockdown, so the thread is locked down too.
            lifeline();
            peer.wait_closed(None);
        }
        Err(e) => {
            eprintln!("fake child: {e}");
            std::process::exit(1);
        }
    }
}
