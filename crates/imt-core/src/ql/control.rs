//! Cancellation and progress for long-running queries.
//!
//! A [`QueryControl`] is shared between the code running a query and whoever may stop it (another
//! thread, a UI command). Execution checks it between batches of [`BATCH`] records, so the overhead is
//! one relaxed atomic load per batch and a cancelled query stops within one batch per worker.

use super::error::QlError;
use serde::Serialize;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;

/// Records processed between two cancellation checks (and progress updates).
pub const BATCH: usize = 4096;

/// What a running query is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum QueryStage {
    /// Evaluating the workspace filter (levels, window, search) over every record.
    Matching,
    /// Running the query-language program over the matching records.
    Evaluating,
}

/// A snapshot of a query's progress: `done` of `total` records in the current `stage`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryProgress {
    pub stage: QueryStage,
    pub done: u64,
    pub total: u64,
}

/// Called once when a query is cancelled (e.g. interrupting a database statement).
pub type CancelHook = Box<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct Shared {
    cancelled: AtomicBool,
    stage: AtomicU8,
    done: AtomicU64,
    total: AtomicU64,
    hooks: std::sync::Mutex<Vec<(u64, CancelHook)>>,
    next_hook: AtomicU64,
}

impl fmt::Debug for Shared {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Shared")
            .field("cancelled", &self.cancelled)
            .field("done", &self.done)
            .field("total", &self.total)
            .finish_non_exhaustive()
    }
}

/// Removes its hook from the control when dropped (see [`QueryControl::on_cancel`]).
pub struct HookGuard {
    control: QueryControl,
    id: u64,
}

impl Drop for HookGuard {
    fn drop(&mut self) {
        let mut hooks = self.control.0.hooks.lock().unwrap_or_else(|p| p.into_inner());
        hooks.retain(|(id, _)| *id != self.id);
    }
}

/// Cancel flag and progress counters of one query run. Cheap to clone (all clones share the state).
#[derive(Debug, Clone, Default)]
pub struct QueryControl(Arc<Shared>);

impl QueryControl {
    pub fn new() -> Self {
        Self::default()
    }

    /// Asks the query to stop at its next check, and runs the registered cancel hooks (once).
    pub fn cancel(&self) {
        if self.0.cancelled.swap(true, Ordering::SeqCst) {
            return;
        }
        let hooks = self.0.hooks.lock().unwrap_or_else(|p| p.into_inner());
        for (_, h) in hooks.iter() {
            h();
        }
    }

    /// Runs `hook` when the query is cancelled (at once if it already is) until the returned guard is
    /// dropped. A store registers the interrupt of the statement it is running.
    pub fn on_cancel(&self, hook: CancelHook) -> HookGuard {
        let id = self.0.next_hook.fetch_add(1, Ordering::Relaxed);
        let mut hooks = self.0.hooks.lock().unwrap_or_else(|p| p.into_inner());
        if self.is_cancelled() {
            hook();
        }
        hooks.push((id, hook));
        HookGuard { control: self.clone(), id }
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Relaxed)
    }

    /// `Err(Cancelled)` once [`cancel`](Self::cancel) was called.
    pub fn check(&self) -> Result<(), Cancelled> {
        if self.is_cancelled() {
            Err(Cancelled)
        } else {
            Ok(())
        }
    }

    pub fn progress(&self) -> QueryProgress {
        QueryProgress {
            stage: if self.0.stage.load(Ordering::Relaxed) == 0 {
                QueryStage::Matching
            } else {
                QueryStage::Evaluating
            },
            done: self.0.done.load(Ordering::Relaxed),
            total: self.0.total.load(Ordering::Relaxed),
        }
    }

    /// Starts a stage over `total` records (resets `done`).
    pub fn begin(&self, stage: QueryStage, total: usize) {
        self.0.done.store(0, Ordering::Relaxed);
        self.0.total.store(total as u64, Ordering::Relaxed);
        self.0.stage.store(u8::from(stage == QueryStage::Evaluating), Ordering::Relaxed);
    }

    /// Records `n` more processed records of the current stage.
    pub fn advance(&self, n: usize) {
        self.0.done.fetch_add(n as u64, Ordering::Relaxed);
    }

    /// Wraps `inner` so it stops early (and counts progress) once the query is cancelled. Check
    /// [`is_cancelled`](Self::is_cancelled) afterwards: a short iteration may mean "cancelled".
    pub fn watch<I: Iterator>(&self, inner: I) -> Watched<'_, I> {
        Watched {
            inner,
            control: self,
            since_check: 0,
            counts: true,
        }
    }

    /// Like [`watch`](Self::watch) without counting progress: for later passes over records the
    /// current stage has already counted.
    pub fn guard<I: Iterator>(&self, inner: I) -> Watched<'_, I> {
        Watched {
            counts: false,
            ..self.watch(inner)
        }
    }

    fn advance_if(&self, counts: bool, n: usize) {
        if counts {
            self.advance(n);
        }
    }
}

/// The query was cancelled through its [`QueryControl`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

impl fmt::Display for Cancelled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the query was cancelled")
    }
}

impl std::error::Error for Cancelled {}

/// Why a controlled query run produced no result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    /// The program is invalid or failed (with the span to underline).
    Invalid(QlError),
    /// Stopped through its [`QueryControl`]; not an error the user has to fix.
    Cancelled,
}

impl RunError {
    /// For callers without a cancel path: `Cancelled` becomes a plain error without a span.
    pub fn into_ql_error(self) -> QlError {
        match self {
            RunError::Invalid(e) => e,
            RunError::Cancelled => QlError::new(Cancelled.to_string(), 0, 0),
        }
    }
}

impl From<QlError> for RunError {
    fn from(e: QlError) -> Self {
        RunError::Invalid(e)
    }
}

impl From<Cancelled> for RunError {
    fn from(_: Cancelled) -> Self {
        RunError::Cancelled
    }
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunError::Invalid(e) => e.fmt(f),
            RunError::Cancelled => Cancelled.fmt(f),
        }
    }
}

impl std::error::Error for RunError {}

/// See [`QueryControl::watch`].
pub struct Watched<'c, I> {
    inner: I,
    control: &'c QueryControl,
    since_check: usize,
    counts: bool,
}

impl<I: Iterator> Iterator for Watched<'_, I> {
    type Item = I::Item;

    fn next(&mut self) -> Option<I::Item> {
        if self.since_check == BATCH {
            self.control.advance_if(self.counts, BATCH);
            self.since_check = 0;
            if self.control.is_cancelled() {
                return None;
            }
        }
        let item = self.inner.next();
        if item.is_some() {
            self.since_check += 1;
        } else {
            self.control.advance_if(self.counts, self.since_check);
            self.since_check = 0;
        }
        item
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_counts_progress_and_stops_after_cancel() {
        let c = QueryControl::new();
        c.begin(QueryStage::Evaluating, 10_000);
        assert_eq!(c.watch(0..10_000).count(), 10_000);
        assert_eq!(
            c.progress(),
            QueryProgress {
                stage: QueryStage::Evaluating,
                done: 10_000,
                total: 10_000
            }
        );

        c.begin(QueryStage::Matching, 1_000_000);
        let mut seen = 0;
        for _ in c.watch(0..1_000_000) {
            seen += 1;
            if seen == 10 {
                c.cancel();
            }
        }
        assert_eq!(seen, BATCH, "stops at the next batch boundary");
        assert_eq!(c.progress().stage, QueryStage::Matching);
        let before = c.progress().done;
        assert_eq!(
            c.guard(0..100_000).count(),
            BATCH,
            "a cancelled guard stops at the first batch check"
        );
        let fresh = QueryControl::new();
        assert_eq!(fresh.guard(0..10_000).count(), 10_000);
        assert_eq!(fresh.progress().done, 0, "guard does not count");
        assert_eq!(c.progress().done, before);
        assert_eq!(c.check(), Err(Cancelled));
        assert_eq!(RunError::from(Cancelled), RunError::Cancelled);
        assert_eq!(RunError::Cancelled.to_string(), "the query was cancelled");
        let e = QlError::new("bad", 1, 2);
        assert_eq!(RunError::from(e.clone()).to_string(), e.to_string());
        assert_eq!(RunError::from(e.clone()).into_ql_error(), e);
        assert_eq!(RunError::Cancelled.into_ql_error(), QlError::new("the query was cancelled", 0, 0));
    }

    fn records(n: usize) -> Vec<crate::model::Record> {
        (0..n)
            .map(|i| {
                let mut r = crate::model::Record::new(i as u32);
                r.message = format!("m{i}");
                r.fields.insert("N".into(), serde_json::json!(i % 10));
                r
            })
            .collect()
    }

    /// Yields `records` and cancels `control` after `after` of them.
    fn cancelling<'a>(
        records: &'a [crate::model::Record],
        control: &'a QueryControl,
        after: usize,
    ) -> impl Iterator<Item = (&'a crate::model::Record, &'a str)> + 'a {
        records.iter().enumerate().map(move |(i, r)| {
            if i == after {
                control.cancel();
            }
            (r, "s")
        })
    }

    #[test]
    fn every_stage_stops_on_cancel() {
        let recs = records(BATCH * 4);
        let program = super::super::parse("N >= 0 | where N < 5 | sort N | stats count() by N").unwrap();
        // Cancelled while filtering (stages are covered in `exec`).
        for (ql, after) in [
            ("N >= 0", 10),
            ("N >= 0 | where N < 5", BATCH * 4 - 1),
            ("N >= 0 | sort N", BATCH * 4 - 1),
            ("N >= 0 | stats count() by N", BATCH * 4 - 1),
            ("N >= 0 | head 1", BATCH * 4 - 1),
        ] {
            let program = super::super::parse(ql).unwrap();
            let control = QueryControl::new();
            let out = super::super::run_with(&program, &mut cancelling(&recs, &control, after), &control);
            assert_eq!(out, Err(RunError::Cancelled), "{ql}");
        }
        // Not cancelled: the same result as `run`.
        let control = QueryControl::new();
        let a = super::super::run_with(&program, &mut recs.iter().map(|r| (r, "s")), &control).unwrap();
        let b = super::super::run(&program, &mut recs.iter().map(|r| (r, "s"))).unwrap();
        assert_eq!(a, b);
        let p = control.progress();
        assert_eq!(
            (p.stage, p.done, p.total),
            (QueryStage::Evaluating, recs.len() as u64, recs.len() as u64)
        );
    }

    #[test]
    fn clones_share_state() {
        let a = QueryControl::new();
        let b = a.clone();
        assert!(a.check().is_ok());
        b.cancel();
        b.cancel();
        assert!(a.is_cancelled());
    }

    #[test]
    fn cancel_hooks_run_once_and_unregister() {
        use std::sync::atomic::AtomicUsize;
        let control = QueryControl::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let guard = control.on_cancel(Box::new(move || {
            c.fetch_add(1, Ordering::SeqCst);
        }));
        let dropped = calls.clone();
        drop(control.on_cancel(Box::new(move || {
            dropped.fetch_add(100, Ordering::SeqCst);
        })));
        control.cancel();
        control.cancel();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        drop(guard);
        // Registering after the cancel runs the hook at once.
        let late = calls.clone();
        let _g = control.on_cancel(Box::new(move || {
            late.fetch_add(10, Ordering::SeqCst);
        }));
        assert_eq!(calls.load(Ordering::SeqCst), 11);
        assert!(format!("{control:?}").contains("cancelled"));
    }
}
