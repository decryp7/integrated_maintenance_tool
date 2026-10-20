//! Downloading PDBs from a symbol server into the cache (symstore layout), only when the user enabled a server.
//!
//! One request per PDB per [`HttpFetcher`] (a capture): `GET <url>/<pdb>/<key>/<pdb>`, HTTPS with the platform's
//! certificate verifier (never switched off), bounded in time and size, written to a temporary file and renamed
//! into place only when it is a PDB (MSF 7.0 magic). The PDB name comes from an untrusted image and is checked
//! ([`crate::config::safe_pdb_name`]) before it becomes part of a path or an address.

use crate::config::{cache_file, safe_pdb_name, ServerConfig};
use crate::pe::PdbId;
use crate::SymbolError;
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The first bytes of an MSF 7.0 (PDB) file.
pub const PDB_MAGIC: &[u8] = b"Microsoft C/C++ MSF 7.00\r\n\x1aDS\0\0\0";

/// Why a PDB was not downloaded.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FetchError {
    #[error("the symbol server has no {0}")]
    NotFound(String),
    #[error("{0}")]
    Refused(String),
    #[error("the PDB is larger than {0} bytes")]
    TooLarge(u64),
    #[error("the symbol server could not be reached: {0}")]
    Network(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Gets a PDB into the cache.
pub trait Fetcher: Send + Sync {
    /// The cached PDB of `id`, downloaded into `cache` first when it is not there.
    fn fetch(&self, id: &PdbId, cache: &Path) -> Result<PathBuf, FetchError>;
}

/// A symbol server over HTTPS.
pub struct HttpFetcher {
    cfg: ServerConfig,
    agent: ureq::Agent,
    /// PDBs already asked for (once per fetcher).
    tried: Mutex<HashSet<String>>,
}

impl HttpFetcher {
    pub fn new(cfg: ServerConfig) -> Result<HttpFetcher, SymbolError> {
        cfg.check()?;
        let https = cfg.url.starts_with("https://");
        let tls = ureq::tls::TlsConfig::builder()
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build();
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .https_only(https)
            .max_redirects(5)
            .timeout_global(Some(cfg.timeout))
            .user_agent("imt-symbols/0.1")
            .tls_config(tls)
            .build()
            .into();
        Ok(HttpFetcher {
            cfg,
            agent,
            tried: Mutex::new(HashSet::new()),
        })
    }

    fn download(&self, name: &str, id: &PdbId, dest: &Path) -> Result<(), FetchError> {
        let url = format!("{}/{name}/{}/{name}", self.cfg.url.trim_end_matches('/'), id.key());
        let resp = self.agent.get(&url).call().map_err(|e| FetchError::Network(e.to_string()))?;
        match resp.status().as_u16() {
            200 => {}
            404 => return Err(FetchError::NotFound(name.to_string())),
            s => return Err(FetchError::Network(format!("HTTP {s}"))),
        }
        let max = self.cfg.max_bytes;
        let body = resp.into_body();
        if body.content_length().is_some_and(|n| n > max) {
            return Err(FetchError::TooLarge(max));
        }
        let dir = dest.parent().ok_or_else(|| FetchError::Refused("no cache folder".into()))?;
        std::fs::create_dir_all(dir)?;
        let part = dir.join(format!("{name}.part-{}", std::process::id()));
        let result = (|| {
            let mut reader = body.into_with_config().limit(max.saturating_add(1)).reader();
            let mut file = std::fs::File::create(&part)?;
            let mut head = Vec::with_capacity(PDB_MAGIC.len());
            let mut buf = vec![0u8; 64 * 1024];
            let mut total = 0u64;
            loop {
                let n = match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(e) => return Err(FetchError::Network(e.to_string())),
                };
                total += n as u64;
                if total > max {
                    return Err(FetchError::TooLarge(max));
                }
                if head.len() < PDB_MAGIC.len() {
                    let want = (PDB_MAGIC.len() - head.len()).min(n);
                    head.extend_from_slice(&buf[..want]);
                }
                file.write_all(&buf[..n])?;
            }
            if !head.starts_with(PDB_MAGIC) {
                return Err(FetchError::Refused(format!("the symbol server's answer for {name} is not a PDB")));
            }
            file.sync_all()?;
            Ok(())
        })();
        match result {
            Ok(()) => std::fs::rename(&part, dest).map_err(|e| {
                let _ = std::fs::remove_file(&part);
                FetchError::Io(e)
            }),
            Err(e) => {
                let _ = std::fs::remove_file(&part);
                Err(e)
            }
        }
    }
}

impl Fetcher for HttpFetcher {
    fn fetch(&self, id: &PdbId, cache: &Path) -> Result<PathBuf, FetchError> {
        let name = safe_pdb_name(id).ok_or_else(|| FetchError::Refused(format!("unusable PDB name \"{}\"", id.path)))?;
        let dest = cache_file(cache, id).ok_or_else(|| FetchError::Refused(format!("unusable PDB name \"{}\"", id.path)))?;
        if dest.is_file() {
            return Ok(dest);
        }
        let first = self
            .tried
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(format!("{name}/{}", id.key()));
        if !first {
            return Err(FetchError::NotFound(name.to_string()));
        }
        self.download(name, id, &dest)?;
        Ok(dest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::time::Duration;

    /// A one-request-per-connection HTTP server answering `responses` in order; returns its base address.
    fn serve(responses: Vec<Vec<u8>>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = format!("http://{}", l.local_addr().unwrap());
        let t = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for r in responses {
                let Ok((mut s, _)) = l.accept() else { break };
                let mut buf = [0u8; 4096];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                seen.push(req.lines().next().unwrap_or_default().to_string());
                let _ = s.write_all(&r);
            }
            seen
        });
        (addr, t)
    }

    fn http(status: &str, body: &[u8]) -> Vec<u8> {
        let mut r = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
        r.extend_from_slice(body);
        r
    }

    fn id(name: &str) -> PdbId {
        PdbId {
            guid: [0xab; 16],
            age: 1,
            path: format!(r"D:\b\{name}"),
        }
    }

    fn fetcher(url: &str, max_bytes: u64) -> HttpFetcher {
        HttpFetcher::new(ServerConfig {
            url: url.into(),
            timeout: Duration::from_secs(10),
            max_bytes,
        })
        .unwrap()
    }

    #[test]
    fn downloads_into_the_symstore_layout_once() {
        let mut pdb = PDB_MAGIC.to_vec();
        pdb.extend_from_slice(&[7; 100]);
        let (url, server) = serve(vec![http("200 OK", &pdb), http("404 Not Found", b"")]);
        let cache = tempfile::tempdir().unwrap();
        let f = fetcher(&url, 1 << 20);
        let got = f.fetch(&id("busy.pdb"), cache.path()).unwrap();
        assert_eq!(std::fs::read(&got).unwrap(), pdb);
        assert!(got.ends_with(Path::new("busy.pdb").join(id("x").key()).join("busy.pdb")));
        // Cached: no request.
        assert_eq!(f.fetch(&id("busy.pdb"), cache.path()).unwrap(), got);
        // Missing on the server: asked once, then remembered.
        assert!(matches!(f.fetch(&id("gone.pdb"), cache.path()), Err(FetchError::NotFound(_))));
        assert!(matches!(f.fetch(&id("gone.pdb"), cache.path()), Err(FetchError::NotFound(_))));
        let seen = server.join().unwrap();
        assert_eq!(seen.len(), 2, "{seen:?}");
        assert!(
            seen[0].starts_with(&format!("GET /busy.pdb/{}/busy.pdb", id("x").key())),
            "{seen:?}"
        );
    }

    #[test]
    fn oversized_foreign_and_failing_answers_leave_nothing_behind() {
        let mut big = PDB_MAGIC.to_vec();
        big.extend_from_slice(&[0; 2000]);
        let (url, server) = serve(vec![
            http("200 OK", &big),
            http("200 OK", b"<html>sign in</html>"),
            http("500 Internal Server Error", b""),
        ]);
        let cache = tempfile::tempdir().unwrap();
        let f = fetcher(&url, 1000);
        assert!(matches!(f.fetch(&id("a.pdb"), cache.path()), Err(FetchError::TooLarge(1000))));
        assert!(matches!(f.fetch(&id("b.pdb"), cache.path()), Err(FetchError::Refused(_))));
        assert!(matches!(f.fetch(&id("c.pdb"), cache.path()), Err(FetchError::Network(m)) if m.contains("500")));
        server.join().unwrap();
        let left: Vec<_> = walk(cache.path());
        assert!(left.is_empty(), "{left:?}");
        // Unsafe names are refused before any request.
        assert!(matches!(f.fetch(&id("a:b.pdb"), cache.path()), Err(FetchError::Refused(_))));
        // Nobody listening.
        let dead = fetcher("http://127.0.0.1:9", 10);
        assert!(matches!(dead.fetch(&id("d.pdb"), cache.path()), Err(FetchError::Network(_))));
        assert!(HttpFetcher::new(ServerConfig {
            url: "http://example.com".into(),
            ..ServerConfig::microsoft()
        })
        .is_err());
    }

    fn walk(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
        out
    }
}
