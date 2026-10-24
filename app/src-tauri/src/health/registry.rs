//! The registry: one entry per service, updated by that service's supervisor, announcing every real change once.
//! Pure: no clock, no threads, no I/O; the sink (an event to every window, a notification) is passed in.

use super::model::{Cause, Change, RepairAction, ServiceHealth, ServiceState, Snapshot};
use std::sync::Mutex;

/// Told after each change, outside the registry's lock.
pub type Sink = Box<dyn Fn(&Snapshot, &Change) + Send + Sync>;

struct Inner {
    revision: u64,
    services: Vec<ServiceHealth>,
}

pub struct Registry {
    inner: Mutex<Inner>,
    sink: Sink,
}

impl Registry {
    /// A registry of `services` (id, label), all `ok` (or `starting` for `starting`), in this order.
    pub fn new(services: &[(&str, &str)], sink: Sink) -> Registry {
        Registry {
            inner: Mutex::new(Inner {
                revision: 0,
                services: services
                    .iter()
                    .map(|(id, label)| ServiceHealth {
                        service: (*id).into(),
                        label: (*label).into(),
                        state: ServiceState::Ok,
                        cause: None,
                        repair: None,
                    })
                    .collect(),
            }),
            sink,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn snapshot(&self) -> Snapshot {
        let g = self.lock();
        Snapshot {
            revision: g.revision,
            services: g.services.clone(),
        }
    }

    pub fn get(&self, service: &str) -> Option<ServiceHealth> {
        self.lock().services.iter().find(|s| s.service == service).cloned()
    }

    /// Sets a service's state. Saying the same thing again changes nothing and announces nothing (that is the dedupe);
    /// an unknown service is ignored. Returns the change, if there was one.
    pub fn set(&self, service: &str, state: ServiceState, cause: Option<Cause>, repair: Option<RepairAction>) -> Option<Change> {
        let (snapshot, change) = {
            let mut g = self.lock();
            let revision = g.revision + 1;
            let cause = if state == ServiceState::Ok { None } else { cause };
            let repair = if state == ServiceState::Ok { None } else { repair };
            let entry = g.services.iter_mut().find(|s| s.service == service)?;
            if entry.state == state && entry.cause == cause && entry.repair == repair {
                return None;
            }
            let change = Change {
                service: service.into(),
                from: entry.state,
                to: state,
            };
            entry.state = state;
            entry.cause = cause;
            entry.repair = repair;
            g.revision = revision;
            (
                Snapshot {
                    revision,
                    services: g.services.clone(),
                },
                change,
            )
        };
        (self.sink)(&snapshot, &change);
        Some(change)
    }

    /// The service works.
    pub fn ok(&self, service: &str) -> Option<Change> {
        self.set(service, ServiceState::Ok, None, None)
    }

    /// The service is not working, with a cause and what repairs it.
    pub fn down(&self, service: &str, cause: Cause, repair: Option<RepairAction>) -> Option<Change> {
        self.set(service, ServiceState::Down, Some(cause), repair)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    type Seen = Arc<Mutex<Vec<(u64, Change)>>>;

    fn registry() -> (Registry, Seen) {
        let seen: Seen = Arc::default();
        let s = seen.clone();
        let r = Registry::new(
            &[("store", "the log store"), ("ai", "the AI worker")],
            Box::new(move |snap, c| s.lock().unwrap().push((snap.revision, c.clone()))),
        );
        (r, seen)
    }

    #[test]
    fn every_service_starts_ok_in_the_given_order() {
        let (r, _) = registry();
        let s = r.snapshot();
        assert_eq!(s.revision, 0);
        assert_eq!(s.services.iter().map(|s| s.service.as_str()).collect::<Vec<_>>(), ["store", "ai"]);
        assert!(s.services.iter().all(|s| s.state == ServiceState::Ok && s.cause.is_none()));
    }

    #[test]
    fn the_same_failure_three_times_is_announced_once() {
        let (r, seen) = registry();
        for _ in 0..3 {
            r.down("store", Cause::new("The log store stopped."), Some(RepairAction::Restart));
        }
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert_eq!(r.snapshot().revision, 1);
        let h = r.get("store").unwrap();
        assert_eq!((h.state, h.repair), (ServiceState::Down, Some(RepairAction::Restart)));
    }

    #[test]
    fn a_new_cause_updates_in_place_and_ok_clears_the_cause() {
        let (r, seen) = registry();
        r.down("store", Cause::new("a"), Some(RepairAction::Restart));
        r.down("store", Cause::new("b").hint("do x"), Some(RepairAction::Restart));
        assert_eq!(r.get("store").unwrap().cause.unwrap().message, "b");
        let c = r.ok("store").unwrap();
        assert_eq!((c.from, c.to), (ServiceState::Down, ServiceState::Ok));
        assert!(r.get("store").unwrap().cause.is_none());
        assert_eq!(seen.lock().unwrap().len(), 3);
        assert!(r.ok("store").is_none(), "already ok");
    }

    #[test]
    fn an_unknown_service_is_ignored_and_ok_ignores_a_stray_cause() {
        let (r, seen) = registry();
        assert!(r.down("nope", Cause::new("x"), None).is_none());
        assert!(r
            .set("ai", ServiceState::Ok, Some(Cause::new("stale")), Some(RepairAction::Retry))
            .is_none());
        assert!(seen.lock().unwrap().is_empty());
    }

    #[test]
    fn states_are_problems_or_not() {
        use ServiceState::*;
        assert!([Ok, Starting].iter().all(|s| s.is_ok()));
        assert!([Degraded, Down, Quarantined, Repairing].iter().all(|s| s.is_problem()));
    }

    #[test]
    fn the_wire_shape_is_camel_case() {
        let (r, _) = registry();
        r.set(
            "ai",
            ServiceState::Quarantined,
            Some(Cause::new("m").detail(" ")),
            Some(RepairAction::LiftQuarantine),
        );
        let v = serde_json::to_value(r.snapshot()).unwrap();
        assert_eq!(v["services"][1]["state"], "quarantined");
        assert_eq!(v["services"][1]["repair"], "liftQuarantine");
        assert_eq!(v["services"][1]["cause"]["detail"], serde_json::Value::Null);
    }
}
