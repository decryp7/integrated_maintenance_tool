//! Annotators over the window's store: plugin contributions that derive time ranges and points (reboots, a
//! service's uptime) from the events. They run after every import, reload, refresh and profile change, on that
//! import job's thread (never the UI's), reading the store through the bounded [`PortView`] and called only
//! through the registry's guarded adapters: a failing, panicking or slow annotator keeps its previous set and is
//! reported for its plugin (the plugin's status lists the note; repeated panics quarantine it).

use crate::windows::WindowState;
use imt_core::plugin::annotate::PortView;
use imt_core::ports::Store;

/// Runs the window's annotators over `store` (nothing when no plugin contributes one). `told` is the job's
/// message line while they run.
pub(crate) fn run(win: &WindowState, store: &dyn Store, told: &dyn Fn(&str)) {
    let registry = &win.host.registry;
    if !registry.has_annotators() {
        return;
    }
    told("Running the annotators");
    let started = std::time::Instant::now();
    match PortView::new(store) {
        Ok(view) => registry.run_annotators(&view, None),
        Err(e) => log::warn!("The annotators did not run: the store did not answer ({e})"),
    }
    log::info!(ElapsedMs = started.elapsed().as_millis() as u64; "Annotators ran");
}
