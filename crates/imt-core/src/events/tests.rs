use super::*;
use std::time::{Duration, Instant};

fn wait_until(what: &str, f: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[derive(Debug)]
struct Custom(u32);

#[test]
fn delivers_in_order_per_subscriber_and_by_type() {
    let bus = EventBus::new(Arc::new(Health::default()));
    assert!(!bus.has_subscribers::<Custom>());
    bus.publish(Custom(0)); // nobody listens: nothing happens
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = seen.clone();
    let id = bus.subscribe::<Custom>("p", move |e| s.lock().unwrap().push(e.0)).unwrap();
    let other = Arc::new(AtomicU64::new(0));
    let o = other.clone();
    bus.subscribe::<WorkspaceCleared>("q", move |_| {
        o.fetch_add(1, Ordering::SeqCst);
    })
    .unwrap();
    assert!(bus.has_subscribers::<Custom>());
    for i in 1..=100 {
        bus.publish(Custom(i));
    }
    bus.publish(WorkspaceCleared);
    wait_until("delivery", || {
        seen.lock().unwrap().len() == 100 && other.load(Ordering::SeqCst) == 1
    });
    assert_eq!(*seen.lock().unwrap(), (1..=100).collect::<Vec<_>>());
    let m = bus.metrics();
    let mine = m.iter().find(|m| m.plugin == "p").unwrap();
    assert_eq!((mine.delivered, mine.dropped, mine.failures, mine.queued), (100, 0, 0, 0));
    assert!(mine.event.ends_with("Custom"));
    bus.unsubscribe(id);
    bus.unsubscribe(SubscriptionId(999));
    bus.publish(Custom(1));
    assert_eq!(bus.metrics().len(), 1);
    bus.unsubscribe_plugin("q");
    assert!(bus.metrics().is_empty());
}

#[test]
fn a_slow_subscriber_drops_the_oldest_and_never_blocks_publish() {
    let bus = EventBus::with_capacity(Arc::new(Health::default()), 8);
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (g, s) = (gate.clone(), seen.clone());
    bus.subscribe::<Custom>("slow", move |e| {
        let (open, cv) = &*g;
        let mut open = open.lock().unwrap();
        while !*open {
            open = cv.wait(open).unwrap();
        }
        s.lock().unwrap().push(e.0);
    })
    .unwrap();
    bus.publish(Custom(0));
    wait_until("the first event to be taken", || bus.metrics()[0].queued == 0);
    let start = Instant::now();
    for i in 1..=1000 {
        bus.publish(Custom(i));
    }
    assert!(start.elapsed() < Duration::from_secs(1), "publish blocked: {:?}", start.elapsed());
    let m = &bus.metrics()[0];
    assert_eq!((m.queued, m.dropped), (8, 992));
    {
        let (open, cv) = &*gate;
        *open.lock().unwrap() = true;
        cv.notify_all();
    }
    wait_until("the queue to drain", || seen.lock().unwrap().len() == 9);
    // The newest events survive, in order.
    assert_eq!(*seen.lock().unwrap(), vec![0, 993, 994, 995, 996, 997, 998, 999, 1000]);
}

#[test]
fn a_panicking_subscriber_is_quarantined_and_then_skipped() {
    let health = Arc::new(Health::default());
    let bus = EventBus::new(health.clone());
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    bus.subscribe::<Custom>("bad", |_| panic!("subscriber exploded")).unwrap();
    for i in 0..5 {
        bus.publish(Custom(i));
    }
    wait_until("all five handled", || {
        let m = &bus.metrics()[0];
        m.failures + m.skipped == 5
    });
    std::panic::set_hook(prev);
    let m = &bus.metrics()[0];
    assert_eq!((m.failures, m.skipped, m.delivered), (3, 2, 0));
    assert!(health.is_quarantined("bad"));
    assert!(health.get("bad").last_error.unwrap().contains("subscriber exploded"));
}

#[test]
fn dropping_the_bus_stops_the_workers() {
    let bus = EventBus::new(Arc::new(Health::default()));
    let seen = Arc::new(AtomicU64::new(0));
    let s = seen.clone();
    bus.subscribe::<Custom>("p", move |_| {
        s.fetch_add(1, Ordering::SeqCst);
    })
    .unwrap();
    let sub = bus.subscribers.read().unwrap()[0].clone();
    drop(bus);
    assert!(sub.lock().closed);
    sub.push(Custom(1).into_payload());
    assert_eq!(sub.lock().items.len(), 0, "a closed queue takes nothing");
    assert!(sub.next().is_none());
}

trait IntoPayload {
    fn into_payload(self) -> Payload;
}
impl IntoPayload for Custom {
    fn into_payload(self) -> Payload {
        Arc::new(self)
    }
}

#[test]
fn builtin_events_are_plain_values() {
    let e = RecordsAppended {
        count: 2,
        sources: vec![1],
    };
    assert_eq!(e.clone(), e);
    assert_eq!(ProfileChanged { name: "x".into() }.name, "x");
    assert_eq!(SnapshotTaken { id: "s".into() }.id, "s");
    let p = PluginStateChanged {
        id: "p".into(),
        status: PluginStatus::Active,
    };
    assert!(format!("{p:?}").contains("Active"));
    assert_eq!(WorkspaceCleared, WorkspaceCleared);
}
