use super::*;
use crate::health::model::{RepairAction, ServiceHealth};
use crate::notification_centre::centre::{Centre, NoticeId};

fn snapshot_with(service: &str, state: ServiceState, cause: Option<Cause>, repair: Option<RepairAction>) -> (Snapshot, Change) {
    let snapshot = Snapshot {
        revision: 1,
        services: vec![ServiceHealth {
            service: service.into(),
            label: "the log store".into(),
            state,
            cause,
            repair,
        }],
    };
    let change = Change {
        service: service.into(),
        from: ServiceState::Ok,
        to: state,
    };
    (snapshot, change)
}

#[test]
fn an_outage_is_one_error_notification_with_the_services_cause_key_and_a_repair_button() {
    let (s, c) = snapshot_with(
        STORE,
        ServiceState::Down,
        Some(Cause::new("Debrief's log store stopped unexpectedly.").hint("Choose Try again.")),
        Some(RepairAction::Restart),
    );
    let n = notice_for(&s, &c).unwrap();
    assert_eq!(
        (n.cause.as_str(), n.severity, n.source),
        ("health:store", NoticeSeverity::Error, NoticeSource::Health)
    );
    assert_eq!(n.title, "Debrief's log store stopped unexpectedly.");
    assert_eq!(n.body, "Choose Try again.");
    assert_eq!(n.primary, Some(NoticeAction::RepairService { service: "store".into() }));
}

#[test]
fn the_same_outage_three_times_is_one_notification_and_ok_resolves_it() {
    let (s, c) = snapshot_with(STORE, ServiceState::Down, Some(Cause::new("stopped")), Some(RepairAction::Restart));
    let mut centre = Centre::new(10);
    for i in 0..3u64 {
        centre.post(notice_for(&s, &c).unwrap(), NoticeId(i + 1), 100 + i as i64);
    }
    assert_eq!(centre.notices().len(), 1, "same cause: updated in place");
    assert_eq!(centre.notices()[0].count, 3);
    assert!(centre.resolve_cause("health:store").is_some());
    assert!(centre.notices().is_empty());
}

#[test]
fn a_repair_in_progress_and_a_recovery_post_nothing() {
    let (s, c) = snapshot_with(
        STORE,
        ServiceState::Repairing,
        Some(Cause::new("stopped")),
        Some(RepairAction::Restart),
    );
    assert!(notice_for(&s, &c).is_none());
    let (s, c) = snapshot_with(STORE, ServiceState::Ok, None, None);
    assert!(notice_for(&s, &c).is_none());
}

#[test]
fn a_degraded_service_is_a_warning_without_a_button_when_nothing_repairs_it() {
    let (s, c) = snapshot_with(AI, ServiceState::Degraded, Some(Cause::new("slow")), None);
    let n = notice_for(&s, &c).unwrap();
    assert_eq!(n.severity, NoticeSeverity::Warning);
    assert!(n.primary.is_none());
}

#[test]
fn the_ai_states_map_to_health() {
    assert_eq!(ai_health(EngineStateDto::Disabled), AiHealth::Switched);
    assert_eq!(ai_health(EngineStateDto::Failed), AiHealth::LoadFailed);
    for ok in [
        EngineStateDto::Ready,
        EngineStateDto::Idle,
        EngineStateDto::Loading,
        EngineStateDto::NoModel,
        EngineStateDto::Unavailable,
    ] {
        assert_eq!(ai_health(ok), AiHealth::Fine);
    }
}

#[test]
fn the_engines_store_error_names_the_cause_and_reports_the_outage_once() {
    let seen = Arc::new(std::sync::Mutex::new(0usize));
    let s = seen.clone();
    let h = Health::new(Box::new(move |_, _| *s.lock().unwrap() += 1));
    hub::use_in_this_thread(h.clone());
    let store = debrief_core::ports::StoreError::Engine("the store worker exited (signal: 6)".into());
    let e: CmdError = crate::engine::store_error(store.clone());
    let again: CmdError = crate::engine::store_error(store);
    assert_eq!(e.cause, Some(crate::error::ServiceCause::Store));
    assert_eq!(serde_json::to_value(&e).unwrap()["cause"], "health:store");
    assert_eq!(e, again);
    assert_eq!(h.state(STORE), Some(ServiceState::Down));
    assert_eq!(*seen.lock().unwrap(), 1);
}
