//! Downloading PDBs from symbol servers into the cache (symstore layout), only when the user enabled a server.
//!
//! One request per PDB per [`HttpFetcher`]: `GET <url>/<pdb>/<key>/<pdb>`, HTTPS with the platform's certificate
//! verifier (never switched off), bounded in time and size, written to a temporary file and renamed into place only
//! when it is a PDB (MSF 7.0) **recording the GUID and age asked for**: a server's answer for another build is
//! refused, so a PDB is never matched by its file name alone. Names come from untrusted files and are parsed into a
//! [`PdbKey`] before they become part of a path or an address.

use crate::id::PdbKey;
use crate::{msf, ServerConfig, SymbolError};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

pub use crate::msf::PDB_MAGIC;

/// PDBs a fetcher remembers it could not get (so a server is not asked again for them).
const MAX_REMEMBERED: usize = 4096;

/// Why a PDB was not downloaded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum FetchError {
    #[error("the symbol server has no {0}")]
    NotFound(String),
    #[error("{0}")]
    Refused(String),
    #[error("the PDB is larger than {} MB", .0 >> 20)]
    TooLarge(u64),
    #[error("the symbol server's {0}")]
    Mismatch(String),
    #[error("the symbol server could not be reached: {0}")]
    Network(String),
    #[error("{0}")]
    Io(String),
    /// The caller cancelled while the PDB was downloading; the partial file was removed.
    #[error("the download was cancelled")]
    Cancelled,
}

impl From<std::io::Error> for FetchError {
    fn from(e: std::io::Error) -> FetchError {
        FetchError::Io(e.to_string())
    }
}

impl FetchError {
    /// Whether asking again later may succeed (the network, the disk); other failures are about the PDB itself.
    pub fn is_transient(&self) -> bool {
        matches!(self, FetchError::Network(_) | FetchError::Io(_))
    }
}

/// What a download reports and honours while it runs: the cancel flag (checked between reads) and the bytes
/// received, called with each chunk's size from the downloading thread.
pub struct DownloadWatch<'a> {
    pub cancel: &'a AtomicBool,
    pub bytes: &'a (dyn Fn(u64) + Sync),
}

/// Gets a PDB into the cache.
pub trait Fetcher: Send + Sync {
    /// The cached PDB of `key`, downloaded into `cache` first when it is not there.
    fn fetch(&self, key: &PdbKey, cache: &Path) -> Result<PathBuf, FetchError>;

    /// [`Self::fetch`] that stops inside a running download once `watch.cancel` is set (the result is
    /// [`FetchError::Cancelled`] and nothing is cached) and reports the bytes received. A fetcher that cannot do
    /// either just fetches.
    fn fetch_watched(&self, key: &PdbKey, cache: &Path, _watch: &DownloadWatch<'_>) -> Result<PathBuf, FetchError> {
        self.fetch(key, cache)
    }
}

/// Distinct names for temporary files of concurrent downloads in one process.
static NEXT_PART: AtomicU64 = AtomicU64::new(0);

/// A symbol server over HTTPS.
pub struct HttpFetcher {
    cfg: ServerConfig,
    agent: ureq::Agent,
    /// PDBs this server could not give (missing, too large, another build), by store path.
    failed: Mutex<HashMap<String, FetchError>>,
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
            .user_agent("debrief-symbols/0.1")
            .tls_config(tls)
            .build()
            .into();
        Ok(HttpFetcher {
            cfg,
            agent,
            failed: Mutex::new(HashMap::new()),
        })
    }

    pub fn url(&self) -> &str {
        &self.cfg.url
    }

    fn download(&self, key: &PdbKey, dest: &Path, watch: &DownloadWatch<'_>) -> Result<(), FetchError> {
        let name = key.name().as_str();
        let url = format!("{}/{}", self.cfg.url.trim_end_matches('/'), key.store_path());
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
        let part = dir.join(format!(
            "{name}.part-{}-{}",
            std::process::id(),
            NEXT_PART.fetch_add(1, Ordering::Relaxed)
        ));
        let result = (|| {
            let mut reader = body.into_with_config().limit(max.saturating_add(1)).reader();
            let mut file = std::fs::File::create(&part)?;
            let mut buf = vec![0u8; 64 * 1024];
            let mut total = 0u64;
            loop {
                if watch.cancel.load(Ordering::Relaxed) {
                    return Err(FetchError::Cancelled);
                }
                let n = match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(e) => return Err(FetchError::Network(e.to_string())),
                };
                total += n as u64;
                if total > max {
                    return Err(FetchError::TooLarge(max));
                }
                file.write_all(&buf[..n])?;
                (watch.bytes)(n as u64);
            }
            file.sync_all()?;
            drop(file);
            let mut written = std::fs::File::open(&part)?;
            match msf::identity(&mut written) {
                None => Err(FetchError::Refused(format!("the symbol server's answer for {name} is not a PDB"))),
                Some((guid, age)) if !key.matches(&guid, age) => Err(FetchError::Mismatch(format!(
                    "{name} is another build (GUID or age differ from {})",
                    key.key()
                ))),
                Some(_) => Ok(()),
            }
        })();
        match result {
            Ok(()) => std::fs::rename(&part, dest).map_err(|e| {
                let _ = std::fs::remove_file(&part);
                FetchError::from(e)
            }),
            Err(e) => {
                let _ = std::fs::remove_file(&part);
                Err(e)
            }
        }
    }
}

impl Fetcher for HttpFetcher {
    fn fetch(&self, key: &PdbKey, cache: &Path) -> Result<PathBuf, FetchError> {
        self.fetch_watched(
            key,
            cache,
            &DownloadWatch {
                cancel: &AtomicBool::new(false),
                bytes: &|_| {},
            },
        )
    }

    fn fetch_watched(&self, key: &PdbKey, cache: &Path, watch: &DownloadWatch<'_>) -> Result<PathBuf, FetchError> {
        let dest = key.store_file(cache);
        if dest.is_file() {
            crate::cache::touch(&dest);
            return Ok(dest);
        }
        let id = key.store_path();
        if let Some(e) = self.failed.lock().unwrap_or_else(|p| p.into_inner()).get(&id) {
            return Err(e.clone());
        }
        let result = self.download(key, &dest, watch);
        if let Err(e) = &result {
            let mut failed = self.failed.lock().unwrap_or_else(|p| p.into_inner());
            if !e.is_transient() && *e != FetchError::Cancelled && failed.len() < MAX_REMEMBERED {
                failed.insert(id, e.clone());
            }
        }
        result.map(|()| dest)
    }
}

/// Several symbol servers, asked in order: the first that has the PDB gives it.
pub struct ServerList {
    servers: Vec<HttpFetcher>,
}

impl ServerList {
    pub fn new(servers: Vec<HttpFetcher>) -> ServerList {
        ServerList { servers }
    }

    pub fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }
}

impl Fetcher for ServerList {
    /// `NotFound` when no server has it; otherwise the first other failure.
    fn fetch(&self, key: &PdbKey, cache: &Path) -> Result<PathBuf, FetchError> {
        self.fetch_watched(
            key,
            cache,
            &DownloadWatch {
                cancel: &AtomicBool::new(false),
                bytes: &|_| {},
            },
        )
    }

    fn fetch_watched(&self, key: &PdbKey, cache: &Path, watch: &DownloadWatch<'_>) -> Result<PathBuf, FetchError> {
        let mut failure = None;
        for s in &self.servers {
            match s.fetch_watched(key, cache, watch) {
                Ok(p) => return Ok(p),
                Err(FetchError::Cancelled) => return Err(FetchError::Cancelled),
                Err(FetchError::NotFound(_)) => {}
                Err(e) => {
                    failure.get_or_insert(e);
                }
            }
        }
        Err(failure.unwrap_or_else(|| FetchError::NotFound(key.name().to_string())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{http, FakeServer};
    use std::time::Duration;

    const GUID: [u8; 16] = [0xab; 16];

    fn key(name: &str) -> PdbKey {
        PdbKey::new(crate::id::PdbName::parse(name).unwrap(), GUID, 1)
    }

    fn fetcher(url: &str, max_bytes: u64) -> HttpFetcher {
        HttpFetcher::new(ServerConfig {
            url: url.into(),
            timeout: Duration::from_secs(10),
            max_bytes,
        })
        .unwrap()
    }

    fn files(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(files(&p));
            } else {
                out.push(p);
            }
        }
        out
    }

    #[test]
    fn a_matching_pdb_is_downloaded_once_and_then_served_from_the_cache() {
        let pdb = msf::synthetic(&GUID, 1);
        let body = pdb.clone();
        let server = FakeServer::start(move |path| {
            if path.starts_with("/busy.pdb/") {
                http("200 OK", &body)
            } else {
                http("404 Not Found", b"")
            }
        });
        let cache = tempfile::tempdir().unwrap();
        let f = fetcher(&server.url, 1 << 20);
        let got = f.fetch(&key("busy.pdb"), cache.path()).unwrap();
        assert_eq!(std::fs::read(&got).unwrap(), pdb);
        assert_eq!(got, key("busy.pdb").store_file(cache.path()));
        // A cache hit makes no request, also from a fresh fetcher.
        assert_eq!(fetcher(&server.url, 1 << 20).fetch(&key("busy.pdb"), cache.path()).unwrap(), got);
        // Missing on the server: asked once, then remembered.
        assert!(matches!(f.fetch(&key("gone.pdb"), cache.path()), Err(FetchError::NotFound(_))));
        assert!(matches!(f.fetch(&key("gone.pdb"), cache.path()), Err(FetchError::NotFound(_))));
        let seen = server.requests();
        assert_eq!(
            seen,
            [
                format!("/{}", key("busy.pdb").store_path()),
                format!("/{}", key("gone.pdb").store_path())
            ]
        );
    }

    /// Cancel is honoured inside a running download: it stops at the next chunk, leaves nothing in the cache, is not
    /// remembered as a failure of the PDB, and the bytes received were reported.
    #[test]
    fn a_download_in_progress_stops_on_cancel_and_reports_its_bytes() {
        let body = vec![7u8; 900_000];
        let server = FakeServer::start(move |_| http("200 OK", &body));
        let cache = tempfile::tempdir().unwrap();
        let f = fetcher(&server.url, 1 << 21);
        let cancel = AtomicBool::new(false);
        let received = AtomicU64::new(0);
        let count = |n: u64| {
            received.fetch_add(n, Ordering::Relaxed);
            cancel.store(true, Ordering::Relaxed);
        };
        let r = f.fetch_watched(
            &key("big.pdb"),
            cache.path(),
            &DownloadWatch {
                cancel: &cancel,
                bytes: &count,
            },
        );
        assert_eq!(r, Err(FetchError::Cancelled));
        let got = received.load(Ordering::Relaxed);
        assert!(got > 0 && got < 900_000, "stopped part way: {got}");
        assert!(files(cache.path()).is_empty(), "the partial file is removed");
        // Not remembered as a failure: without the cancel it downloads (and fails only on being no PDB).
        let again = f.fetch_watched(
            &key("big.pdb"),
            cache.path(),
            &DownloadWatch {
                cancel: &AtomicBool::new(false),
                bytes: &|_| {},
            },
        );
        assert!(matches!(again, Err(FetchError::Refused(_))), "{again:?}");
        assert!(!FetchError::Cancelled.is_transient());
    }

    #[test]
    fn another_builds_pdb_is_refused_and_not_cached() {
        let other = msf::synthetic(&[0xcd; 16], 1);
        let older = msf::synthetic(&GUID, 2);
        let server = FakeServer::start(move |path| {
            if path.contains("guid") {
                http("200 OK", &other)
            } else {
                http("200 OK", &older)
            }
        });
        let cache = tempfile::tempdir().unwrap();
        let f = fetcher(&server.url, 1 << 20);
        for name in ["guid.pdb", "age.pdb"] {
            let e = f.fetch(&key(name), cache.path()).unwrap_err();
            assert!(matches!(&e, FetchError::Mismatch(m) if m.contains("another build")), "{name}: {e}");
            assert!(!e.is_transient());
        }
        assert!(files(cache.path()).is_empty(), "{:?}", files(cache.path()));
        // Remembered: no second request.
        assert!(matches!(f.fetch(&key("age.pdb"), cache.path()), Err(FetchError::Mismatch(_))));
        assert_eq!(server.requests().len(), 2);
    }

    #[test]
    fn oversized_foreign_and_failing_answers_leave_nothing_behind() {
        let mut big = msf::synthetic(&GUID, 1);
        big.resize(5000, 0);
        let streamed = big.clone();
        let server = FakeServer::start(move |path| match path.split('/').nth(1).unwrap_or_default() {
            "a.pdb" => http("200 OK", &big),
            // No length: the body is cut off while it streams.
            "s.pdb" => {
                let mut r = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
                r.extend_from_slice(&streamed);
                r
            }
            "b.pdb" => http("200 OK", b"<html>sign in</html>"),
            _ => http("500 Internal Server Error", b""),
        });
        let cache = tempfile::tempdir().unwrap();
        let f = fetcher(&server.url, 1000);
        assert_eq!(f.fetch(&key("a.pdb"), cache.path()), Err(FetchError::TooLarge(1000)));
        assert_eq!(f.fetch(&key("s.pdb"), cache.path()), Err(FetchError::TooLarge(1000)));
        assert!(matches!(f.fetch(&key("b.pdb"), cache.path()), Err(FetchError::Refused(_))));
        let e = f.fetch(&key("c.pdb"), cache.path()).unwrap_err();
        assert!(matches!(&e, FetchError::Network(m) if m.contains("500")) && e.is_transient());
        // A transient failure is asked again.
        assert!(f.fetch(&key("c.pdb"), cache.path()).is_err());
        assert_eq!(server.requests().len(), 5);
        assert!(files(cache.path()).is_empty(), "{:?}", files(cache.path()));
        // Nobody listening.
        let dead = fetcher("http://127.0.0.1:9", 10);
        assert!(matches!(dead.fetch(&key("d.pdb"), cache.path()), Err(FetchError::Network(_))));
        assert!(HttpFetcher::new(ServerConfig {
            url: "http://example.com".into(),
            ..ServerConfig::microsoft()
        })
        .is_err());
        assert_eq!(FetchError::TooLarge(2 << 20).to_string(), "the PDB is larger than 2 MB");
        assert_eq!(FetchError::from(std::io::Error::other("x")), FetchError::Io("x".into()));
    }

    #[test]
    fn servers_are_asked_in_order_until_one_has_the_pdb() {
        let pdb = msf::synthetic(&GUID, 1);
        let empty = FakeServer::start(|_| http("404 Not Found", b""));
        let full = FakeServer::start(move |_| http("200 OK", &pdb));
        let cache = tempfile::tempdir().unwrap();
        let list = ServerList::new(vec![fetcher(&empty.url, 1 << 20), fetcher(&full.url, 1 << 20)]);
        assert!(!list.is_empty() && list.servers[0].url() == empty.url);
        assert!(list.fetch(&key("x.pdb"), cache.path()).is_ok());
        assert_eq!((empty.requests().len(), full.requests().len()), (1, 1));
        let none = ServerList::new(vec![fetcher(&empty.url, 1 << 20)]);
        assert!(matches!(none.fetch(&key("y.pdb"), cache.path()), Err(FetchError::NotFound(_))));
        let dead = ServerList::new(vec![fetcher("http://127.0.0.1:9", 10), fetcher(&empty.url, 1 << 20)]);
        assert!(matches!(dead.fetch(&key("z.pdb"), cache.path()), Err(FetchError::Network(_))));
        assert!(matches!(
            ServerList::new(Vec::new()).fetch(&key("z.pdb"), cache.path()),
            Err(FetchError::NotFound(_))
        ));
    }
}
