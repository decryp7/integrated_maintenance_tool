//! The follower engine: one reader thread per channel, a bounded queue and
//! one writer that appends records to an [`EventSink`] (rolling CLEF files
//! in the app) and persists the channel positions.
//!
//! Per channel, the start position is, in order:
//! 1. the persisted bookmark (resume after the last written record),
//! 2. with a backfill window, the stored events of that window (`EvtQuery`),
//!    then live events after the last one read,
//! 3. otherwise live events after the newest record at start.
//!
//! Live events come from a subscription that starts after a bookmark, so
//! events written while the backfill ran are neither missed nor repeated.
//! Positions are saved after the records are written (at least once: a crash
//! between the two can repeat the last batch, never lose it).
//!
//! Every wait is bounded; readers retry after failures (a cleared log, a
//! stopped service) and report them in the status; panics are contained.

use super::bookmark::BookmarkStore;
use super::query::{build_xpath, Backfill, Channel, EventLevel, LevelSet, XPathFilter, MAX_CHANNELS};
use super::xml::record_from_xml;
use super::{Batch, EventLog, EventLogError, EventStream, RawEvent};
use anyhow::{anyhow, bail, Result};
use imt_core::export::EventSink;
use imt_core::model::{Level, Record};
use serde::Serialize;
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Most recent errors kept for the status.
const MAX_ERRORS: usize = 20;
/// Longest error message kept.
const MAX_ERROR_CHARS: usize = 500;

/// What to follow, parsed from the tool's arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowerConfig {
    pub channels: Vec<Channel>,
    pub levels: LevelSet,
    pub filter: Option<XPathFilter>,
    pub backfill: Backfill,
    /// Continue after the persisted bookmarks (default) instead of starting afresh.
    pub resume: bool,
}

impl FollowerConfig {
    /// `{ channels: [..], levels?: [..], xpath?, backfill?: "none"|"1h"|"24h"|"7d", resumeFromBookmark?: bool }`.
    pub fn from_json(v: &Value) -> Result<FollowerConfig> {
        let Some(obj) = v.as_object() else {
            bail!("the event log follower settings must be an object");
        };
        let names = match obj.get("channels") {
            Some(Value::Array(a)) => a,
            _ => bail!("choose at least one event log channel"),
        };
        let mut channels = Vec::new();
        for n in names {
            let c = Channel::parse(n.as_str().ok_or_else(|| anyhow!("channel names must be text"))?)?;
            if !channels.contains(&c) {
                channels.push(c);
            }
        }
        if channels.is_empty() {
            bail!("choose at least one event log channel");
        }
        if channels.len() > MAX_CHANNELS {
            bail!("follow at most {MAX_CHANNELS} channels at once ({} chosen)", channels.len());
        }
        let levels = match obj.get("levels") {
            None | Some(Value::Null) => LevelSet::default(),
            Some(Value::Array(a)) => LevelSet::new(
                a.iter()
                    .map(|l| EventLevel::parse(l.as_str().unwrap_or_default()))
                    .collect::<Result<Vec<_>>>()?,
            ),
            Some(_) => bail!("levels must be a list"),
        };
        let filter = match obj.get("xpath") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => XPathFilter::parse(s)?,
            Some(_) => bail!("the XPath filter must be text"),
        };
        let backfill = match obj.get("backfill") {
            None | Some(Value::Null) => Backfill::None,
            Some(Value::String(s)) => Backfill::parse(s)?,
            Some(_) => bail!("the backfill window must be text"),
        };
        let resume = obj.get("resumeFromBookmark").and_then(Value::as_bool).unwrap_or(true);
        Ok(FollowerConfig {
            channels,
            levels,
            filter,
            backfill,
            resume,
        })
    }

    /// The query for live events (levels and the user filter).
    pub fn live_xpath(&self) -> String {
        build_xpath(&self.levels, Backfill::None, self.filter.as_ref())
    }

    /// The query for the backfill window.
    pub fn backfill_xpath(&self) -> String {
        build_xpath(&self.levels, self.backfill, self.filter.as_ref())
    }
}

/// Bounds and pacing (tests shorten them).
#[derive(Debug, Clone)]
pub struct Limits {
    /// Records waiting between the readers and the writer.
    pub queue: usize,
    /// Most events per read and per write.
    pub batch: usize,
    /// Longest wait for new events before checking for stop/pause.
    pub wait: Duration,
    /// Pause before a failed reader tries again.
    pub retry: Duration,
    /// How often positions are saved while events flow.
    pub save_every: Duration,
    /// How long `stop` waits for the threads.
    pub stop_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            queue: 4096,
            batch: 256,
            wait: Duration::from_millis(250),
            retry: Duration::from_secs(5),
            save_every: Duration::from_secs(1),
            stop_timeout: Duration::from_secs(10),
        }
    }
}

/// What a channel's reader is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChannelState {
    Starting,
    Backfill,
    Live,
    /// Failed; tries again after a pause.
    Retrying,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelStatus {
    pub channel: Channel,
    pub state: ChannelState,
    /// Events written from this channel in this run.
    pub events: u64,
    /// The last written record (the bookmark).
    pub last_record_id: Option<u64>,
    /// The current failure, while retrying.
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastEvent {
    /// Epoch milliseconds of the event.
    pub ts: Option<i64>,
    pub channel: String,
    pub provider: String,
    pub event_id: Value,
    pub level: Level,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorEntry {
    /// Epoch milliseconds when it happened.
    pub at: i64,
    pub channel: Option<String>,
    pub message: String,
}

/// A snapshot for the UI.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FollowerStatus {
    pub running: bool,
    pub paused: bool,
    /// Events written in this run.
    pub events: u64,
    /// Over the last ten seconds.
    pub events_per_second: f64,
    pub last_event: Option<LastEvent>,
    pub channels: Vec<ChannelStatus>,
    /// Most recent first, at most 20.
    pub errors: Vec<ErrorEntry>,
    /// Events that could not be read (malformed XML) and were skipped.
    pub unreadable: u64,
    pub queued: usize,
    pub queue_capacity: usize,
    /// The live query.
    pub xpath: String,
}

#[derive(Default)]
struct Stats {
    events: u64,
    unreadable: u64,
    last: Option<LastEvent>,
    channels: Vec<ChannelStatus>,
    errors: VecDeque<ErrorEntry>,
    samples: VecDeque<(Instant, u64)>,
}

impl Stats {
    fn error(&mut self, channel: Option<&Channel>, message: &str) {
        if self.errors.len() >= MAX_ERRORS {
            self.errors.pop_back();
        }
        self.errors.push_front(ErrorEntry {
            at: chrono::Utc::now().timestamp_millis(),
            channel: channel.map(|c| c.to_string()),
            message: message.chars().take(MAX_ERROR_CHARS).collect(),
        });
    }

    fn sample(&mut self, now: Instant) {
        if self
            .samples
            .back()
            .is_some_and(|(t, _)| now.duration_since(*t) < Duration::from_millis(500))
        {
            return;
        }
        self.samples.push_back((now, self.events));
        while self
            .samples
            .front()
            .is_some_and(|(t, _)| now.duration_since(*t) > Duration::from_secs(10))
        {
            self.samples.pop_front();
        }
    }

    fn rate(&self) -> f64 {
        match (self.samples.front(), self.samples.back()) {
            (Some((t0, n0)), Some((t1, n1))) if t1 > t0 => (n1 - n0) as f64 / t1.duration_since(*t0).as_secs_f64(),
            _ => 0.0,
        }
    }
}

struct Shared {
    stop: AtomicBool,
    paused: AtomicBool,
    queued: AtomicUsize,
    stats: Mutex<Stats>,
}

impl Shared {
    fn stats(&self) -> MutexGuard<'_, Stats> {
        self.stats.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn stopping(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    fn set_state(&self, index: usize, state: ChannelState, error: Option<&EventLogError>) {
        let mut s = self.stats();
        if let Some(c) = s.channels.get_mut(index) {
            c.state = state;
            c.error = error.map(|e| e.to_string());
        }
        if let Some(e) = error {
            let channel = s.channels.get(index).map(|c| c.channel.clone());
            s.error(channel.as_ref(), &e.to_string());
        }
    }

    /// Sleeps up to `d`, waking early on stop. False when stopping.
    fn nap(&self, d: Duration) -> bool {
        let until = Instant::now() + d;
        while Instant::now() < until {
            if self.stopping() {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20).min(until.saturating_duration_since(Instant::now())));
        }
        !self.stopping()
    }

    /// Waits while paused. False when stopping.
    fn wait_unpaused(&self) -> bool {
        while self.paused.load(Ordering::SeqCst) {
            if !self.nap(Duration::from_millis(50)) {
                return false;
            }
        }
        !self.stopping()
    }
}

/// One record on its way to the writer.
struct Item {
    channel: usize,
    record_id: Option<u64>,
    record: Record,
}

/// Where a reader starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Start {
    /// Live events after this record (`None`: from the oldest record).
    After(Option<u64>),
    /// The backfill window, then live; `seed` is the newest record at start.
    Backfill { seed: Option<u64> },
}

/// A running follower. Dropping it without [`Follower::stop`] stops it too
/// (without waiting).
pub struct Follower {
    shared: Arc<Shared>,
    readers: Vec<JoinHandle<()>>,
    writer: Option<JoinHandle<Result<()>>>,
    xpath: String,
    capacity: usize,
    stop_timeout: Duration,
}

impl Follower {
    /// Checks every channel (rights, existence, the query) and starts. Fails
    /// fast with the first refusal, so missing rights for Security surface
    /// as the access-denied error the UI turns into "Restart as administrator".
    /// `on_end` runs when the follower ends on its own (a write failure).
    pub fn start(
        log: Arc<dyn EventLog>,
        config: FollowerConfig,
        sink: Arc<dyn EventSink>,
        bookmarks: Option<BookmarkStore>,
        limits: Limits,
        on_end: Box<dyn FnOnce() + Send>,
    ) -> Result<Follower> {
        let live = config.live_xpath();
        let backfill = config.backfill_xpath();
        let mut starts = Vec::new();
        let mut resumed = Vec::new();
        for channel in &config.channels {
            let probe_query = if config.backfill == Backfill::None { &live } else { &backfill };
            let newest = log.probe(channel, probe_query).map_err(anyhow::Error::new)?;
            let bookmark = bookmarks.as_ref().filter(|_| config.resume).and_then(|b| b.get(channel));
            starts.push(match (bookmark, config.backfill) {
                (Some(id), _) => Start::After(Some(id)),
                (None, Backfill::None) => Start::After(newest),
                (None, _) => Start::Backfill { seed: newest },
            });
            resumed.push(bookmark);
        }
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            queued: AtomicUsize::new(0),
            stats: Mutex::new(Stats {
                channels: config
                    .channels
                    .iter()
                    .zip(&resumed)
                    .map(|(c, bookmark)| ChannelStatus {
                        channel: c.clone(),
                        state: ChannelState::Starting,
                        events: 0,
                        last_record_id: *bookmark,
                        error: None,
                    })
                    .collect(),
                ..Stats::default()
            }),
        });
        let capacity = limits.queue.max(1);
        let (tx, rx) = sync_channel::<Item>(capacity);
        let mut readers = Vec::new();
        for (index, (channel, start)) in config.channels.iter().cloned().zip(starts).enumerate() {
            let reader = Reader {
                index,
                channel,
                start,
                live: live.clone(),
                backfill: backfill.clone(),
                log: log.clone(),
                shared: shared.clone(),
                tx: tx.clone(),
                limits: limits.clone(),
            };
            let spawned = std::thread::Builder::new()
                .name(format!("imt-eventlog-{index}"))
                .spawn(move || reader.run());
            match spawned {
                Ok(h) => readers.push(h),
                Err(e) => {
                    shared.stop.store(true, Ordering::SeqCst);
                    bail!("cannot start an event log reader: {e}");
                }
            }
        }
        drop(tx);
        let writer = Writer {
            rx,
            sink,
            bookmarks,
            channels: config.channels.clone(),
            shared: shared.clone(),
            limits: limits.clone(),
        };
        let ended = shared.clone();
        let writer = std::thread::Builder::new().name("imt-eventlog-writer".into()).spawn(move || {
            let result = imt_core::guard::catch("the event log writer", || writer.run());
            if !ended.stop.swap(true, Ordering::SeqCst) {
                // Ended on its own (a write failure): the readers stop too.
                on_end();
            }
            result
        });
        let writer = match writer {
            Ok(w) => w,
            Err(e) => {
                shared.stop.store(true, Ordering::SeqCst);
                bail!("cannot start the event log writer: {e}");
            }
        };
        Ok(Follower {
            shared,
            readers,
            writer: Some(writer),
            xpath: live,
            capacity,
            stop_timeout: limits.stop_timeout,
        })
    }

    pub fn pause(&self) {
        self.shared.paused.store(true, Ordering::SeqCst);
    }

    pub fn resume(&self) {
        self.shared.paused.store(false, Ordering::SeqCst);
    }

    /// Whether the writer still runs.
    pub fn is_running(&self) -> bool {
        self.writer.as_ref().is_some_and(|w| !w.is_finished())
    }

    pub fn status(&self) -> FollowerStatus {
        let s = self.shared.stats();
        FollowerStatus {
            running: self.is_running(),
            paused: self.shared.paused.load(Ordering::SeqCst),
            events: s.events,
            events_per_second: (s.rate() * 10.0).round() / 10.0,
            last_event: s.last.clone(),
            channels: s.channels.clone(),
            errors: s.errors.iter().cloned().collect(),
            unreadable: s.unreadable,
            queued: self.shared.queued.load(Ordering::SeqCst),
            queue_capacity: self.capacity,
            xpath: self.xpath.clone(),
        }
    }

    /// Stops reading, writes what is queued, saves the positions. Waits at
    /// most the stop timeout; the writer's failure, if any, is returned.
    pub fn stop(mut self) -> Result<()> {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.shared.paused.store(false, Ordering::SeqCst);
        let deadline = Instant::now() + self.stop_timeout;
        let threads_done = |f: &Follower| f.readers.iter().all(|r| r.is_finished()) && f.writer.as_ref().is_none_or(|w| w.is_finished());
        while !threads_done(&self) {
            if Instant::now() >= deadline {
                bail!(
                    "the event log follower did not stop within {} s; it ends in the background",
                    self.stop_timeout.as_secs()
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        for r in self.readers.drain(..) {
            let _ = r.join();
        }
        match self.writer.take().map(JoinHandle::join) {
            Some(Ok(result)) => result,
            Some(Err(_)) => Err(anyhow!("the event log writer crashed")),
            None => Ok(()),
        }
    }
}

impl Drop for Follower {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
    }
}

struct Reader {
    index: usize,
    channel: Channel,
    start: Start,
    live: String,
    backfill: String,
    log: Arc<dyn EventLog>,
    shared: Arc<Shared>,
    tx: SyncSender<Item>,
    limits: Limits,
}

/// How a stream ended.
enum Ended {
    Stopped,
    /// A query ran out of events.
    Done,
    Failed(EventLogError),
}

impl Reader {
    fn run(self) {
        let what = format!("the {} event log reader", self.channel);
        let shared = self.shared.clone();
        let index = self.index;
        let channel = self.channel.clone();
        if let Err(e) = imt_core::guard::catch(&what, || {
            self.follow();
            Ok(())
        }) {
            shared.stats().error(Some(&channel), &e.to_string());
        }
        shared.set_state(index, ChannelState::Stopped, None);
    }

    fn follow(&self) {
        let mut last = match self.start {
            Start::After(after) => after,
            Start::Backfill { seed } => {
                self.shared.set_state(self.index, ChannelState::Backfill, None);
                let mut read = None;
                let ended = match self.log.query(&self.channel, &self.backfill) {
                    Ok(stream) => self.pump(stream, &mut read, true),
                    Err(e) => Ended::Failed(e),
                };
                match ended {
                    Ended::Stopped => return,
                    Ended::Done => {}
                    // Give up on the rest of the window (reading it again would
                    // repeat events) and continue live after what was read.
                    Ended::Failed(e) => self.shared.set_state(self.index, ChannelState::Retrying, Some(&e)),
                }
                read.or(seed)
            }
        };
        loop {
            let ended = match self.log.subscribe(&self.channel, &self.live, last) {
                Ok(stream) => {
                    self.shared.set_state(self.index, ChannelState::Live, None);
                    self.pump(stream, &mut last, false)
                }
                Err(e) => Ended::Failed(e),
            };
            match ended {
                Ended::Stopped => return,
                Ended::Done => {}
                Ended::Failed(e) => self.shared.set_state(self.index, ChannelState::Retrying, Some(&e)),
            }
            if !self.shared.nap(self.limits.retry) {
                return;
            }
        }
    }

    /// Reads `stream` until it ends, stops or fails; `last` follows the
    /// record ids handed to the writer.
    fn pump(&self, mut stream: Box<dyn EventStream>, last: &mut Option<u64>, query: bool) -> Ended {
        let mut line = 0u32;
        loop {
            if !self.shared.wait_unpaused() {
                return Ended::Stopped;
            }
            let events = match stream.next_batch(self.limits.batch.max(1), self.limits.wait) {
                Ok(Batch::Events(events)) => events,
                Ok(Batch::Idle) => continue,
                // A query is done; a subscription that ends is reopened.
                Ok(Batch::End) => return Ended::Done,
                Err(e) => return Ended::Failed(e),
            };
            for event in events {
                line = line.wrapping_add(1);
                let Some(item) = self.item(event, line) else { continue };
                // The bookmark event itself, should a source repeat it.
                if item.record_id.is_some() && item.record_id == *last && !query {
                    continue;
                }
                let id = item.record_id;
                if !self.send(item) {
                    return Ended::Stopped;
                }
                if id.is_some() {
                    *last = id;
                }
            }
        }
    }

    fn item(&self, event: RawEvent, line: u32) -> Option<Item> {
        match record_from_xml(&event.xml, event.message.as_deref(), line) {
            Ok(record) => {
                let record_id = record.fields.get("RecordId").and_then(Value::as_u64).filter(|id| *id > 0);
                Some(Item {
                    channel: self.index,
                    record_id,
                    record,
                })
            }
            Err(e) => {
                let mut s = self.shared.stats();
                s.unreadable += 1;
                if s.unreadable <= 3 {
                    s.error(Some(&self.channel), &format!("skipped an unreadable event: {e}"));
                }
                None
            }
        }
    }

    /// Hands `item` to the writer, waiting while the queue is full. False when stopping.
    fn send(&self, mut item: Item) -> bool {
        loop {
            match self.tx.try_send(item) {
                Ok(()) => {
                    self.shared.queued.fetch_add(1, Ordering::SeqCst);
                    return true;
                }
                Err(TrySendError::Full(back)) => {
                    item = back;
                    if !self.shared.nap(Duration::from_millis(5)) {
                        return false;
                    }
                }
                Err(TrySendError::Disconnected(_)) => return false,
            }
        }
    }
}

struct Writer {
    rx: Receiver<Item>,
    sink: Arc<dyn EventSink>,
    bookmarks: Option<BookmarkStore>,
    channels: Vec<Channel>,
    shared: Arc<Shared>,
    limits: Limits,
}

impl Writer {
    fn run(mut self) -> Result<()> {
        let mut batch: Vec<Item> = Vec::with_capacity(self.limits.batch);
        let mut dirty = false;
        let mut saved = Instant::now();
        let mut stop_seen: Option<Instant> = None;
        let max = self.limits.batch.max(1);
        let flush_wait = self.limits.wait.min(Duration::from_millis(200));
        loop {
            let disconnected = match self.rx.recv_timeout(flush_wait) {
                Ok(item) => {
                    batch.push(item);
                    while batch.len() < max {
                        match self.rx.try_recv() {
                            Ok(item) => batch.push(item),
                            Err(_) => break,
                        }
                    }
                    false
                }
                Err(RecvTimeoutError::Timeout) => false,
                Err(RecvTimeoutError::Disconnected) => true,
            };
            if !batch.is_empty() {
                self.shared.queued.fetch_sub(batch.len(), Ordering::SeqCst);
                if let Err(e) = self.write(&mut batch) {
                    self.save();
                    let message = format!("writing the event log files failed: {e:#}");
                    self.shared.stats().error(None, &message);
                    return Err(anyhow!(message));
                }
                dirty = true;
            }
            if dirty && saved.elapsed() >= self.limits.save_every {
                self.save();
                dirty = false;
                saved = Instant::now();
            }
            self.shared.stats().sample(Instant::now());
            if self.shared.stopping() {
                let since = *stop_seen.get_or_insert_with(Instant::now);
                // Readers end within one wait; one that hangs in the Event Log
                // must not keep the files open.
                if disconnected || since.elapsed() > self.limits.wait * 4 + Duration::from_secs(1) {
                    break;
                }
            } else if disconnected {
                break;
            }
        }
        self.save();
        Ok(())
    }

    fn write(&mut self, batch: &mut Vec<Item>) -> Result<()> {
        let records: Vec<Record> = batch.iter().map(|i| i.record.clone()).collect();
        self.sink.write(&records)?;
        let mut s = self.shared.stats();
        s.events += batch.len() as u64;
        for item in batch.drain(..) {
            if let Some(c) = s.channels.get_mut(item.channel) {
                c.events += 1;
                if item.record_id.is_some() {
                    c.last_record_id = item.record_id;
                }
            }
            if let (Some(store), Some(id), Some(channel)) = (self.bookmarks.as_mut(), item.record_id, self.channels.get(item.channel)) {
                store.set(channel, id);
            }
            let text = |k: &str| item.record.fields.get(k).map(|v| imt_core::model::value_text(v).into_owned());
            s.last = Some(LastEvent {
                ts: item.record.ts,
                channel: text("Channel").unwrap_or_default(),
                provider: text("Provider").unwrap_or_default(),
                event_id: item.record.fields.get("EventId").cloned().unwrap_or(Value::Null),
                level: item.record.level,
            });
        }
        Ok(())
    }

    fn save(&self) {
        if let Some(store) = &self.bookmarks {
            if let Err(e) = store.save() {
                self.shared
                    .stats()
                    .error(None, &format!("cannot save the event log positions: {e:#}"));
            }
        }
    }
}

#[cfg(test)]
mod tests;
