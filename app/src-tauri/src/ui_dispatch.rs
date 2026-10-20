//! One way to update the native UI (tray menu and tooltip, the window list) from any thread: the work is
//! posted to the main thread with `run_on_main_thread` and nobody waits for it, and requests made while
//! one is still pending coalesce into it. Tray and window getters wait for the main thread when called
//! elsewhere, so calling them from a worker that holds a lock the main thread needs deadlocks; posting
//! here never does. Callers must not hold their own locks across [`post`] either (it may run the work at
//! once when already on the main thread, or in tests).

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Manager, Runtime};

/// What a request updates; one pending request per slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Tray,
    WindowList,
}

const SLOTS: usize = 2;

/// The pending flags (managed state, created on first use).
#[derive(Default)]
pub struct UiDispatch {
    pending: [AtomicBool; SLOTS],
}

impl UiDispatch {
    /// Whether this request must post (no request for `slot` is pending); otherwise it coalesces.
    fn claim(&self, slot: Slot) -> bool {
        !self.pending[slot as usize].swap(true, Ordering::SeqCst)
    }

    /// The posted work starts: later requests post again (they may see newer state).
    fn release(&self, slot: Slot) {
        self.pending[slot as usize].store(false, Ordering::SeqCst);
    }
}

/// Runs `work` on the main thread soon, without waiting; coalesces with a pending request for `slot`.
pub fn post<R: Runtime>(app: &AppHandle<R>, slot: Slot, work: fn(&AppHandle<R>)) {
    if app.try_state::<UiDispatch>().is_none() {
        app.manage(UiDispatch::default());
    }
    let Some(dispatch) = app.try_state::<UiDispatch>() else { return };
    if !dispatch.claim(slot) {
        return;
    }
    let handle = app.clone();
    let posted = app.run_on_main_thread(move || {
        if let Some(d) = handle.try_state::<UiDispatch>() {
            d.release(slot);
        }
        work(&handle);
    });
    if let Err(e) = posted {
        // The event loop is gone (shutting down): nothing left to update.
        dispatch.release(slot);
        log::debug!("native UI update not posted: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn requests_coalesce_per_slot_until_the_work_starts() {
        let d = UiDispatch::default();
        assert!(d.claim(Slot::Tray));
        assert!(!d.claim(Slot::Tray), "coalesced into the pending one");
        assert!(d.claim(Slot::WindowList), "slots are independent");
        d.release(Slot::Tray);
        assert!(d.claim(Slot::Tray));
    }

    static RAN: AtomicUsize = AtomicUsize::new(0);

    fn count<R: Runtime>(_: &AppHandle<R>) {
        RAN.fetch_add(1, Ordering::SeqCst);
    }

    #[test]
    fn posted_work_runs_and_releases_its_slot() {
        let app = crate::test_support::test_app(None);
        let before = RAN.load(Ordering::SeqCst);
        post(app.handle(), Slot::Tray, count);
        post(app.handle(), Slot::Tray, count);
        // The mock runtime runs main-thread work at once, so neither request is left pending.
        assert_eq!(RAN.load(Ordering::SeqCst), before + 2);
        assert!(app.state::<UiDispatch>().claim(Slot::Tray));
    }
}
