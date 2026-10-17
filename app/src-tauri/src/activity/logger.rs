//! The logger in front of the log plugin: records activity entries as breadcrumbs, deduplicates repeated
//! errors, and attaches the latest breadcrumbs to every error so the log shows what led to it.

use super::breadcrumbs::{Breadcrumb, Breadcrumbs, BREADCRUMBS_PER_ERROR};
use super::SOURCE_CONTEXT;
use log::kv::{Error as KvError, Key, Source, Value, VisitSource};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Repeated identical errors within this window are logged once, then summarised.
pub const DEDUP_WINDOW: Duration = Duration::from_secs(60);
/// Most distinct errors tracked for deduplication.
const MAX_DEDUP_KEYS: usize = 256;
/// Breadcrumbs attached to one error are capped at this many bytes of JSON.
const MAX_BREADCRUMB_JSON: usize = 16 * 1024;

fn ring() -> &'static Mutex<Breadcrumbs> {
    static RING: OnceLock<Mutex<Breadcrumbs>> = OnceLock::new();
    RING.get_or_init(|| Mutex::new(Breadcrumbs::default()))
}

/// The latest activity entries of `window` (every window's when it has none), oldest first.
pub fn breadcrumbs(window: Option<&str>, n: usize) -> Vec<Breadcrumb> {
    ring().lock().unwrap_or_else(|e| e.into_inner()).recent_for(window, n)
}

/// What [`ErrorDedup`] decides for one error.
#[derive(Debug, PartialEq, Eq)]
pub enum Dedup {
    Log,
    /// Log it, after a summary: it was suppressed `n` times in the previous window.
    LogAfterRepeats(u64),
    Suppress,
}

/// Logs the first of identical errors in each [`DEDUP_WINDOW`], counts the rest.
#[derive(Debug, Default)]
pub struct ErrorDedup {
    seen: HashMap<String, (Instant, u64)>,
}

impl ErrorDedup {
    pub fn check(&mut self, key: &str, now: Instant) -> Dedup {
        if self.seen.len() >= MAX_DEDUP_KEYS && !self.seen.contains_key(key) {
            self.seen.retain(|_, (t, _)| now.duration_since(*t) < DEDUP_WINDOW);
            if self.seen.len() >= MAX_DEDUP_KEYS {
                self.seen.clear();
            }
        }
        match self.seen.get_mut(key) {
            Some((start, n)) if now.duration_since(*start) < DEDUP_WINDOW => {
                *n += 1;
                Dedup::Suppress
            }
            Some((start, n)) => {
                let repeats = *n;
                *start = now;
                *n = 0;
                if repeats > 0 {
                    Dedup::LogAfterRepeats(repeats)
                } else {
                    Dedup::Log
                }
            }
            None => {
                self.seen.insert(key.to_string(), (now, 0));
                Dedup::Log
            }
        }
    }

    /// Errors with pending repeats (key, count); clears them.
    pub fn drain_repeats(&mut self) -> Vec<(String, u64)> {
        let out = self
            .seen
            .iter()
            .filter(|(_, (_, n))| *n > 0)
            .map(|(k, (_, n))| (k.clone(), *n))
            .collect();
        for (_, n) in self.seen.values_mut() {
            *n = 0;
        }
        out
    }
}

/// A record's properties as text.
struct Props(BTreeMap<String, String>);

impl<'kvs> VisitSource<'kvs> for Props {
    fn visit_pair(&mut self, key: Key<'kvs>, value: Value<'kvs>) -> Result<(), KvError> {
        self.0.insert(key.as_str().to_string(), value.to_string());
        Ok(())
    }
}

/// The record's properties followed by `extra`.
struct WithExtra<'a> {
    base: &'a dyn Source,
    extra: &'a [(&'static str, String)],
}

impl Source for WithExtra<'_> {
    fn visit<'kvs>(&'kvs self, visitor: &mut dyn VisitSource<'kvs>) -> Result<(), KvError> {
        self.base.visit(visitor)?;
        for (k, v) in self.extra {
            visitor.visit_pair(Key::from_str(k), Value::from(v.as_str()))?;
        }
        Ok(())
    }
}

/// Wraps the log plugin's logger (see the module docs). Never panics and never blocks on a poisoned lock.
pub struct ActivityLogger {
    inner: Box<dyn log::Log>,
    dedup: Mutex<ErrorDedup>,
}

impl ActivityLogger {
    pub fn new(inner: Box<dyn log::Log>) -> Self {
        ActivityLogger {
            inner,
            dedup: Mutex::new(ErrorDedup::default()),
        }
    }

    fn forward(&self, record: &log::Record, extra: &[(&'static str, String)]) {
        if extra.is_empty() {
            return self.inner.log(record);
        }
        let kvs = WithExtra {
            base: record.key_values(),
            extra,
        };
        self.inner.log(
            &log::Record::builder()
                .level(record.level())
                .target(record.target())
                .module_path(record.module_path())
                .file(record.file())
                .line(record.line())
                .key_values(&kvs)
                .args(*record.args())
                .build(),
        );
    }

    fn summary(&self, record: &log::Record, n: u64) {
        let extra = [
            ("Repeats", n.to_string()),
            ("DedupWindowSeconds", DEDUP_WINDOW.as_secs().to_string()),
        ];
        let kvs = WithExtra {
            base: record.key_values(),
            extra: &extra,
        };
        self.inner.log(
            &log::Record::builder()
                .level(record.level())
                .target(record.target())
                .key_values(&kvs)
                .args(format_args!(
                    "{} (repeated {n} times in {}s)",
                    record.args(),
                    DEDUP_WINDOW.as_secs()
                ))
                .build(),
        );
    }
}

impl log::Log for ActivityLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        self.inner.enabled(metadata)
    }

    fn log(&self, record: &log::Record) {
        let mut props = Props(BTreeMap::new());
        let _ = record.key_values().visit(&mut props);
        let props = props.0;
        let context = props.get("SourceContext").map(String::as_str).unwrap_or(record.target());
        if context == SOURCE_CONTEXT {
            let time = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
            if let Some(b) = Breadcrumb::from_props(time, &props) {
                ring().lock().unwrap_or_else(|e| e.into_inner()).push(b);
            }
        }
        let fatal = props.get("Severity").is_some_and(|s| s == "Fatal");
        if record.level() != log::Level::Error && !fatal {
            return self.inner.log(record);
        }
        let key = format!("{context}\u{1f}{}", record.args());
        match self.dedup.lock().unwrap_or_else(|e| e.into_inner()).check(&key, Instant::now()) {
            Dedup::Suppress if !fatal => return,
            Dedup::LogAfterRepeats(n) => self.summary(record, n),
            _ => {}
        }
        let window = props.get("Window").map(String::as_str);
        let crumbs = breadcrumbs(window, BREADCRUMBS_PER_ERROR);
        let mut extra: Vec<(&'static str, String)> = Vec::new();
        if !crumbs.is_empty() && !props.contains_key("Breadcrumbs") {
            let mut crumbs = crumbs;
            let mut json = serde_json::to_string(&crumbs).unwrap_or_default();
            while json.len() > MAX_BREADCRUMB_JSON && !crumbs.is_empty() {
                crumbs.remove(0);
                json = serde_json::to_string(&crumbs).unwrap_or_default();
            }
            extra.push(("Breadcrumbs", json));
            extra.push(("StepsToReproduce", super::steps::steps_text(&crumbs, None)));
        }
        self.forward(record, &extra);
    }

    fn flush(&self) {
        let pending = self.dedup.lock().unwrap_or_else(|e| e.into_inner()).drain_repeats();
        for (key, n) in pending {
            let (context, message) = key.split_once('\u{1f}').unwrap_or(("", key.as_str()));
            log::Log::log(
                &*self.inner,
                &log::Record::builder()
                    .level(log::Level::Error)
                    .target(context)
                    .args(format_args!("{message} (repeated {n} times in {}s)", DEDUP_WINDOW.as_secs()))
                    .build(),
            );
        }
        self.inner.flush();
    }
}

#[cfg(test)]
pub(crate) use test_support::{captured, test_capture, test_lock, Captured};

#[cfg(test)]
mod test_support {
    use super::*;
    use std::sync::MutexGuard;

    /// One record as the capture logger saw it.
    #[derive(Debug, Clone)]
    pub struct Captured {
        pub level: log::Level,
        pub target: String,
        pub message: String,
        pub props: BTreeMap<String, String>,
    }

    struct Capture;
    fn sink() -> &'static Mutex<Vec<Captured>> {
        static SINK: OnceLock<Mutex<Vec<Captured>>> = OnceLock::new();
        SINK.get_or_init(|| Mutex::new(Vec::new()))
    }
    impl log::Log for Capture {
        fn enabled(&self, _: &log::Metadata) -> bool {
            true
        }
        fn log(&self, record: &log::Record) {
            let mut props = Props(BTreeMap::new());
            let _ = record.key_values().visit(&mut props);
            sink().lock().unwrap_or_else(|e| e.into_inner()).push(Captured {
                level: record.level(),
                target: record.target().to_string(),
                message: record.args().to_string(),
                props: props.0,
            });
        }
        fn flush(&self) {}
    }

    /// Serialises tests that touch process-wide logging state.
    pub fn test_lock() -> MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Installs the activity logger over a capturing logger (once per process), clears what it captured,
    /// and holds the test lock.
    pub fn test_capture() -> MutexGuard<'static, ()> {
        let guard = test_lock();
        static INSTALLED: OnceLock<bool> = OnceLock::new();
        INSTALLED.get_or_init(|| log::set_boxed_logger(Box::new(ActivityLogger::new(Box::new(Capture)))).is_ok());
        log::set_max_level(log::LevelFilter::Trace);
        sink().lock().unwrap_or_else(|e| e.into_inner()).clear();
        guard
    }

    /// Everything captured since [`test_capture`].
    pub fn captured() -> Vec<Captured> {
        sink().lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn activity(op: &str, window: &str) {
        log::info!(target: SOURCE_CONTEXT, SourceContext = SOURCE_CONTEXT, Operation = op, Trigger = "menu", Window = window, OperationId = "op-1"; "User {op} via menu");
    }

    #[test]
    fn errors_carry_the_breadcrumbs_of_their_window() {
        let _g = test_capture();
        activity("file.open", "w-crumbs");
        activity("export.csv", "w-crumbs");
        log::error!(Window = "w-crumbs"; "Export failed: disk full (crumb test)");
        let all = captured();
        let err = all.iter().find(|c| c.message.contains("crumb test")).expect("error logged");
        let crumbs: Vec<Breadcrumb> = serde_json::from_str(&err.props["Breadcrumbs"]).unwrap();
        let ops: Vec<&str> = crumbs.iter().map(|b| b.operation.as_str()).collect();
        assert_eq!(ops, ["file.open", "export.csv"]);
        assert!(err.props["StepsToReproduce"].contains("1."), "{}", err.props["StepsToReproduce"]);
        assert_eq!(breadcrumbs(Some("w-crumbs"), 1)[0].operation, "export.csv");
        // Non-errors pass through untouched.
        let info = all.iter().find(|c| c.message == "User export.csv via menu").unwrap();
        assert!(!info.props.contains_key("Breadcrumbs"));
    }

    #[test]
    fn repeated_errors_are_deduplicated() {
        let _g = test_capture();
        for _ in 0..5 {
            log::error!("Watcher failed (dedup test)");
        }
        let n = captured().iter().filter(|c| c.message.contains("dedup test")).count();
        assert_eq!(n, 1);
        log::logger().flush();
        let summary = captured()
            .into_iter()
            .find(|c| c.message.contains("repeated 4 times"))
            .expect("summary");
        assert_eq!(summary.level, log::Level::Error);
        // Fatal entries are never suppressed.
        for _ in 0..2 {
            log::error!(Severity = "Fatal"; "Crash (fatal dedup test)");
        }
        assert_eq!(captured().iter().filter(|c| c.message.contains("fatal dedup test")).count(), 2);
    }

    #[test]
    fn dedup_policy() {
        let t0 = Instant::now();
        let mut d = ErrorDedup::default();
        assert_eq!(d.check("a", t0), Dedup::Log);
        assert_eq!(d.check("a", t0 + Duration::from_secs(1)), Dedup::Suppress);
        assert_eq!(d.check("a", t0 + Duration::from_secs(2)), Dedup::Suppress);
        assert_eq!(d.check("b", t0), Dedup::Log);
        assert_eq!(d.check("a", t0 + DEDUP_WINDOW), Dedup::LogAfterRepeats(2));
        assert_eq!(d.check("a", t0 + DEDUP_WINDOW * 3), Dedup::Log);
        assert_eq!(d.check("a", t0 + DEDUP_WINDOW * 3), Dedup::Suppress);
        assert_eq!(d.drain_repeats(), vec![("a".to_string(), 1)]);
        assert!(d.drain_repeats().is_empty());
        for i in 0..(MAX_DEDUP_KEYS * 2) {
            d.check(&format!("k{i}"), t0 + DEDUP_WINDOW * 3);
        }
        assert!(d.seen.len() <= MAX_DEDUP_KEYS);
    }
}
