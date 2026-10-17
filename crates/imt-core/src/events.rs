//! Typed event bus between the engine and plugins (ADR 0003).
//!
//! Any `Send + Sync + 'static` type is an event, so plugins define their own
//! by declaring a struct; subscribers are matched by `TypeId`. The engine
//! publishes the built-in events below.
//!
//! Each subscriber has its own bounded queue and worker thread, so events
//! reach it in publish order and off the engine thread. [`EventBus::publish`]
//! never blocks: when a queue is full the oldest event is dropped and counted.
//! Delivery runs under the subscriber plugin's guard: a panic is contained,
//! counted towards quarantine, and a quarantined or inactive subscriber is
//! skipped.

use crate::guard::{CallKind, Guard, Health};
use crate::plugin::PluginStatus;
use crate::workspace::LiveAlert;
use anyhow::{anyhow, Result};
use serde::Serialize;
use std::any::{Any, TypeId};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};

/// Default queue length per subscriber.
pub const DEFAULT_CAPACITY: usize = 1024;

/// Records were imported or appended (live tail).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordsAppended {
    pub count: usize,
    /// Ids of the sources that received records.
    pub sources: Vec<u32>,
}

/// A new profile was applied (records are re-imported next).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileChanged {
    pub name: String,
}

/// A live detector or alert rule fired.
#[derive(Debug, Clone)]
pub struct LiveAlertRaised(pub LiveAlert);

/// A plugin's status changed (quarantine, cascade, reset, disable).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginStateChanged {
    pub id: String,
    pub status: PluginStatus,
}

/// Everything was closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceCleared;

/// An error snapshot was saved (published by the app shell).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotTaken {
    pub id: String,
}

/// Delivery counters of one subscriber.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriberMetrics {
    pub id: u64,
    pub plugin: String,
    /// Type name of the event.
    pub event: String,
    pub delivered: u64,
    /// Dropped because the queue was full (oldest first).
    pub dropped: u64,
    /// Deliveries that failed (the handler panicked or overran its budget).
    pub failures: u64,
    /// Not delivered because the subscriber was quarantined, hung or inactive.
    pub skipped: u64,
    pub queued: usize,
}

/// Identifies a subscription for [`EventBus::unsubscribe`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SubscriptionId(pub u64);

type Payload = Arc<dyn Any + Send + Sync>;
type Handler = Box<dyn Fn(&(dyn Any + Send + Sync)) + Send + Sync>;

struct Queue {
    items: VecDeque<Payload>,
    closed: bool,
}

struct Subscriber {
    id: u64,
    plugin: String,
    event: &'static str,
    type_id: TypeId,
    capacity: usize,
    queue: Mutex<Queue>,
    ready: Condvar,
    delivered: AtomicU64,
    dropped: AtomicU64,
    failures: AtomicU64,
    skipped: AtomicU64,
}

impl Subscriber {
    fn lock(&self) -> std::sync::MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn close(&self) {
        self.lock().closed = true;
        self.ready.notify_all();
    }

    fn push(&self, item: Payload) {
        let mut q = self.lock();
        if q.closed {
            return;
        }
        if q.items.len() >= self.capacity {
            q.items.pop_front();
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
        q.items.push_back(item);
        drop(q);
        self.ready.notify_one();
    }

    /// The next event, or `None` once closed.
    fn next(&self) -> Option<Payload> {
        let mut q = self.lock();
        loop {
            if q.closed {
                return None;
            }
            if let Some(item) = q.items.pop_front() {
                return Some(item);
            }
            q = self.ready.wait(q).unwrap_or_else(|p| p.into_inner());
        }
    }

    fn metrics(&self) -> SubscriberMetrics {
        SubscriberMetrics {
            id: self.id,
            plugin: self.plugin.clone(),
            event: self.event.to_string(),
            delivered: self.delivered.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
            failures: self.failures.load(Ordering::Relaxed),
            skipped: self.skipped.load(Ordering::Relaxed),
            queued: self.lock().items.len(),
        }
    }
}

/// Publish/subscribe for typed events. Cheap to share (`Arc<EventBus>`).
pub struct EventBus {
    health: Arc<Health>,
    capacity: usize,
    subscribers: RwLock<Vec<Arc<Subscriber>>>,
    next_id: AtomicU64,
}

impl EventBus {
    pub fn new(health: Arc<Health>) -> EventBus {
        Self::with_capacity(health, DEFAULT_CAPACITY)
    }

    /// A bus whose subscribers queue at most `capacity` events (at least 1).
    pub fn with_capacity(health: Arc<Health>, capacity: usize) -> EventBus {
        EventBus {
            health,
            capacity: capacity.max(1),
            subscribers: RwLock::new(Vec::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// Calls `handler` with every `E` published from now on, in order, on a
    /// worker thread of its own, under `plugin`'s guard.
    pub fn subscribe<E: Send + Sync + 'static>(
        &self,
        plugin: &str,
        handler: impl Fn(&E) + Send + Sync + 'static,
    ) -> Result<SubscriptionId> {
        let handler: Handler = Box::new(move |any| any.downcast_ref::<E>().into_iter().for_each(&handler));
        let sub = Arc::new(Subscriber {
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            plugin: plugin.to_string(),
            event: std::any::type_name::<E>(),
            type_id: TypeId::of::<E>(),
            capacity: self.capacity,
            queue: Mutex::new(Queue {
                items: VecDeque::new(),
                closed: false,
            }),
            ready: Condvar::new(),
            delivered: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            failures: AtomicU64::new(0),
            skipped: AtomicU64::new(0),
        });
        let guard = Guard::new(plugin, self.health.clone());
        let worker = sub.clone();
        std::thread::Builder::new()
            .name(format!("imt-events-{plugin}"))
            .spawn(move || deliver(&worker, &guard, &handler))
            .map_err(|e| anyhow!("could not start the event worker for plugin \"{plugin}\": {e}"))?;
        let id = sub.id;
        self.subscribers.write().unwrap_or_else(|p| p.into_inner()).push(sub);
        Ok(SubscriptionId(id))
    }

    /// Stops a subscription; events still queued are discarded.
    pub fn unsubscribe(&self, id: SubscriptionId) {
        let mut subs = self.subscribers.write().unwrap_or_else(|p| p.into_inner());
        if let Some(i) = subs.iter().position(|s| s.id == id.0) {
            subs.remove(i).close();
        }
    }

    /// Stops every subscription of `plugin`.
    pub fn unsubscribe_plugin(&self, plugin: &str) {
        let mut subs = self.subscribers.write().unwrap_or_else(|p| p.into_inner());
        subs.retain(|s| {
            let keep = s.plugin != plugin;
            if !keep {
                s.close();
            }
            keep
        });
    }

    /// Queues `event` for every subscriber of its type. Never blocks on a
    /// subscriber; a full queue drops its oldest event.
    pub fn publish<E: Send + Sync + 'static>(&self, event: E) {
        let subs = self.subscribers.read().unwrap_or_else(|p| p.into_inner());
        let mut matching = subs.iter().filter(|s| s.type_id == TypeId::of::<E>()).peekable();
        if matching.peek().is_none() {
            return;
        }
        let payload: Payload = Arc::new(event);
        for s in matching {
            s.push(payload.clone());
        }
    }

    /// Whether anyone listens to `E` (lets publishers skip building events).
    pub fn has_subscribers<E: 'static>(&self) -> bool {
        let subs = self.subscribers.read().unwrap_or_else(|p| p.into_inner());
        subs.iter().any(|s| s.type_id == TypeId::of::<E>())
    }

    pub fn metrics(&self) -> Vec<SubscriberMetrics> {
        let subs = self.subscribers.read().unwrap_or_else(|p| p.into_inner());
        subs.iter().map(|s| s.metrics()).collect()
    }
}

impl Drop for EventBus {
    fn drop(&mut self) {
        let subs = self.subscribers.get_mut().unwrap_or_else(|p| p.into_inner());
        for s in subs.drain(..) {
            s.close();
        }
    }
}

fn deliver(sub: &Subscriber, guard: &Guard, handler: &Handler) {
    while let Some(item) = sub.next() {
        let mut ran = false;
        let result = guard.call(CallKind::Event, sub.event, || {
            ran = true;
            handler(item.as_ref());
            Ok(())
        });
        let counter = match (result, ran) {
            (Ok(()), _) => &sub.delivered,
            (Err(_), true) => &sub.failures,
            // Refused before running: quarantined, hung or inactive.
            (Err(_), false) => &sub.skipped,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests;
