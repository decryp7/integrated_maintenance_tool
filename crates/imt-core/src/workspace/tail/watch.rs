//! Watching opened roots for live tail, independent of any watcher library:
//! the shell supplies a [`WatcherFactory`] (an adapter over `notify`), this
//! module picks events or polling per root, falls back to polling when a
//! watcher fails, and coalesces bursts of change notifications into batches
//! (at most one per `min_interval`, never one per event).

use super::identity::is_network_path;
use super::types::WatchMode;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Paths remembered per batch; beyond this the batch asks for a rescan of every root.
const MAX_PENDING_PATHS: usize = 100_000;

/// Which watcher to use.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WatchPreference {
    /// Events, except polling for network paths and after a watcher error.
    #[default]
    Auto,
    Events,
    Polling,
}

/// Watcher settings of a live session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WatchSettings {
    pub preference: WatchPreference,
    pub poll_interval_ms: u64,
    /// A batch is delivered once no change arrived for this long...
    pub quiet_ms: u64,
    /// ...or this long after its first change, whichever is first...
    pub max_delay_ms: u64,
    /// ...but never sooner than this after the previous batch (≈ 4 per second).
    pub min_interval_ms: u64,
    /// An empty batch is delivered this often (flushes partial lines, wall-clock silences).
    pub tick_ms: u64,
}

impl Default for WatchSettings {
    fn default() -> Self {
        WatchSettings {
            preference: WatchPreference::Auto,
            poll_interval_ms: 2_000,
            quiet_ms: 100,
            max_delay_ms: 250,
            min_interval_ms: 250,
            tick_ms: 1_000,
        }
    }
}

/// What a watcher reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchSignal {
    Paths(Vec<PathBuf>),
    /// The watcher failed (e.g. the event queue overflowed or the share disconnected).
    Error(String),
}

pub type WatchSink = Arc<dyn Fn(WatchSignal) + Send + Sync>;

/// One running watcher (events or polling).
pub trait FileWatcher: Send {
    fn watch(&mut self, path: &Path, recursive: bool) -> Result<()>;
}

/// Creates watchers; implemented by the shell over its watcher library, and by tests.
pub trait WatcherFactory: Send + Sync {
    fn create(&self, mode: WatchMode, poll_interval: Duration, sink: WatchSink) -> Result<Box<dyn FileWatcher>>;
}

/// The watch mode for a root under a preference.
pub fn initial_mode(pref: WatchPreference, root: &Path) -> WatchMode {
    match pref {
        WatchPreference::Polling => WatchMode::Polling,
        WatchPreference::Events => WatchMode::Events,
        WatchPreference::Auto if is_network_path(root) => WatchMode::Polling,
        WatchPreference::Auto => WatchMode::Events,
    }
}

/// Collects changed paths and decides when a batch is due.
#[derive(Debug)]
pub struct Coalescer {
    settings: WatchSettings,
    pending: BTreeSet<PathBuf>,
    overflow: bool,
    first: Option<Instant>,
    last: Option<Instant>,
    flushed: Option<Instant>,
}

impl Coalescer {
    pub fn new(settings: WatchSettings) -> Coalescer {
        Coalescer {
            settings,
            pending: BTreeSet::new(),
            overflow: false,
            first: None,
            last: None,
            flushed: None,
        }
    }

    pub fn push(&mut self, paths: impl IntoIterator<Item = PathBuf>, now: Instant) {
        for p in paths {
            if self.pending.len() >= MAX_PENDING_PATHS {
                self.overflow = true;
                break;
            }
            self.pending.insert(p);
        }
        self.first.get_or_insert(now);
        self.last = Some(now);
    }

    /// When the pending batch is due, if any.
    pub fn deadline(&self) -> Option<Instant> {
        let (first, last) = (self.first?, self.last?);
        let ms = Duration::from_millis;
        let due = (last + ms(self.settings.quiet_ms)).min(first + ms(self.settings.max_delay_ms));
        Some(match self.flushed {
            Some(f) => due.max(f + ms(self.settings.min_interval_ms)),
            None => due,
        })
    }

    pub fn due(&self, now: Instant) -> bool {
        self.deadline().is_some_and(|d| now >= d)
    }

    /// The batch: distinct paths, or `None` for "rescan everything" after an overflow.
    pub fn take(&mut self, now: Instant) -> Option<Vec<PathBuf>> {
        self.first = None;
        self.last = None;
        self.flushed = Some(now);
        let paths = std::mem::take(&mut self.pending).into_iter().collect();
        (!std::mem::take(&mut self.overflow)).then_some(paths)
    }
}

/// A watched root and how it is watched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchedRoot {
    pub path: PathBuf,
    pub recursive: bool,
    pub mode: WatchMode,
    /// Why the root fell back to polling (or could not be watched).
    pub error: Option<String>,
}

struct Watchers {
    roots: Vec<WatchedRoot>,
    running: Vec<Box<dyn FileWatcher>>,
}

enum Msg {
    Signal(WatchSignal),
    Stop,
}

/// Receives batches of changed paths (on a tick: the polled roots, else
/// nothing; every root after an overflow or a watcher error).
pub type BatchFn = Box<dyn FnMut(Vec<PathBuf>) + Send>;
/// Told when a root falls back to polling, with the reason.
pub type FallbackFn = Box<dyn Fn(&Path, &str) + Send + Sync>;

/// A live session: watchers over the roots and a worker thread delivering batches.
pub struct LiveTail {
    watchers: Arc<Mutex<Watchers>>,
    tx: Sender<Msg>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl LiveTail {
    /// Watches `roots` (folders recursively; a file through its folder, so a
    /// rotated replacement is seen too) and calls `on_batch` from a worker thread.
    pub fn start(
        roots: &[PathBuf],
        settings: WatchSettings,
        factory: Arc<dyn WatcherFactory>,
        on_batch: BatchFn,
        on_fallback: FallbackFn,
    ) -> LiveTail {
        let (tx, rx) = channel::<Msg>();
        let sink_tx = Mutex::new(tx.clone());
        let sink: WatchSink = Arc::new(move |s| {
            if let Ok(tx) = sink_tx.lock() {
                let _ = tx.send(Msg::Signal(s));
            }
        });
        let mut targets: Vec<(PathBuf, bool)> = Vec::new();
        for r in roots {
            let target = if r.is_dir() {
                (r.clone(), true)
            } else {
                (r.parent().map(Path::to_path_buf).unwrap_or_else(|| r.clone()), false)
            };
            if !targets.contains(&target) {
                targets.push(target);
            }
        }
        let watched = targets
            .into_iter()
            .map(|(path, recursive)| WatchedRoot {
                mode: initial_mode(settings.preference, &path),
                path,
                recursive,
                error: None,
            })
            .collect();
        let watchers = Arc::new(Mutex::new(Watchers {
            roots: watched,
            running: Vec::new(),
        }));
        let poll = Duration::from_millis(settings.poll_interval_ms.max(10));
        {
            let mut w = lock(&watchers);
            install(&mut w, factory.as_ref(), poll, &sink, &on_fallback);
        }
        let all_roots: Vec<PathBuf> = roots.to_vec();
        let worker_watchers = watchers.clone();
        let worker = std::thread::Builder::new()
            .name("imt-live-tail".into())
            .spawn(move || {
                let mut on_batch = on_batch;
                let mut co = Coalescer::new(settings);
                let tick = Duration::from_millis(settings.tick_ms.max(10));
                let mut next_tick = Instant::now() + tick;
                loop {
                    let now = Instant::now();
                    let wake = co.deadline().map_or(next_tick, |d| d.min(next_tick));
                    match rx.recv_timeout(wake.saturating_duration_since(now)) {
                        Ok(Msg::Signal(WatchSignal::Paths(p))) => co.push(p, Instant::now()),
                        Ok(Msg::Signal(WatchSignal::Error(e))) => {
                            let mut w = lock(&worker_watchers);
                            fall_back(&mut w, &e, factory.as_ref(), poll, &sink, &on_fallback);
                            // Something may have been missed: look at everything.
                            co.push(all_roots.iter().cloned(), Instant::now());
                        }
                        Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                        Err(RecvTimeoutError::Timeout) => {}
                    }
                    let now = Instant::now();
                    if co.due(now) {
                        let batch = co.take(now).unwrap_or_else(|| all_roots.clone());
                        on_batch(batch);
                        next_tick = now + tick;
                    } else if now >= next_tick {
                        // Polled roots are also checked here: pollers can miss
                        // changes within their timestamp resolution.
                        let polled: Vec<PathBuf> = lock(&worker_watchers)
                            .roots
                            .iter()
                            .filter(|r| r.mode == WatchMode::Polling)
                            .map(|r| r.path.clone())
                            .collect();
                        on_batch(polled);
                        next_tick = now + tick;
                    }
                }
            })
            .ok();
        LiveTail { watchers, tx, worker }
    }

    /// The watched roots and their modes.
    pub fn roots(&self) -> Vec<WatchedRoot> {
        lock(&self.watchers).roots.clone()
    }

    /// The mode a file is watched with (its deepest watched root's).
    pub fn mode_of(&self, path: &Path) -> WatchMode {
        mode_of(&lock(&self.watchers).roots, path)
    }

    /// Delivers a signal as a watcher would (tests, or a shell forwarding errors).
    pub fn signal(&self, s: WatchSignal) {
        let _ = self.tx.send(Msg::Signal(s));
    }
}

impl Drop for LiveTail {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Stop);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
        lock(&self.watchers).running.clear();
    }
}

/// The mode of the deepest root containing `path`; `Off` outside every root.
pub fn mode_of(roots: &[WatchedRoot], path: &Path) -> WatchMode {
    roots
        .iter()
        .filter(|r| path.starts_with(&r.path))
        .max_by_key(|r| r.path.components().count())
        .map_or(WatchMode::Off, |r| r.mode)
}

fn lock(w: &Mutex<Watchers>) -> std::sync::MutexGuard<'_, Watchers> {
    w.lock().unwrap_or_else(|p| p.into_inner())
}

/// (Re)creates one watcher per mode and registers the roots; a root that
/// cannot be watched with events falls back to polling.
fn install(w: &mut Watchers, factory: &dyn WatcherFactory, poll: Duration, sink: &WatchSink, on_fallback: &FallbackFn) {
    w.running.clear();
    for mode in [WatchMode::Events, WatchMode::Polling] {
        if !w.roots.iter().any(|r| r.mode == mode) {
            continue;
        }
        let mut watcher = match factory.create(mode, poll, sink.clone()) {
            Ok(watcher) => watcher,
            Err(e) if mode == WatchMode::Events => {
                let reason = format!("change notifications unavailable: {e:#}");
                for r in w.roots.iter_mut().filter(|r| r.mode == WatchMode::Events) {
                    on_fallback(&r.path, &reason);
                    r.mode = WatchMode::Polling;
                    r.error = Some(reason.clone());
                }
                return install(w, factory, poll, sink, on_fallback);
            }
            Err(e) => {
                for r in w.roots.iter_mut().filter(|r| r.mode == mode) {
                    r.mode = WatchMode::Off;
                    r.error = Some(format!("cannot poll: {e:#}"));
                }
                continue;
            }
        };
        let mut retry = false;
        for r in w.roots.iter_mut().filter(|r| r.mode == mode) {
            if let Err(e) = watcher.watch(&r.path, r.recursive) {
                let reason = format!("{e:#}");
                if mode == WatchMode::Events {
                    on_fallback(&r.path, &reason);
                    r.mode = WatchMode::Polling;
                    retry = true;
                } else {
                    r.mode = WatchMode::Off;
                }
                r.error = Some(reason);
            }
        }
        w.running.push(watcher);
        if retry {
            return install(w, factory, poll, sink, on_fallback);
        }
    }
}

/// A watcher error: every event-watched root falls back to polling.
fn fall_back(w: &mut Watchers, error: &str, factory: &dyn WatcherFactory, poll: Duration, sink: &WatchSink, on_fallback: &FallbackFn) {
    let mut changed = false;
    for r in w.roots.iter_mut().filter(|r| r.mode == WatchMode::Events) {
        on_fallback(&r.path, error);
        r.mode = WatchMode::Polling;
        r.error = Some(error.to_string());
        changed = true;
    }
    if changed {
        install(w, factory, poll, sink, on_fallback);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coalesces_bursts() {
        let s = WatchSettings::default();
        let mut co = Coalescer::new(s);
        let t0 = Instant::now();
        assert!(co.deadline().is_none() && !co.due(t0));
        for i in 0..10_000u64 {
            co.push([PathBuf::from("a.log")], t0 + Duration::from_micros(i * 20));
        }
        // Still arriving: the max delay bounds the wait.
        assert!(!co.due(t0 + Duration::from_millis(150)));
        assert!(co.due(t0 + Duration::from_millis(251)));
        assert_eq!(co.take(t0 + Duration::from_millis(251)).unwrap(), vec![PathBuf::from("a.log")]);
        // The next batch waits for the minimum interval.
        let t1 = t0 + Duration::from_millis(300);
        co.push([PathBuf::from("b.log")], t1);
        assert_eq!(co.deadline().unwrap(), t0 + Duration::from_millis(251 + 250));
        let mut big = Coalescer::new(s);
        big.push((0..MAX_PENDING_PATHS + 1).map(|i| PathBuf::from(i.to_string())), t0);
        assert!(big.take(t0).is_none());
    }

    #[test]
    fn modes() {
        assert_eq!(initial_mode(WatchPreference::Polling, Path::new("/x")), WatchMode::Polling);
        assert_eq!(initial_mode(WatchPreference::Events, Path::new(r"\\s\share")), WatchMode::Events);
        assert_eq!(initial_mode(WatchPreference::Auto, Path::new(r"\\s\share")), WatchMode::Polling);
        assert_eq!(initial_mode(WatchPreference::Auto, Path::new("/x")), WatchMode::Events);
        let roots = vec![
            WatchedRoot {
                path: "/a".into(),
                recursive: true,
                mode: WatchMode::Events,
                error: None,
            },
            WatchedRoot {
                path: "/a/b".into(),
                recursive: true,
                mode: WatchMode::Polling,
                error: None,
            },
        ];
        assert_eq!(mode_of(&roots, Path::new("/a/b/c.log")), WatchMode::Polling);
        assert_eq!(mode_of(&roots, Path::new("/a/c.log")), WatchMode::Events);
        assert_eq!(mode_of(&roots, Path::new("/z")), WatchMode::Off);
    }
}
