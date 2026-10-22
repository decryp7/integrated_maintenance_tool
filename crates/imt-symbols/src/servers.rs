//! Symbol servers the user configured (off by default), and getting the PDBs of many modules at once: each PDB once,
//! at most a configured number of downloads at a time across every caller ([`Gate`]), each bounded in size and
//! time by its [`ServerConfig`]. Nothing is requested while downloads are off.

use crate::fetch::{FetchError, Fetcher, HttpFetcher, ServerList};
use crate::id::PdbKey;
use crate::{ServerConfig, SymbolError};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

/// Most servers configured.
pub const MAX_SERVERS: usize = 8;
/// Most concurrent downloads.
pub const MAX_CONCURRENT: usize = 8;

/// The user's symbol-server settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServersConfig {
    /// Downloads happen only when this is on (off by default).
    pub enabled: bool,
    /// Base addresses, asked in order.
    pub urls: Vec<String>,
    /// Time one download may take.
    pub timeout: Duration,
    /// Largest PDB downloaded.
    pub max_bytes: u64,
    /// Downloads at the same time.
    pub concurrent: usize,
    /// Largest the cache may grow.
    pub cache_max_bytes: u64,
}

impl Default for ServersConfig {
    fn default() -> ServersConfig {
        let ms = ServerConfig::microsoft();
        ServersConfig {
            enabled: false,
            urls: vec![ms.url],
            timeout: ms.timeout,
            max_bytes: ms.max_bytes,
            concurrent: 2,
            cache_max_bytes: 4 << 30,
        }
    }
}

impl ServersConfig {
    /// The servers to ask: `None` when downloads are off or no server is configured. Every address is checked
    /// (`https://`, see [`ServerConfig::check`]).
    pub fn servers(&self) -> Result<Option<ServerList>, SymbolError> {
        if !self.enabled || self.urls.is_empty() {
            return Ok(None);
        }
        if self.urls.len() > MAX_SERVERS {
            return Err(SymbolError::InvalidPath(format!("at most {MAX_SERVERS} symbol servers")));
        }
        let list = self
            .urls
            .iter()
            .map(|url| {
                HttpFetcher::new(ServerConfig {
                    url: url.trim().to_string(),
                    timeout: self.timeout,
                    max_bytes: self.max_bytes,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(ServerList::new(list)))
    }
}

/// A counting semaphore: at most `max` holders at once.
pub struct Gate {
    max: usize,
    used: Mutex<usize>,
    freed: Condvar,
}

/// A held place in a [`Gate`]; dropping it frees the place.
pub struct Permit<'a>(&'a Gate);

impl Gate {
    pub fn new(max: usize) -> Gate {
        Gate {
            max: max.clamp(1, MAX_CONCURRENT),
            used: Mutex::new(0),
            freed: Condvar::new(),
        }
    }

    pub fn max(&self) -> usize {
        self.max
    }

    /// Waits for a free place.
    pub fn acquire(&self) -> Permit<'_> {
        let mut used = self.used.lock().unwrap_or_else(|p| p.into_inner());
        while *used >= self.max {
            used = self.freed.wait(used).unwrap_or_else(|p| p.into_inner());
        }
        *used += 1;
        Permit(self)
    }
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        let mut used = self.0.used.lock().unwrap_or_else(|p| p.into_inner());
        *used = used.saturating_sub(1);
        self.0.freed.notify_one();
    }
}

/// What getting one PDB came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// It was in the cache already.
    Cached(PathBuf),
    Downloaded(PathBuf),
    /// Downloads are off: nothing was asked.
    Disabled,
    Failed(FetchError),
    /// The caller cancelled before it was asked (a download already running is bounded by its own time and size
    /// limits and finishes).
    Cancelled,
}

/// Gets every PDB of `keys` (duplicates once) into `cache` through `fetcher`, with at most `gate.max()` downloads
/// at a time (the gate may be shared with other callers). `None` as the fetcher makes no request.
pub fn get_all(fetcher: Option<&dyn Fetcher>, keys: &[PdbKey], cache: &Path, gate: &Gate) -> Vec<(PdbKey, Outcome)> {
    get_all_observed(fetcher, keys, cache, gate, &AtomicBool::new(false), &|_, _| {})
}

/// [`get_all`] that stops asking for more once `cancel` is set (the rest are [`Outcome::Cancelled`]) and tells
/// `each` every PDB's outcome as soon as it is known (from the worker threads), for progress and early use.
pub fn get_all_observed(
    fetcher: Option<&dyn Fetcher>,
    keys: &[PdbKey],
    cache: &Path,
    gate: &Gate,
    cancel: &AtomicBool,
    each: &(dyn Fn(&PdbKey, &Outcome) + Sync),
) -> Vec<(PdbKey, Outcome)> {
    let mut unique: Vec<PdbKey> = keys.to_vec();
    unique.sort();
    unique.dedup();
    let cached = |k: &PdbKey| {
        let f = k.store_file(cache);
        f.is_file().then(|| {
            crate::cache::touch(&f);
            f
        })
    };
    let Some(fetcher) = fetcher else {
        return unique
            .into_iter()
            .map(|k| {
                let o = cached(&k).map_or(Outcome::Disabled, Outcome::Cached);
                each(&k, &o);
                (k, o)
            })
            .collect();
    };
    let results: Vec<Mutex<Option<Outcome>>> = unique.iter().map(|_| Mutex::new(None)).collect();
    let next = AtomicUsize::new(0);
    let workers = gate.max().min(unique.len());
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(k) = unique.get(i) else { break };
                let outcome = if cancel.load(Ordering::Relaxed) {
                    Outcome::Cancelled
                } else {
                    match cached(k) {
                        Some(f) => Outcome::Cached(f),
                        None => {
                            let _permit = gate.acquire();
                            // Waiting for a place may have taken a while.
                            if cancel.load(Ordering::Relaxed) {
                                Outcome::Cancelled
                            } else {
                                match fetcher.fetch(k, cache) {
                                    Ok(f) => Outcome::Downloaded(f),
                                    Err(e) => Outcome::Failed(e),
                                }
                            }
                        }
                    }
                };
                each(k, &outcome);
                *results[i].lock().unwrap_or_else(|p| p.into_inner()) = Some(outcome);
            });
        }
    });
    unique
        .into_iter()
        .zip(results)
        .map(|(k, r)| {
            let o = r
                .into_inner()
                .unwrap_or_else(|p| p.into_inner())
                .unwrap_or_else(|| Outcome::Failed(FetchError::Io("the download did not run".into())));
            (k, o)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::PdbName;
    use crate::msf;
    use crate::testing::{http, FakeServer};
    use std::sync::Arc;

    fn key(name: &str, g: u8) -> PdbKey {
        PdbKey::new(PdbName::parse(name).unwrap(), [g; 16], 1)
    }

    fn enabled(url: &str) -> ServersConfig {
        ServersConfig {
            enabled: true,
            urls: vec![url.into()],
            timeout: Duration::from_secs(10),
            ..ServersConfig::default()
        }
    }

    #[test]
    fn disabled_by_default_makes_no_request() {
        let server = FakeServer::start(|_| http("200 OK", &msf::synthetic(&[1; 16], 1)));
        let off = ServersConfig {
            urls: vec![server.url.clone()],
            ..ServersConfig::default()
        };
        assert!(!ServersConfig::default().enabled);
        assert!(ServersConfig::default().urls[0].starts_with("https://"));
        let servers = off.servers().unwrap();
        assert!(servers.is_none());
        let cache = tempfile::tempdir().unwrap();
        let got = get_all(None, &[key("a.pdb", 1)], cache.path(), &Gate::new(2));
        assert_eq!(got, [(key("a.pdb", 1), Outcome::Disabled)]);
        assert!(server.requests().is_empty());
        // A PDB already in the cache is still found.
        let f = key("a.pdb", 1).store_file(cache.path());
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(&f, b"x").unwrap();
        assert_eq!(
            get_all(None, &[key("a.pdb", 1)], cache.path(), &Gate::new(2))[0].1,
            Outcome::Cached(f)
        );
        assert!(ServersConfig {
            urls: Vec::new(),
            ..enabled("x")
        }
        .servers()
        .unwrap()
        .is_none());
    }

    #[test]
    fn each_pdb_is_asked_once_and_a_cache_hit_avoids_the_second_download() {
        let server = FakeServer::start(|path| {
            let g = if path.starts_with("/a.pdb") { 1 } else { 2 };
            if path.starts_with("/missing") {
                http("404 Not Found", b"")
            } else {
                http("200 OK", &msf::synthetic(&[g; 16], 1))
            }
        });
        let cfg = enabled(&server.url);
        let cache = tempfile::tempdir().unwrap();
        let gate = Gate::new(cfg.concurrent);
        let keys = [key("a.pdb", 1), key("b.pdb", 2), key("a.pdb", 1), key("missing.pdb", 3)];
        let servers = cfg.servers().unwrap().unwrap();
        let got = get_all(Some(&servers), &keys, cache.path(), &gate);
        assert_eq!(got.len(), 3, "duplicates are asked once");
        assert!(matches!(got[0].1, Outcome::Downloaded(_)), "{got:?}");
        assert!(matches!(got[1].1, Outcome::Downloaded(_)), "{got:?}");
        assert!(matches!(got[2].1, Outcome::Failed(FetchError::NotFound(_))), "{got:?}");
        assert_eq!(server.requests().len(), 3);
        // Again, with fresh fetchers: the cache answers.
        let again = get_all(Some(&cfg.servers().unwrap().unwrap()), &keys[..2], cache.path(), &gate);
        assert!(again.iter().all(|(_, o)| matches!(o, Outcome::Cached(_))), "{again:?}");
        assert_eq!(server.requests().len(), 3);
    }

    #[test]
    fn the_gate_bounds_concurrent_downloads() {
        struct Slow {
            now: AtomicUsize,
            peak: AtomicUsize,
        }
        impl Fetcher for Slow {
            fn fetch(&self, k: &PdbKey, _: &Path) -> Result<PathBuf, FetchError> {
                let n = self.now.fetch_add(1, Ordering::SeqCst) + 1;
                self.peak.fetch_max(n, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(20));
                self.now.fetch_sub(1, Ordering::SeqCst);
                Err(FetchError::NotFound(k.name().to_string()))
            }
        }
        let slow = Slow {
            now: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        };
        let gate = Arc::new(Gate::new(2));
        let keys: Vec<PdbKey> = (0..8).map(|i| key("m.pdb", i)).collect();
        let cache = tempfile::tempdir().unwrap();
        // Two callers share one gate.
        std::thread::scope(|s| {
            for _ in 0..2 {
                s.spawn(|| get_all(Some(&slow), &keys, cache.path(), &gate));
            }
        });
        assert_eq!(slow.peak.load(Ordering::SeqCst), 2);
        assert_eq!((Gate::new(0).max(), Gate::new(99).max()), (1, MAX_CONCURRENT));
    }

    #[test]
    fn cancelling_stops_the_downloads_not_yet_asked_and_each_outcome_is_reported() {
        struct Counting(AtomicUsize);
        impl Fetcher for Counting {
            fn fetch(&self, k: &PdbKey, _: &Path) -> Result<PathBuf, FetchError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Err(FetchError::NotFound(k.name().to_string()))
            }
        }
        let f = Counting(AtomicUsize::new(0));
        let cache = tempfile::tempdir().unwrap();
        let keys: Vec<PdbKey> = (0..6).map(|i| key("m.pdb", i)).collect();
        let seen = Mutex::new(Vec::new());
        let cancel = AtomicBool::new(false);
        // Cancelled by the first outcome: with one worker nothing more is asked.
        let got = get_all_observed(Some(&f), &keys, cache.path(), &Gate::new(1), &cancel, &|k, o| {
            seen.lock().unwrap().push((k.clone(), o.clone()));
            cancel.store(true, Ordering::SeqCst);
        });
        assert_eq!(got.len(), 6);
        assert!(matches!(got[0].1, Outcome::Failed(FetchError::NotFound(_))), "{got:?}");
        assert!(got[1..].iter().all(|(_, o)| *o == Outcome::Cancelled), "{got:?}");
        assert_eq!(f.0.load(Ordering::SeqCst), 1);
        assert_eq!(seen.lock().unwrap().len(), 6, "every PDB is reported, cancelled ones too");
        // Cancelled before it started: nothing is asked at all.
        let off = AtomicBool::new(true);
        let none = get_all_observed(Some(&f), &keys, cache.path(), &Gate::new(2), &off, &|_, _| {});
        assert!(none.iter().all(|(_, o)| *o == Outcome::Cancelled));
        assert_eq!(f.0.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn bad_addresses_and_too_many_servers_are_refused() {
        assert!(enabled("http://example.com/s").servers().is_err());
        let many = ServersConfig {
            urls: vec!["https://a.example".into(); MAX_SERVERS + 1],
            ..enabled("x")
        };
        assert!(many.servers().is_err());
    }
}
