use super::*;
use crate::eventlog::bookmark::BookmarkStore;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn xml(channel: &str, id: u64, provider: &str, event_id: u32, level: u8) -> String {
    format!(
        "<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System><Provider Name='{provider}'/>\
         <EventID>{event_id}</EventID><Level>{level}</Level><TimeCreated SystemTime='2026-10-17T10:00:{:02}.000Z'/>\
         <EventRecordID>{id}</EventRecordID><Channel>{channel}</Channel><Computer>pc1</Computer></System>\
         <EventData><Data Name='n'>{id}</Data></EventData></Event>",
        id % 60
    )
}

#[derive(Default)]
struct Log {
    /// Channel name to its stored events (record id, xml).
    events: BTreeMap<String, Vec<(u64, String)>>,
    probe_errors: BTreeMap<String, EventLogError>,
    /// Errors the next subscriptions fail with, in order.
    subscribe_errors: Vec<EventLogError>,
    /// After this many events a live stream fails once with `Stale`.
    stale_after: Option<usize>,
    /// Events written "while the backfill query ran".
    during_backfill: Vec<(String, u64)>,
    calls: Vec<String>,
    panic_in_stream: bool,
    /// Streams sleep this long instead of honouring the wait.
    hang: Option<Duration>,
}

#[derive(Clone, Default)]
struct FakeLog(Arc<Mutex<Log>>);

impl FakeLog {
    fn with(channels: &[(&str, u64)]) -> FakeLog {
        let f = FakeLog::default();
        for (c, n) in channels {
            for _ in 0..*n {
                f.append(c);
            }
        }
        f
    }

    fn lock(&self) -> MutexGuard<'_, Log> {
        self.0.lock().unwrap()
    }

    fn append(&self, channel: &str) -> u64 {
        let mut l = self.lock();
        let list = l.events.entry(channel.into()).or_default();
        let id = list.last().map_or(1, |(i, _)| i + 1);
        list.push((id, xml(channel, id, "Contoso", 7036, 4)));
        id
    }

    fn push_raw(&self, channel: &str, id: u64, text: &str) {
        self.lock().events.entry(channel.into()).or_default().push((id, text.into()));
    }

    fn calls(&self) -> Vec<String> {
        self.lock().calls.clone()
    }
}

struct Stream {
    log: FakeLog,
    channel: String,
    /// Next index into the channel's events.
    pos: usize,
    /// Query: stop at this index.
    end: Option<usize>,
    delivered: usize,
    stale_after: Option<usize>,
}

impl EventStream for Stream {
    fn next_batch(&mut self, max: usize, wait: Duration) -> Result<Batch, EventLogError> {
        let (panic, hang) = {
            let l = self.log.lock();
            (l.panic_in_stream, l.hang)
        };
        if panic {
            panic!("event log adapter bug");
        }
        if let Some(h) = hang {
            std::thread::sleep(h);
        }
        if self.stale_after.is_some_and(|n| self.delivered >= n) {
            self.stale_after = None;
            self.log.lock().stale_after = None;
            return Err(EventLogError::Stale {
                channel: self.channel.clone(),
            });
        }
        let mut l = self.log.lock();
        let all = l.events.get(&self.channel).cloned().unwrap_or_default();
        let end = self.end.unwrap_or(all.len()).min(all.len());
        if self.pos >= end {
            if self.end.is_some() {
                // The query is done; events written meanwhile go to its channel. Only this channel's: the channels'
                // queries run in any order, and another channel finishing first must not write them early.
                let (during, rest) = std::mem::take(&mut l.during_backfill)
                    .into_iter()
                    .partition(|(c, _)| *c == self.channel);
                l.during_backfill = rest;
                drop(l);
                for (c, n) in during {
                    for _ in 0..n {
                        self.log.append(&c);
                    }
                }
                return Ok(Batch::End);
            }
            drop(l);
            std::thread::sleep(wait.min(Duration::from_millis(5)));
            return Ok(Batch::Idle);
        }
        let take = (end - self.pos)
            .min(max)
            .min(self.stale_after.map_or(usize::MAX, |n| n - self.delivered));
        let events = all[self.pos..self.pos + take]
            .iter()
            .map(|(_, x)| RawEvent {
                xml: x.clone(),
                message: Some(format!("formatted {}", x.len())),
            })
            .collect();
        self.pos += take;
        self.delivered += take;
        Ok(Batch::Events(events))
    }
}

impl EventLog for FakeLog {
    fn channel_names(&self) -> Result<Vec<String>, EventLogError> {
        Ok(self.lock().events.keys().cloned().collect())
    }

    fn probe(&self, channel: &Channel, xpath: &str) -> Result<Option<u64>, EventLogError> {
        let mut l = self.lock();
        l.calls.push(format!("probe {channel} {xpath}"));
        if let Some(e) = l.probe_errors.get(channel.as_str()) {
            return Err(e.clone());
        }
        Ok(l.events.get(channel.as_str()).and_then(|e| e.last()).map(|(i, _)| *i))
    }

    fn query(&self, channel: &Channel, xpath: &str) -> Result<Box<dyn EventStream>, EventLogError> {
        let mut l = self.lock();
        l.calls.push(format!("query {channel} {xpath}"));
        let end = l.events.get(channel.as_str()).map_or(0, Vec::len);
        Ok(Box::new(Stream {
            log: self.clone(),
            channel: channel.to_string(),
            pos: 0,
            end: Some(end),
            delivered: 0,
            stale_after: None,
        }))
    }

    fn subscribe(&self, channel: &Channel, xpath: &str, after: Option<u64>) -> Result<Box<dyn EventStream>, EventLogError> {
        let mut l = self.lock();
        l.calls.push(format!("subscribe {channel} {xpath} after {after:?}"));
        if !l.subscribe_errors.is_empty() {
            return Err(l.subscribe_errors.remove(0));
        }
        let events = l.events.get(channel.as_str()).cloned().unwrap_or_default();
        let pos = match after {
            Some(a) => events.iter().position(|(i, _)| *i > a).unwrap_or(events.len()),
            None => 0,
        };
        Ok(Box::new(Stream {
            log: self.clone(),
            channel: channel.to_string(),
            pos,
            end: None,
            delivered: 0,
            stale_after: l.stale_after,
        }))
    }
}

#[derive(Default)]
struct MemSink {
    records: Mutex<Vec<Record>>,
    fail: AtomicBool,
    slow: Option<Duration>,
}

impl EventSink for MemSink {
    fn write(&self, records: &[Record]) -> Result<()> {
        if self.fail.load(Ordering::SeqCst) {
            bail!("disk full");
        }
        if let Some(d) = self.slow {
            std::thread::sleep(d);
        }
        self.records.lock().unwrap().extend_from_slice(records);
        Ok(())
    }
}

impl MemSink {
    fn ids(&self, channel: &str) -> Vec<u64> {
        self.records
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.fields["Channel"] == channel)
            .map(|r| r.fields["RecordId"].as_u64().unwrap())
            .collect()
    }
}

fn limits() -> Limits {
    Limits {
        queue: 64,
        batch: 8,
        wait: Duration::from_millis(10),
        retry: Duration::from_millis(20),
        save_every: Duration::from_millis(10),
        stop_timeout: Duration::from_secs(5),
    }
}

fn config(v: Value) -> FollowerConfig {
    FollowerConfig::from_json(&v).unwrap()
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn start(log: &FakeLog, cfg: FollowerConfig, sink: &Arc<MemSink>, store: Option<BookmarkStore>) -> Result<Follower> {
    Follower::start(Arc::new(log.clone()), cfg, sink.clone(), store, limits(), Box::new(|| {}))
}

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("imt-follower-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

#[test]
fn config_from_json() {
    let c = config(json!({
        "channels": ["System", "system", "Microsoft-Windows-TaskScheduler/Operational"],
        "levels": ["error", "critical"],
        "xpath": "*[System[EventID=7036]]",
        "backfill": "24h",
        "resumeFromBookmark": false
    }));
    assert_eq!(c.channels.len(), 2, "duplicates are merged");
    assert!(!c.resume);
    assert_eq!(c.live_xpath(), "*[System[(Level=1 or Level=2)] and (System[EventID=7036])]");
    assert_eq!(
        c.backfill_xpath(),
        "*[System[(Level=1 or Level=2)] and System[TimeCreated[timediff(@SystemTime) <= 86400000]] and (System[EventID=7036])]"
    );
    let c = config(json!({ "channels": ["Application"], "levels": null, "xpath": null, "backfill": null }));
    assert!(c.resume);
    assert_eq!((c.live_xpath().as_str(), c.backfill), ("*", Backfill::None));
    for (bad, why) in [
        (json!([]), "must be an object"),
        (json!({}), "at least one"),
        (json!({ "channels": [] }), "at least one"),
        (json!({ "channels": [1] }), "must be text"),
        (json!({ "channels": ["Nope"] }), "not a channel"),
        (json!({ "channels": ["System"], "levels": "error" }), "must be a list"),
        (json!({ "channels": ["System"], "levels": [3] }), "not a level"),
        (json!({ "channels": ["System"], "xpath": 1 }), "must be text"),
        (json!({ "channels": ["System"], "xpath": "System[" }), "unclosed"),
        (json!({ "channels": ["System"], "backfill": 1 }), "must be text"),
        (json!({ "channels": ["System"], "backfill": "1y" }), "not a backfill"),
    ] {
        let e = FollowerConfig::from_json(&bad).unwrap_err().to_string();
        assert!(e.contains(why), "{bad}: {e}");
    }
    let many: Vec<String> = (0..=MAX_CHANNELS).map(|i| format!("Microsoft-Windows-C{i}/Operational")).collect();
    let e = FollowerConfig::from_json(&json!({ "channels": many })).unwrap_err().to_string();
    assert!(e.contains("at most 32"), "{e}");
}

#[test]
fn backfill_then_live_without_gaps_or_duplicates() {
    let log = FakeLog::with(&[("System", 5), ("Application", 2)]);
    log.lock().during_backfill = vec![("System".into(), 2)];
    let sink = Arc::new(MemSink::default());
    let f = start(
        &log,
        config(json!({ "channels": ["System", "Application"], "backfill": "1h" })),
        &sink,
        None,
    )
    .unwrap();
    wait_until("backfill and the events written meanwhile", || sink.ids("System").len() == 7);
    log.append("System");
    log.append("Application");
    wait_until("live events", || {
        sink.ids("System").len() == 8 && sink.ids("Application").len() == 3
    });
    let status = f.status();
    assert!(status.running && !status.paused);
    assert_eq!(status.events, 11);
    assert!(
        status.channels.iter().all(|c| c.state == ChannelState::Live),
        "{:?}",
        status.channels
    );
    assert_eq!(status.channels[0].last_record_id, Some(8));
    assert_eq!(status.queue_capacity, 64);
    assert_eq!(status.xpath, "*");
    let last = status.last_event.unwrap();
    assert_eq!(
        (last.provider.as_str(), last.event_id.clone(), last.level),
        ("Contoso", json!(7036), Level::Information)
    );
    f.stop().unwrap();
    assert_eq!(sink.ids("System"), (1..=8).collect::<Vec<_>>());
    assert_eq!(sink.ids("Application"), [1, 2, 3]);
    // The formatted message is the record's message.
    assert!(sink.records.lock().unwrap()[0].message.starts_with("formatted "));
    let calls = log.calls();
    assert!(
        calls.contains(&"probe System *[System[TimeCreated[timediff(@SystemTime) <= 3600000]]]".to_string()),
        "{calls:?}"
    );
    assert!(calls.contains(&"subscribe System * after Some(5)".to_string()), "{calls:?}");
}

#[test]
fn without_backfill_only_new_events_and_an_empty_log_starts_at_its_oldest() {
    let log = FakeLog::with(&[("System", 3)]);
    let sink = Arc::new(MemSink::default());
    let f = start(&log, config(json!({ "channels": ["System", "Setup"] })), &sink, None).unwrap();
    wait_until("subscriptions", || {
        log.calls().iter().filter(|c| c.starts_with("subscribe")).count() == 2
    });
    log.append("System");
    log.append("Setup");
    wait_until("new events", || sink.ids("System") == [4] && sink.ids("Setup") == [1]);
    f.stop().unwrap();
    let calls = log.calls();
    assert!(calls.contains(&"subscribe System * after Some(3)".to_string()), "{calls:?}");
    assert!(calls.contains(&"subscribe Setup * after None".to_string()), "{calls:?}");
}

#[test]
fn restarts_resume_after_the_saved_bookmark() {
    let dir = temp("resume");
    let path = dir.join("bookmarks.json");
    let log = FakeLog::with(&[("System", 3)]);
    let sink = Arc::new(MemSink::default());
    let cfg = config(json!({ "channels": ["System"], "backfill": "7d" }));
    let f = start(&log, cfg.clone(), &sink, Some(BookmarkStore::load(&path))).unwrap();
    wait_until("backfill", || sink.ids("System").len() == 3);
    f.stop().unwrap();
    let system = Channel::parse("System").unwrap();
    assert_eq!(BookmarkStore::load(&path).get(&system), Some(3));
    // Written while IMT was closed.
    log.append("System");
    log.append("System");
    let f = start(&log, cfg.clone(), &sink, Some(BookmarkStore::load(&path))).unwrap();
    assert_eq!(f.status().channels[0].last_record_id, Some(3));
    wait_until("the missed events", || sink.ids("System").len() == 5);
    f.stop().unwrap();
    assert_eq!(sink.ids("System"), [1, 2, 3, 4, 5], "no gap, no duplicate, no second backfill");
    assert_eq!(BookmarkStore::load(&path).get(&system), Some(5));
    assert_eq!(log.calls().iter().filter(|c| c.starts_with("query")).count(), 1);
    // resume: false ignores the bookmark (and backfills again).
    let mut fresh = cfg;
    fresh.resume = false;
    let f = start(&log, fresh, &sink, Some(BookmarkStore::load(&path))).unwrap();
    wait_until("a second backfill", || sink.ids("System").len() == 10);
    f.stop().unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn access_denied_fails_the_start_with_the_rights_error() {
    let log = FakeLog::with(&[("System", 1)]);
    log.lock().probe_errors.insert(
        "Security".into(),
        EventLogError::AccessDenied {
            channel: "Security".into(),
        },
    );
    let sink = Arc::new(MemSink::default());
    let e = start(&log, config(json!({ "channels": ["System", "Security"] })), &sink, None)
        .err()
        .unwrap()
        .to_string();
    assert!(
        e.contains("Access is denied to the Security event log") && e.contains("administrator"),
        "{e}"
    );
    assert!(!log.calls().iter().any(|c| c.starts_with("subscribe")), "nothing started");
    let mut l = log.lock();
    l.probe_errors.insert(
        "System".into(),
        EventLogError::InvalidQuery {
            channel: "System".into(),
            detail: "bad".into(),
        },
    );
    drop(l);
    let e = start(&log, config(json!({ "channels": ["System"] })), &sink, None).err().unwrap();
    assert!(e.to_string().contains("Check the XPath filter"), "{e}");
}

#[test]
fn failed_subscriptions_are_retried_after_the_last_event() {
    let log = FakeLog::with(&[("System", 2)]);
    {
        let mut l = log.lock();
        l.subscribe_errors = vec![EventLogError::Other {
            channel: "System".into(),
            code: 1726,
            message: "rpc failed".into(),
        }];
        l.stale_after = Some(2);
    }
    let sink = Arc::new(MemSink::default());
    let f = start(&log, config(json!({ "channels": ["System"], "backfill": "1h" })), &sink, None).unwrap();
    wait_until("the backfill", || sink.ids("System").len() == 2);
    wait_until("the first error", || !f.status().errors.is_empty());
    for _ in 0..4 {
        log.append("System");
    }
    wait_until("events after the retries", || sink.ids("System").len() == 6);
    let errors: Vec<String> = f.status().errors.into_iter().map(|e| e.message).collect();
    assert!(errors.iter().any(|e| e.contains("rpc failed")), "{errors:?}");
    assert!(errors.iter().any(|e| e.contains("cleared")), "{errors:?}");
    f.stop().unwrap();
    assert_eq!(sink.ids("System"), [1, 2, 3, 4, 5, 6]);
    let calls = log.calls();
    let subs: Vec<&String> = calls.iter().filter(|c| c.starts_with("subscribe")).collect();
    assert_eq!(subs[0], "subscribe System * after Some(2)");
    assert!(
        subs.iter().any(|c| c.ends_with("after Some(4)")),
        "stale stream reopened after the last event: {subs:?}"
    );
}

#[test]
fn retrying_channels_report_their_error() {
    let log = FakeLog::with(&[("System", 1)]);
    log.lock().subscribe_errors = vec![
        EventLogError::Unavailable("service stopped".into()),
        EventLogError::Unavailable("service stopped".into()),
    ];
    let sink = Arc::new(MemSink::default());
    let mut lim = limits();
    lim.retry = Duration::from_millis(300);
    let f = Follower::start(
        Arc::new(log.clone()),
        config(json!({ "channels": ["System"] })),
        sink.clone(),
        None,
        lim,
        Box::new(|| {}),
    )
    .unwrap();
    wait_until("retrying", || f.status().channels[0].state == ChannelState::Retrying);
    let s = f.status();
    assert!(s.channels[0].error.as_deref().unwrap().contains("service stopped"));
    assert_eq!(s.errors[0].channel.as_deref(), Some("System"));
    wait_until("live again", || f.status().channels[0].state == ChannelState::Live);
    assert_eq!(f.status().channels[0].error, None);
    f.stop().unwrap();
}

#[test]
fn pause_holds_events_until_resumed() {
    let log = FakeLog::with(&[("System", 0)]);
    let sink = Arc::new(MemSink::default());
    let f = start(&log, config(json!({ "channels": ["System"] })), &sink, None).unwrap();
    wait_until("live", || f.status().channels[0].state == ChannelState::Live);
    f.pause();
    assert!(f.status().paused);
    std::thread::sleep(Duration::from_millis(50));
    log.append("System");
    std::thread::sleep(Duration::from_millis(100));
    assert!(sink.ids("System").is_empty(), "nothing read while paused");
    f.resume();
    wait_until("the held event", || sink.ids("System") == [1]);
    f.stop().unwrap();
}

#[test]
fn a_failing_sink_ends_the_run_and_reports_it() {
    let log = FakeLog::with(&[("System", 0)]);
    let sink = Arc::new(MemSink::default());
    sink.fail.store(true, Ordering::SeqCst);
    let ended = Arc::new(AtomicBool::new(false));
    let flag = ended.clone();
    let f = Follower::start(
        Arc::new(log.clone()),
        config(json!({ "channels": ["System"] })),
        sink.clone(),
        None,
        limits(),
        Box::new(move || flag.store(true, Ordering::SeqCst)),
    )
    .unwrap();
    wait_until("live", || f.status().channels[0].state == ChannelState::Live);
    log.append("System");
    wait_until("the end", || !f.is_running());
    assert!(ended.load(Ordering::SeqCst), "on_end ran");
    let s = f.status();
    assert!(!s.running);
    assert!(s.errors[0].message.contains("disk full"), "{:?}", s.errors);
    let e = f.stop().unwrap_err().to_string();
    assert!(e.contains("writing the event log files failed: disk full"), "{e}");
}

#[test]
fn unreadable_events_are_skipped_and_counted() {
    let log = FakeLog::with(&[("System", 1)]);
    log.push_raw("System", 2, "<Event><System>");
    log.push_raw("System", 3, &xml("System", 3, "Contoso", 1, 2));
    let sink = Arc::new(MemSink::default());
    let f = start(&log, config(json!({ "channels": ["System"], "backfill": "1h" })), &sink, None).unwrap();
    wait_until("the readable events", || sink.ids("System") == [1, 3]);
    let s = f.status();
    assert_eq!(s.unreadable, 1);
    assert!(s.errors.iter().any(|e| e.message.contains("unreadable event")), "{:?}", s.errors);
    assert_eq!(sink.records.lock().unwrap()[1].level, Level::Error);
    f.stop().unwrap();
}

#[test]
fn a_crashing_adapter_is_contained() {
    let log = FakeLog::with(&[("System", 1)]);
    log.lock().panic_in_stream = true;
    let sink = Arc::new(MemSink::default());
    let f = start(&log, config(json!({ "channels": ["System"] })), &sink, None).unwrap();
    wait_until("the crash report", || f.status().channels[0].state == ChannelState::Stopped);
    let s = f.status();
    assert!(s.errors[0].message.contains("crashed: event log adapter bug"), "{:?}", s.errors);
    // With every reader gone the run ends on its own (and says why).
    wait_until("the end of the run", || !f.is_running());
    f.stop().unwrap();
}

#[test]
fn a_full_queue_slows_readers_without_losing_events() {
    let log = FakeLog::with(&[("System", 200)]);
    let sink = Arc::new(MemSink {
        slow: Some(Duration::from_millis(2)),
        ..MemSink::default()
    });
    let mut lim = limits();
    lim.queue = 4;
    lim.batch = 3;
    let f = Follower::start(
        Arc::new(log.clone()),
        config(json!({ "channels": ["System"], "backfill": "24h" })),
        sink.clone(),
        None,
        lim,
        Box::new(|| {}),
    )
    .unwrap();
    let mut max_queued = 0;
    wait_until("every event", || {
        max_queued = max_queued.max(f.status().queued);
        sink.ids("System").len() == 200
    });
    assert!(max_queued <= 4, "{max_queued}");
    assert!(f.status().events_per_second >= 0.0);
    f.stop().unwrap();
    assert_eq!(sink.ids("System"), (1..=200).collect::<Vec<_>>());
}

#[test]
fn stop_is_bounded_even_when_the_event_log_hangs() {
    let log = FakeLog::with(&[("System", 1)]);
    let sink = Arc::new(MemSink::default());
    let mut lim = limits();
    lim.stop_timeout = Duration::from_millis(100);
    let f = Follower::start(
        Arc::new(log.clone()),
        config(json!({ "channels": ["System"] })),
        sink.clone(),
        None,
        lim,
        Box::new(|| {}),
    )
    .unwrap();
    wait_until("live", || f.status().channels[0].state == ChannelState::Live);
    log.lock().hang = Some(Duration::from_secs(2));
    std::thread::sleep(Duration::from_millis(50));
    let t = Instant::now();
    let e = f.stop().unwrap_err().to_string();
    assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
    assert!(e.contains("did not stop within"), "{e}");
}

#[test]
fn a_failed_backfill_continues_live_and_rates_are_sampled() {
    let log = FakeLog::with(&[("System", 2)]);
    log.lock().stale_after = None;
    // The backfill query itself fails: the follower continues live after the seed.
    struct NoQuery(FakeLog);
    impl EventLog for NoQuery {
        fn channel_names(&self) -> Result<Vec<String>, EventLogError> {
            self.0.channel_names()
        }
        fn probe(&self, c: &Channel, x: &str) -> Result<Option<u64>, EventLogError> {
            self.0.probe(c, x)
        }
        fn query(&self, c: &Channel, _: &str) -> Result<Box<dyn EventStream>, EventLogError> {
            Err(EventLogError::Stale { channel: c.to_string() })
        }
        fn subscribe(&self, c: &Channel, x: &str, a: Option<u64>) -> Result<Box<dyn EventStream>, EventLogError> {
            self.0.subscribe(c, x, a)
        }
    }
    let sink = Arc::new(MemSink::default());
    let f = Follower::start(
        Arc::new(NoQuery(log.clone())),
        config(json!({ "channels": ["System"], "backfill": "1h" })),
        sink.clone(),
        None,
        limits(),
        Box::new(|| {}),
    )
    .unwrap();
    wait_until("live", || f.status().channels[0].state == ChannelState::Live);
    assert!(f.status().errors[0].message.contains("cleared"));
    log.append("System");
    wait_until("the live event", || sink.ids("System") == [3]);
    f.stop().unwrap();

    let mut s = Stats::default();
    let t0 = Instant::now();
    assert_eq!(s.rate(), 0.0);
    s.sample(t0);
    s.events = 50;
    s.sample(t0 + Duration::from_millis(100));
    assert_eq!(s.samples.len(), 1, "samples are at least half a second apart");
    s.sample(t0 + Duration::from_secs(5));
    assert_eq!(s.rate(), 10.0);
    s.sample(t0 + Duration::from_secs(20));
    assert_eq!(s.samples.len(), 1, "old samples are dropped");
    for i in 0..30 {
        s.error(None, &format!("e{i}"));
    }
    assert_eq!(s.errors.len(), MAX_ERRORS);
    assert_eq!(s.errors[0].message, "e29");
}
