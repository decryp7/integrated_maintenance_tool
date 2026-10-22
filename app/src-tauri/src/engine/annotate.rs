//! Annotators over the window's store: plugin contributions that derive time ranges and points (reboots, a
//! service's uptime) from the events. They run after every import, reload, refresh and profile change, on that
//! import job's thread (never the UI's), reading the store through the bounded [`PortView`] and called only
//! through the registry's guarded adapters: a failing, panicking or slow annotator keeps its previous set and is
//! reported for its plugin (the plugin's status lists the note; repeated panics quarantine it).

use crate::windows::WindowState;
use debrief_core::plugin::annotate::{PortView, RecordCursor};
use debrief_core::ports::{NewEvents, Store};
use std::time::{Duration, Instant};

/// Annotators run at most this often while live tail appends (their work is the appended events since).
pub(crate) const TAIL_EVERY: Duration = Duration::from_secs(1);

/// Runs the window's annotators over `store` (nothing when no plugin contributes one). `told` is the job's
/// message line while they run.
pub(crate) fn run(win: &WindowState, store: &dyn Store, told: &dyn Fn(&str)) {
    if !win.host.registry.has_annotators() {
        return;
    }
    told("Running the annotators");
    run_since(win, store, None);
}

/// The first new event of two refreshes: the earlier id and the earlier time.
pub(crate) fn merge(a: Option<RecordCursor>, b: RecordCursor) -> RecordCursor {
    match a {
        None => b,
        Some(a) => RecordCursor {
            id: a.id.min(b.id),
            ts: match (a.ts, b.ts) {
                (Some(x), Some(y)) => Some(x.min(y)),
                (x, y) => x.or(y),
            },
        },
    }
}

/// The cursor for events a tail refresh added.
pub(crate) fn cursor_of(new: NewEvents) -> RecordCursor {
    RecordCursor {
        id: u32::try_from(new.id).unwrap_or(u32::MAX),
        ts: new.ts,
    }
}

/// After a live tail refresh: runs the annotators from the first new event, at most once per [`TAIL_EVERY`]; what
/// arrived meanwhile waits in `pending` (merged) for the next run. Returns whether something is still pending (the
/// caller asks again later).
pub(crate) fn after_tail(
    win: &WindowState,
    store: &dyn Store,
    pending: &mut Option<RecordCursor>,
    last: &mut Option<Instant>,
    new: Option<NewEvents>,
    now: Instant,
) -> bool {
    if !win.host.registry.has_annotators() {
        *pending = None;
        return false;
    }
    if let Some(new) = new {
        *pending = Some(merge(*pending, cursor_of(new)));
    }
    let Some(since) = *pending else { return false };
    if last.is_some_and(|t| now.duration_since(t) < TAIL_EVERY) {
        return true;
    }
    *pending = None;
    *last = Some(now);
    run_since(win, store, Some(since));
    false
}

fn run_since(win: &WindowState, store: &dyn Store, since: Option<RecordCursor>) {
    let registry = &win.host.registry;
    let started = std::time::Instant::now();
    match PortView::new(store) {
        Ok(view) => registry.run_annotators(&view, since),
        Err(e) => log::warn!("The annotators did not run: the store did not answer ({e})"),
    }
    log::info!(ElapsedMs = started.elapsed().as_millis() as u64; "Annotators ran");
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use debrief_core::plugin::annotate::{AnnotationView, Annotator};
    use std::sync::{Arc, Mutex};

    /// An annotator that records the cursor of each run.
    #[derive(Default)]
    pub(crate) struct Recorder(pub Mutex<Vec<Option<RecordCursor>>>);

    struct RecorderPlugin(Arc<Recorder>);

    impl Annotator for Recorder {
        fn id(&self) -> &str {
            "test.recorder"
        }
        fn annotate(
            &self,
            _: &dyn AnnotationView,
            since: Option<RecordCursor>,
        ) -> anyhow::Result<Vec<debrief_core::plugin::annotate::Annotation>> {
            self.0.lock().unwrap().push(since);
            Ok(Vec::new())
        }
    }

    impl debrief_core::Plugin for RecorderPlugin {
        fn id(&self) -> &str {
            "test.recorder-plugin"
        }
        fn name(&self) -> &str {
            "Recorder"
        }
        fn version(&self) -> &str {
            "1.0.0"
        }
        fn register(&self, registry: &mut debrief_core::PluginRegistry) -> anyhow::Result<()> {
            registry.add_annotator(self.0.clone()).map(|_| ())
        }
    }

    /// A registry with an annotator that records its runs.
    pub(crate) fn recording_registry() -> (debrief_core::PluginRegistry, Arc<Recorder>) {
        let recorder = Arc::new(Recorder::default());
        let mut registry = debrief_core::PluginRegistry::new();
        registry.install_shared(Arc::new(RecorderPlugin(recorder.clone()))).unwrap();
        (registry, recorder)
    }

    #[test]
    fn skipped_refreshes_merge_into_the_earliest_cursor() {
        let a = RecordCursor { id: 40, ts: Some(500) };
        let b = RecordCursor { id: 90, ts: Some(300) };
        assert_eq!(merge(None, a), a);
        assert_eq!(merge(Some(a), b), RecordCursor { id: 40, ts: Some(300) });
        assert_eq!(
            merge(Some(a), RecordCursor { id: 99, ts: None }),
            RecordCursor { id: 40, ts: Some(500) }
        );
        assert_eq!(cursor_of(NewEvents { id: u64::MAX, ts: None }).id, u32::MAX);
    }

    fn new_at(id: u64, ts: i64) -> Option<NewEvents> {
        Some(NewEvents { id, ts: Some(ts) })
    }

    /// While live tail appends, the annotators run at most once per `TAIL_EVERY`; what arrived meanwhile waits,
    /// merged, and the caller is told to ask again.
    #[test]
    fn the_tail_throttles_the_annotators_and_runs_what_waited_from_the_earliest_cursor() {
        let (registry, recorder) = recording_registry();
        let win = crate::windows::tests::window_state("annotate-throttle", registry);
        let store = win.engine.store().unwrap();
        let (mut pending, mut last) = (None, None);
        let t0 = Instant::now();
        let runs = || recorder.0.lock().unwrap().clone();

        // The first refresh runs at once, from its own first new event.
        assert!(!after_tail(&win, &*store, &mut pending, &mut last, new_at(50, 700), t0));
        assert_eq!(runs(), [Some(RecordCursor { id: 50, ts: Some(700) })]);
        assert_eq!(pending, None);

        // Two more inside the window wait, merged into the earliest cursor; the caller must ask again.
        assert!(after_tail(
            &win,
            &*store,
            &mut pending,
            &mut last,
            new_at(90, 900),
            t0 + TAIL_EVERY / 4
        ));
        assert!(after_tail(
            &win,
            &*store,
            &mut pending,
            &mut last,
            new_at(70, 800),
            t0 + TAIL_EVERY / 2
        ));
        assert_eq!(runs().len(), 1, "throttled");
        assert_eq!(pending, Some(RecordCursor { id: 70, ts: Some(800) }));
        // Asking again with nothing new is still too early.
        assert!(after_tail(&win, &*store, &mut pending, &mut last, None, t0 + TAIL_EVERY / 2));
        assert_eq!(runs().len(), 1);

        // Once the window is over, a refresh with nothing new runs what waited, from the earliest cursor.
        assert!(!after_tail(&win, &*store, &mut pending, &mut last, None, t0 + TAIL_EVERY));
        assert_eq!(runs().len(), 2);
        assert_eq!(runs()[1], Some(RecordCursor { id: 70, ts: Some(800) }));
        assert_eq!(pending, None);
        // Nothing pending and nothing new: nothing runs and nothing is asked.
        assert!(!after_tail(&win, &*store, &mut pending, &mut last, None, t0 + TAIL_EVERY * 5));
        assert_eq!(runs().len(), 2);
    }

    #[test]
    fn without_annotators_the_tail_keeps_nothing_pending() {
        let win = crate::windows::tests::window_state("annotate-none", debrief_core::PluginRegistry::new());
        let store = win.engine.store().unwrap();
        let mut pending = Some(RecordCursor { id: 1, ts: None });
        let mut last = None;
        assert!(!after_tail(&win, &*store, &mut pending, &mut last, new_at(5, 5), Instant::now()));
        assert_eq!((pending, last), (None, None));
    }
}
