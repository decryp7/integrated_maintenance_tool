//! The one child loop (ADR 0017 §2, §5): a child process' `main` calls [`child_main`], which
//!
//! 1. reads its [`Bootstrap`] (one JSON line on stdin, or the file `--imt-bootstrap <file>` names for an elevated
//!    child, which has no stdin pipe);
//! 2. connects to the host and authenticates;
//! 3. locks itself down ([`crate::sandbox::lockdown`]) before it handles anything;
//! 4. answers the host's calls with its handler until the connection closes, then returns (the process exits).
//!
//! A child also exits when its stdin closes (the host died); nothing is left behind.

use crate::elevation::{take_bootstrap, BOOTSTRAP_FLAG};
use crate::launch::Bootstrap;
use crate::link::Token;
use crate::peer::{Handler, Peer, PeerOptions};
use std::io::{self, BufRead, Read};
use std::sync::Arc;

/// Largest bootstrap line read from stdin.
const MAX_BOOTSTRAP: u64 = 64 * 1024;

/// Reads the bootstrap line from `input`.
pub fn read_bootstrap(input: impl Read) -> io::Result<Bootstrap> {
    let mut line = String::new();
    io::BufReader::new(input.take(MAX_BOOTSTRAP)).read_line(&mut line)?;
    serde_json::from_str(line.trim()).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("bad bootstrap: {e}")))
}

/// Where the bootstrap comes from: `--imt-bootstrap <file>` among `args` (an elevated child), else stdin.
pub fn bootstrap_file(args: &[String]) -> Option<std::path::PathBuf> {
    args.iter()
        .position(|a| a == BOOTSTRAP_FLAG)
        .and_then(|i| args.get(i + 1))
        .map(std::path::PathBuf::from)
}

/// A child process' `main`; see the module docs. `label` names the host in messages ("IMT"); `make` builds the
/// handler once the bootstrap is known (it may look at [`Bootstrap::role`]).
pub fn child_main(label: &str, make: impl FnOnce(&Bootstrap) -> Arc<dyn Handler>) -> io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let elevated = bootstrap_file(&args);
    let bootstrap = match &elevated {
        Some(file) => take_bootstrap(file)?,
        None => {
            let b = read_bootstrap(io::stdin().lock())?;
            // The lifeline: when the host closes stdin (or dies), exit at once.
            std::thread::spawn(|| {
                let mut sink = [0u8; 256];
                while matches!(io::stdin().read(&mut sink), Ok(n) if n > 0) {}
                std::process::exit(0);
            });
            b
        }
    };
    let peer = connect_and_lock_down(&bootstrap, elevated.is_some(), label, make)?;
    peer.wait_closed(None);
    Ok(())
}

/// Steps 2–4 of [`child_main`]: connect, lock down, serve. Returns the running peer.
pub fn connect_and_lock_down(
    bootstrap: &Bootstrap,
    elevated: bool,
    label: &str,
    make: impl FnOnce(&Bootstrap) -> Arc<dyn Handler>,
) -> io::Result<Peer> {
    let token =
        Token::from_hex(&bootstrap.token).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad token in the bootstrap"))?;
    let address = bootstrap.address.clone();
    let label_owned = label.to_string();
    let stream = crate::runtime::run(async move { crate::link::handshake(&address, &token, &label_owned).await })
        .map_err(io::Error::other)?
        .map_err(|e| io::Error::other(e.to_string()))?;
    // Nothing has been read from the host yet but the handshake: lock down before the first request.
    let notes = crate::sandbox::lockdown(&bootstrap.sandbox, elevated).map_err(|e| io::Error::other(format!("cannot lock down: {e}")))?;
    // On stderr, so the child's log has them (a gap as a Warning): never less sandboxed without saying so.
    for n in notes {
        eprintln!("{}", n.log_line());
    }
    let handler = make(bootstrap);
    let mut opts = PeerOptions::new(label);
    opts.max_frame = bootstrap.max_frame;
    let rt = crate::runtime::shared().map_err(io::Error::other)?;
    Ok(Peer::start(stream, handler, opts, rt.handle()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bootstrap_comes_from_stdin_or_a_named_file() {
        let b = Bootstrap {
            address: "a".into(),
            token: "t".into(),
            sandbox: Default::default(),
            max_frame: 1024,
            role: String::new(),
        };
        let line = format!("{}\nrest", serde_json::to_string(&b).unwrap());
        assert_eq!(read_bootstrap(line.as_bytes()).unwrap(), b);
        assert_eq!(read_bootstrap(&b"nope\n"[..]).unwrap_err().kind(), io::ErrorKind::InvalidData);
        let long = format!("{{\"address\":\"{}\"}}\n", "a".repeat(70_000));
        assert!(read_bootstrap(long.as_bytes()).is_err());
        let args: Vec<String> = ["x", "--plugin", "p", BOOTSTRAP_FLAG, "/tmp/b.json"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(bootstrap_file(&args), Some("/tmp/b.json".into()));
        assert_eq!(bootstrap_file(&args[..4]), None);
        assert_eq!(bootstrap_file(&[]), None);
    }
}
