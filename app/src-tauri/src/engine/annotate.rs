//! Annotators over the window's store: plugin contributions that derive time ranges and points (reboots, a
//! service's uptime) from the events. They run after every import, reload, refresh and profile change, on that
//! import job's thread (never the UI's), reading the store through the bounded [`PortView`] and called only
//! through the registry's guarded adapters: a failing, panicking or slow annotator keeps its previous set and is
//! reported for its plugin (the plugin's status lists the note; repeated panics quarantine it).

use crate::windows::WindowState;
use imt_core::plugin::annotate::{PortView, RecordCursor};
use imt_core::ports::{NewEvents, Store};
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
mod tests {
    use super::*;

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
}
