use super::*;
use crate::health::model::{Change, ServiceState};
use std::sync::atomic::{AtomicUsize, Ordering};

type Seen = Arc<Mutex<Vec<Change>>>;

fn health() -> (Arc<Health>, Seen) {
    let seen: Seen = Arc::default();
    let s = seen.clone();
    (Health::new(Box::new(move |_, c| lock(&s).push(c.clone()))), seen)
}

fn count(seen: &Seen, service: &str, to: ServiceState) -> usize {
    lock(seen).iter().filter(|c| c.service == service && c.to == to).count()
}

#[test]
fn the_store_worker_dying_is_one_outage_however_many_calls_meet_it() {
    let (h, seen) = health();
    for _ in 0..3 {
        h.store_failed(StoreFailure::Crashed, "the store worker exited (signal: 6)");
    }
    assert_eq!(lock(&seen).len(), 1, "one announcement");
    let s = h.snapshot().services.into_iter().find(|s| s.service == STORE).unwrap();
    assert_eq!((s.state, s.repair), (ServiceState::Down, Some(RepairAction::Restart)));
    let cause = s.cause.unwrap();
    assert_eq!(cause.message, StoreFailure::Crashed.message());
    assert!(cause.detail.unwrap().contains("signal: 6"));
}

#[test]
fn a_quarantined_store_is_reset_and_a_missing_library_is_told_apart() {
    let (h, _) = health();
    h.store_failed(StoreFailure::Quarantined, "was stopped; reset it");
    let s = h.registry.get(STORE).unwrap();
    assert_eq!((s.state, s.repair), (ServiceState::Quarantined, Some(RepairAction::Reset)));
    h.store_failed(StoreFailure::LibraryMissing, "image not found");
    assert_eq!(h.registry.get(STORE).unwrap().state, ServiceState::Down);
}

#[test]
fn repairing_the_store_restarts_it_through_the_shell_and_clears_the_outage() {
    let (h, seen) = health();
    let runs = Arc::new(AtomicUsize::new(0));
    let r = runs.clone();
    h.set_repair(
        STORE,
        Arc::new(move || {
            r.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }),
    );
    h.store_failed(StoreFailure::Crashed, "x");
    let id = h.begin_repair("store").unwrap();
    assert_eq!(h.state(STORE), Some(ServiceState::Repairing));
    h.finish_repair(id);
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    assert_eq!(h.state(STORE), Some(ServiceState::Ok));
    assert!(h.store_failure().is_none());
    assert_eq!(count(&seen, STORE, ServiceState::Ok), 1);
}

#[test]
fn a_repair_that_fails_leaves_the_service_down_with_the_new_cause_and_the_same_repair() {
    let (h, _) = health();
    h.set_repair(STORE, Arc::new(|| Err(Cause::new("still refused").hint("reinstall"))));
    h.store_failed(StoreFailure::LibraryRefused, "x");
    let id = h.begin_repair("store").unwrap();
    h.finish_repair(id);
    let s = h.registry.get(STORE).unwrap();
    assert_eq!((s.state, s.repair), (ServiceState::Down, Some(RepairAction::Restart)));
    assert_eq!(s.cause.unwrap().message, "still refused");
}

#[test]
fn a_repair_that_panics_is_contained() {
    let (h, _) = health();
    h.set_repair(AI, Arc::new(|| panic!("boom")));
    h.ai(AiHealth::Switched, Some("crashed 3 times"));
    let id = h.begin_repair("ai").unwrap();
    h.finish_repair(id);
    assert_eq!(h.state(AI), Some(ServiceState::Down));
}

#[test]
fn repair_is_refused_for_a_working_unknown_or_unrepairable_service() {
    let (h, _) = health();
    assert!(h.begin_repair("store").unwrap_err().contains("nothing to repair"));
    assert!(h.begin_repair("nope").is_err());
    h.store_failed(StoreFailure::Crashed, "x");
    assert!(
        h.begin_repair("store").unwrap_err().contains("nothing repairs"),
        "no repair registered"
    );
    h.set_repair(STORE, Arc::new(|| Ok(())));
    assert!(h.begin_repair("store").is_ok());
    assert!(h.begin_repair("store").is_ok(), "asking again while repairing is fine");
}

#[test]
fn the_background_repair_ends_with_the_service_back() {
    let (h, _) = health();
    h.set_repair(STORE, Arc::new(|| Ok(())));
    h.store_failed(StoreFailure::Crashed, "x");
    h.repair("store").unwrap();
    for _ in 0..200 {
        if h.state(STORE) == Some(ServiceState::Ok) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the repair did not finish");
}

#[test]
fn the_ai_worker_crash_is_one_quarantine_with_a_reset_and_clears_when_it_runs() {
    let (h, seen) = health();
    h.ai(AiHealth::Switched, Some("the AI worker failed 3 times"));
    h.ai(AiHealth::Switched, Some("the AI worker failed 3 times"));
    let s = h.registry.get(AI).unwrap();
    assert_eq!((s.state, s.repair), (ServiceState::Quarantined, Some(RepairAction::Reset)));
    assert_eq!(lock(&seen).len(), 1);
    h.ai(AiHealth::LoadFailed, Some("out of memory"));
    assert_eq!(h.state(AI), Some(ServiceState::Down));
    h.ai(AiHealth::Fine, None);
    assert_eq!(h.state(AI), Some(ServiceState::Ok));
}

#[test]
fn plugins_are_one_service_that_lists_who_was_switched_off() {
    let (h, _) = health();
    h.plugin_quarantined("net.a", "A", "crashed");
    let one = h.registry.get(PLUGINS).unwrap();
    assert_eq!(
        (one.state, one.repair),
        (ServiceState::Quarantined, Some(RepairAction::LiftQuarantine))
    );
    assert!(one.cause.unwrap().message.contains("net.a"));
    h.plugin_quarantined("net.b", "B", "crashed");
    assert!(h.registry.get(PLUGINS).unwrap().cause.unwrap().message.starts_with("2 plugins"));
    assert_eq!(h.quarantined_plugins(), ["net.a", "net.b"]);
    h.plugin_reset("net.a");
    assert_eq!(h.state(PLUGINS), Some(ServiceState::Quarantined), "one is still off");
    h.plugin_reset("net.b");
    assert_eq!(h.state(PLUGINS), Some(ServiceState::Ok));
}

#[test]
fn a_missing_browser_is_down_with_install_and_clears_when_one_exists() {
    let (h, _) = health();
    h.browser_missing("no Chromium-based browser was found");
    let s = h.registry.get(BROWSER).unwrap();
    assert_eq!((s.state, s.repair), (ServiceState::Down, Some(RepairAction::Install)));
    assert!(s.cause.unwrap().detail.unwrap().contains("no Chromium"));
    h.browser_available();
    assert_eq!(h.state(BROWSER), Some(ServiceState::Ok));
}

#[test]
fn a_failed_download_is_down_with_retry_and_a_later_success_clears_it() {
    let (h, seen) = health();
    h.download_failed(7, "Download Qwen", "the download server answered 503");
    h.download_failed(7, "Download Qwen", "the download server answered 503");
    let s = h.registry.get(DOWNLOADS).unwrap();
    assert_eq!((s.state, s.repair), (ServiceState::Down, Some(RepairAction::Retry)));
    assert_eq!(h.failed_download(), Some(7));
    assert_eq!(count(&seen, DOWNLOADS, ServiceState::Down), 1);
    h.download_done(8);
    assert_eq!(h.state(DOWNLOADS), Some(ServiceState::Ok));
    assert_eq!(h.failed_download(), None);
}

#[test]
fn a_download_reported_as_another_service_is_not_a_downloads_failure() {
    let (h, _) = health();
    h.ignore_download_job(3);
    h.download_failed(3, "Install Chromium", "x");
    assert_eq!(h.state(DOWNLOADS), Some(ServiceState::Ok));
}

#[test]
fn the_store_probe_follows_the_supervisors_state() {
    use ServiceState::*;
    assert_eq!(probe_store(true, false, Down, Some(StoreFailure::Crashed)), StoreProbe::Running);
    assert_eq!(probe_store(true, false, Ok, None), StoreProbe::Nothing);
    assert_eq!(probe_store(false, true, Down, None), StoreProbe::Quarantined);
    assert_eq!(probe_store(false, true, Quarantined, None), StoreProbe::Nothing);
    assert_eq!(probe_store(false, false, Down, Some(StoreFailure::Crashed)), StoreProbe::Restart);
    assert_eq!(
        probe_store(false, false, Down, Some(StoreFailure::LibraryMissing)),
        StoreProbe::Nothing
    );
    assert_eq!(
        probe_store(false, false, Ok, None),
        StoreProbe::Nothing,
        "stopped and fine: it starts on demand"
    );
}

#[test]
fn code_without_a_handle_reports_to_the_threads_health() {
    let (h, _) = health();
    use_in_this_thread(h.clone());
    report_store_failed(StoreFailure::Other, "x");
    report_browser_missing("none");
    report_plugin_quarantined("p", "boom");
    report_plugin_reset("p");
    report_browser_available();
    assert_eq!(h.state(STORE), Some(ServiceState::Down));
    assert_eq!(
        (h.state(BROWSER), h.state(PLUGINS)),
        (Some(ServiceState::Ok), Some(ServiceState::Ok))
    );
}
