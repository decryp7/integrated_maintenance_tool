//! Lazy loading and idle unloading of the model. The (slow) load runs on a background thread the
//! first time the model is needed or when `preload` is called; after `idle` without use the model
//! is dropped to give its memory back, and loaded again on demand.

use super::provider::LlmProvider;
use super::types::{ChatRequest, ChatResponse};
use crate::error::{AiError, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Builds the real provider (loads the weights). Runs off the UI thread.
pub trait ModelLoader: Send + Sync {
    fn load(&self) -> Result<Arc<dyn LlmProvider>>;
    /// Context length to report before the model is loaded.
    fn context_hint(&self) -> usize {
        4096
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadState {
    Unloaded,
    Loading,
    Ready,
    Failed(String),
}

enum Slot {
    Unloaded,
    Loading,
    Ready {
        provider: Arc<dyn LlmProvider>,
        last_used: Instant,
        busy: usize,
    },
    Failed(String),
}

struct Shared {
    slot: Mutex<Slot>,
    changed: Condvar,
}

pub struct LazyProvider {
    loader: Arc<dyn ModelLoader>,
    shared: Arc<Shared>,
    idle: Mutex<Duration>,
    stop: Arc<AtomicBool>,
}

fn lock(m: &Mutex<Slot>) -> MutexGuard<'_, Slot> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl LazyProvider {
    pub fn new(loader: Arc<dyn ModelLoader>, idle: Duration) -> Self {
        LazyProvider {
            loader,
            shared: Arc::new(Shared {
                slot: Mutex::new(Slot::Unloaded),
                changed: Condvar::new(),
            }),
            idle: Mutex::new(idle),
            stop: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn set_idle(&self, idle: Duration) {
        *self.idle.lock().unwrap_or_else(|p| p.into_inner()) = idle;
    }

    pub fn state(&self) -> LoadState {
        match &*lock(&self.shared.slot) {
            Slot::Unloaded => LoadState::Unloaded,
            Slot::Loading => LoadState::Loading,
            Slot::Ready { .. } => LoadState::Ready,
            Slot::Failed(m) => LoadState::Failed(m.clone()),
        }
    }

    /// Starts loading in the background if nothing is loaded or loading.
    pub fn preload(&self) {
        let start = {
            let mut s = lock(&self.shared.slot);
            if matches!(*s, Slot::Unloaded | Slot::Failed(_)) {
                *s = Slot::Loading;
                true
            } else {
                false
            }
        };
        if start {
            let (loader, shared) = (self.loader.clone(), self.shared.clone());
            std::thread::spawn(move || {
                // A panic inside the engine must not leave the slot in `Loading` forever.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| loader.load()))
                    .unwrap_or_else(|_| Err(AiError::Engine("the model loader crashed".into())));
                let mut s = lock(&shared.slot);
                *s = match result {
                    Ok(provider) => Slot::Ready {
                        provider,
                        last_used: Instant::now(),
                        busy: 0,
                    },
                    Err(e) => Slot::Failed(e.to_string()),
                };
                shared.changed.notify_all();
            });
        }
    }

    /// Waits for the model, loading it if needed, and marks it busy until the guard is dropped.
    fn acquire(&self, cancel: Option<&Arc<AtomicBool>>) -> Result<Arc<dyn LlmProvider>> {
        self.preload();
        let mut s = lock(&self.shared.slot);
        loop {
            match &mut *s {
                Slot::Ready { provider, last_used, busy } => {
                    *last_used = Instant::now();
                    *busy += 1;
                    return Ok(provider.clone());
                }
                Slot::Failed(m) => {
                    let e = AiError::Engine(m.clone());
                    // Allow a retry on the next call.
                    *s = Slot::Unloaded;
                    return Err(e);
                }
                _ => {}
            }
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                return Err(AiError::Cancelled);
            }
            let (guard, _) = self
                .shared
                .changed
                .wait_timeout(s, Duration::from_millis(50))
                .unwrap_or_else(|p| p.into_inner());
            s = guard;
        }
    }

    fn release(&self) {
        if let Slot::Ready { busy, last_used, .. } = &mut *lock(&self.shared.slot) {
            *busy = busy.saturating_sub(1);
            *last_used = Instant::now();
        }
    }

    /// Drops the model if it has been idle for the configured time; returns whether it did.
    pub fn unload_if_idle(&self, now: Instant) -> bool {
        let idle = *self.idle.lock().unwrap_or_else(|p| p.into_inner());
        let mut s = lock(&self.shared.slot);
        let expired = matches!(&*s, Slot::Ready { last_used, busy: 0, .. } if now.saturating_duration_since(*last_used) >= idle);
        if expired {
            *s = Slot::Unloaded;
        }
        expired
    }

    pub fn unload(&self) {
        let mut s = lock(&self.shared.slot);
        if matches!(&*s, Slot::Ready { busy: 0, .. } | Slot::Failed(_)) {
            *s = Slot::Unloaded;
        }
    }

    /// Checks for idleness every `every` until `shutdown` (or the provider is dropped).
    pub fn spawn_idle_watcher(self: &Arc<Self>, every: Duration) -> std::thread::JoinHandle<()> {
        let weak = Arc::downgrade(self);
        let stop = self.stop.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(every);
                match weak.upgrade() {
                    Some(p) => {
                        p.unload_if_idle(Instant::now());
                    }
                    None => break,
                }
            }
        })
    }

    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl LlmProvider for LazyProvider {
    fn id(&self) -> &str {
        "embedded"
    }

    fn display_name(&self) -> String {
        match &*lock(&self.shared.slot) {
            Slot::Ready { provider, .. } => provider.display_name(),
            _ => "Local model (not loaded)".into(),
        }
    }

    fn context_length(&self) -> usize {
        match &*lock(&self.shared.slot) {
            Slot::Ready { provider, .. } => provider.context_length(),
            _ => self.loader.context_hint(),
        }
    }

    fn chat(&self, req: &ChatRequest, on_delta: &mut dyn FnMut(&str)) -> Result<ChatResponse> {
        let provider = self.acquire(req.cancel.as_ref())?;
        let result = provider.chat(req, on_delta);
        self.release();
        result
    }
}
