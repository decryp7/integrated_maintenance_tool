//! The browsers of the open Debrief windows and their life cycle (ADR 0057): [`Renderers`] is the one owner of each
//! window's [`Session`] and its phase ([`super::lifecycle`]). Any end (the user closing the window or quitting the
//! browser, a crash, a lost connection) is detected and moves it to gone, and the next use starts a fresh browser. A
//! run (a research) holds a [`Lease`] so it keeps to its own browser.

use super::lifecycle::{GoneReason, Lifecycle, Phase};
use super::renderer::{failed, ACCENT};
use chromium_driver::launch::{LaunchOptions, Profile};
use chromium_driver::{Chromium, Page, WindowBounds};
use debrief_ai::browser::BrowserError;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::{Duration, Instant};

/// How long a closing browser may take to exit by itself before it is ended by force.
#[cfg(not(test))]
const CLOSE_GRACE: Duration = Duration::from_secs(3);
#[cfg(test)]
const CLOSE_GRACE: Duration = Duration::from_millis(600);
/// The shorter grace when Debrief itself is quitting (the teardown step has a limit).
pub const QUIT_GRACE: Duration = Duration::from_millis(1200);
/// Where a browser may come from, in order: the user's setting, the one Debrief installed, one found on the machine.
#[derive(Debug, Clone, Default)]
pub struct Sources {
    pub configured: Option<PathBuf>,
    pub installed: Option<PathBuf>,
}

impl Sources {
    pub fn resolve(&self) -> Option<chromium_driver::BrowserExe> {
        let mk = |p: &PathBuf| {
            p.is_file().then(|| chromium_driver::BrowserExe {
                flavor: chromium_driver::Flavor::Chromium,
                path: p.clone(),
            })
        };
        self.configured
            .as_ref()
            .and_then(mk)
            .or_else(|| self.installed.as_ref().and_then(mk))
            .or_else(fallback_browser)
    }
}

/// One window's running browser and its page. The window is a normal one on screen; Debrief never moves or hides it.
pub struct Session {
    pub(super) chromium: Mutex<Chromium>,
    pub(super) page: Page,
    /// A page needed the user (a check, consent, a sign-in): they used the window, so it is not closed behind their back.
    pub(super) handed_off: AtomicBool,
    /// The address the AI last navigated to (after redirects); a different one later means the user navigated.
    last_url: Mutex<Option<String>>,
}

impl Session {
    #[cfg(test)]
    pub fn page(&self) -> &Page {
        &self.page
    }

    /// Why this browser can no longer be used, if so: no round trip to the browser.
    pub(super) fn gone(&self) -> Option<GoneReason> {
        crate::sync::lock(&self.chromium).gone(&self.page).map(GoneReason::from)
    }

    /// As [`Session::gone`], but skipped (None) while another thread uses the browser, and with `probe` also asking the
    /// browser whether the page still exists (a close it did not report).
    fn watch(&self, probe: bool) -> Option<GoneReason> {
        if let Ok(mut c) = self.chromium.try_lock() {
            if let Some(g) = c.gone(&self.page) {
                return Some(g.into());
            }
        }
        (probe && matches!(self.page.exists(), Ok(false))).then_some(GoneReason::WindowClosed)
    }

    fn kill(&self) {
        crate::sync::lock(&self.chromium).kill();
    }

    /// Closes the browser (gracefully, then by force) and removes its throwaway profile.
    fn close(&self, grace: Duration) {
        crate::sync::lock(&self.chromium).close_gracefully(grace);
    }

    pub(super) fn remember_url(&self) {
        if let Ok(i) = self.page.info() {
            *crate::sync::lock(&self.last_url) = Some(i.url);
        }
    }

    /// The user took part in this browser: a page needed them, they navigated the page, or they opened another one.
    fn user_took_part(&self) -> bool {
        if self.handed_off.load(Ordering::SeqCst) {
            return true;
        }
        let moved = match (crate::sync::lock(&self.last_url).as_ref(), self.page.info()) {
            (Some(last), Ok(now)) => *last != now.url,
            _ => false,
        };
        moved || crate::sync::lock(&self.chromium).page_count().is_ok_and(|n| n > 1)
    }

    #[cfg(test)]
    pub fn pid(&self) -> Option<u32> {
        crate::sync::lock(&self.chromium).pid()
    }
}

/// What happened to a window's browser when its work ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finish {
    /// There was no browser.
    NotRunning,
    /// The browser was closed and its profile removed.
    Closed,
    /// The user took part (a page needed them, or they used the window): it stays open until they close it.
    KeptOpen,
}

/// One run's hold on a browser (a research): the first page use picks the browser, and once that browser is gone the
/// run fails ([`BrowserError::Closed`]) instead of carrying on in a new one. The default lease is unpinned: every use
/// gets the window's browser, started when none runs.
#[derive(Clone, Default)]
pub struct Lease(Option<Arc<Mutex<Option<u64>>>>);

impl Lease {
    /// A lease for one run.
    pub fn run() -> Lease {
        Lease(Some(Arc::new(Mutex::new(None))))
    }
}

/// A window's browser in its life cycle ([`Phase`]); no slot is [`Phase::None`]. Only [`Inner::set`] changes it.
enum Slot {
    Launching {
        generation: u64,
    },
    Ready {
        generation: u64,
        session: Arc<Session>,
        kept: bool,
    },
    Closing {
        generation: u64,
    },
    Gone {
        reason: GoneReason,
    },
}

impl Slot {
    fn lifecycle(&self) -> Lifecycle {
        let (phase, kept) = match self {
            Slot::Launching { .. } => (Phase::Launching, false),
            Slot::Ready { kept, .. } => (Phase::Ready, *kept),
            Slot::Closing { .. } => (Phase::Closing, false),
            Slot::Gone { reason } => (Phase::Gone { reason: *reason }, false),
        };
        Lifecycle { phase, kept }
    }
}

/// Where a window's browser opens (on that window's display), or `None` to let the browser choose.
pub type Placer = Arc<dyn Fn(&str) -> Option<WindowBounds> + Send + Sync>;

/// Hears every change of a window's browser life cycle (`owner`, new state).
pub type Listener = Arc<dyn Fn(&str, Lifecycle) + Send + Sync>;

/// How often a running browser is checked for an end nobody reported (the process, the connection, the page).
const WATCH_EVERY: Duration = Duration::from_millis(200);
/// Every this many checks the browser is also asked whether the page still exists.
const PROBE_EVERY: u32 = 5;
/// The longest a caller waits for another thread's launch or close of the same browser.
const WAIT_FOR_OTHER: Duration = Duration::from_secs(60);

#[derive(Default)]
struct Inner {
    slots: Mutex<HashMap<String, Slot>>,
    changed: Condvar,
    sources: RwLock<Sources>,
    listeners: RwLock<Vec<Listener>>,
    placer: RwLock<Option<Placer>>,
    generations: AtomicU64,
}

impl Inner {
    /// The one place a browser changes phase: checks the step, records it and wakes waiters. The caller tells the
    /// listener ([`Inner::tell`]) after letting go of the lock.
    fn set(&self, slots: &mut HashMap<String, Slot>, owner: &str, to: Slot) -> Lifecycle {
        let from = slots.get(owner).map_or(Phase::None, |s| s.lifecycle().phase);
        let next = to.lifecycle();
        debug_assert!(from.may_become(next.phase), "browser life cycle: {from:?} → {:?}", next.phase);
        if !from.may_become(next.phase) {
            log::warn!("The browser of {owner} went from {from:?} to {:?}", next.phase);
        }
        slots.insert(owner.to_string(), to);
        self.changed.notify_all();
        next
    }

    fn tell(&self, owner: &str, l: Lifecycle) {
        let listeners = crate::sync::read(&self.listeners).clone();
        for f in listeners {
            f(owner, l);
        }
    }

    /// The browser `generation` of `owner` ended by itself (`reason`): it is gone, its "kept" mark with it; whatever
    /// is left of it (a browser without a window, a profile) is removed. False when it was already no longer ready.
    fn lost(&self, owner: &str, generation: u64, reason: GoneReason) -> bool {
        let mut slots = crate::sync::lock(&self.slots);
        let session = match slots.get(owner) {
            Some(Slot::Ready {
                generation: g, session, ..
            }) if *g == generation => session.clone(),
            _ => return false,
        };
        let l = self.set(&mut slots, owner, Slot::Gone { reason });
        drop(slots);
        log::info!("The AI browser of {owner} is gone ({reason:?}); the next use starts a new one");
        self.tell(owner, l);
        session.close(CLOSE_GRACE);
        true
    }

    /// Waits (bounded) until another thread's launch or close of `owner`'s browser is over.
    fn wait_settled<'a>(
        &self,
        mut slots: std::sync::MutexGuard<'a, HashMap<String, Slot>>,
        owner: &str,
    ) -> Result<std::sync::MutexGuard<'a, HashMap<String, Slot>>, BrowserError> {
        let end = Instant::now() + WAIT_FOR_OTHER;
        while matches!(slots.get(owner), Some(Slot::Launching { .. } | Slot::Closing { .. })) {
            let left = end.checked_duration_since(Instant::now()).ok_or(BrowserError::Timeout)?;
            slots = self.changed.wait_timeout(slots, left).unwrap_or_else(|e| e.into_inner()).0;
        }
        Ok(slots)
    }
}

/// The browsers of the open windows, each with its life cycle ([`super::lifecycle`]): this is its only owner.
#[derive(Default)]
pub struct Renderers {
    inner: Arc<Inner>,
}

impl Renderers {
    #[cfg(test)]
    pub fn set_sources(&self, s: Sources) {
        *crate::sync::write(&self.inner.sources) = s;
    }

    pub fn set_configured(&self, p: Option<PathBuf>) {
        crate::sync::write(&self.inner.sources).configured = p;
    }

    pub fn set_installed(&self, p: Option<PathBuf>) {
        crate::sync::write(&self.inner.sources).installed = p;
    }

    /// Tells where each window's browser opens ([`super::placement`]).
    pub fn set_placer(&self, f: Placer) {
        *crate::sync::write(&self.inner.placer) = Some(f);
    }

    /// Adds a listener of every life-cycle change (the window's `lifecycle` events; the service-health registry).
    pub fn listen(&self, f: Listener) {
        crate::sync::write(&self.inner.listeners).push(f);
    }

    /// Where the window's browser is in its life.
    #[cfg_attr(not(test), allow(dead_code))] // the service-health registry (ADR 0055) reads it
    pub fn lifecycle(&self, owner: &str) -> Lifecycle {
        crate::sync::lock(&self.inner.slots)
            .get(owner)
            .map_or(Lifecycle::NONE, Slot::lifecycle)
    }

    /// Every window's browser that was ever started, for health reports.
    #[allow(dead_code)] // the service-health registry (ADR 0055) reads it
    pub fn snapshot(&self) -> Vec<(String, Lifecycle)> {
        let mut all: Vec<_> = crate::sync::lock(&self.inner.slots)
            .iter()
            .map(|(o, s)| (o.clone(), s.lifecycle()))
            .collect();
        all.sort_by(|a, b| a.0.cmp(&b.0));
        all
    }

    /// The window's browser for a use under `lease`: a pinned lease gets its own browser or [`BrowserError::Closed`].
    pub fn session_in(&self, owner: &str, lease: &Lease) -> Result<Arc<Session>, BrowserError> {
        loop {
            let slots = crate::sync::lock(&self.inner.slots);
            let mut slots = self.inner.wait_settled(slots, owner)?;
            let pinned = lease.0.as_ref().and_then(|p| *crate::sync::lock(p));
            if let Some(Slot::Ready { generation, session, .. }) = slots.get(owner) {
                let (generation, session) = (*generation, session.clone());
                drop(slots);
                if let Some(reason) = session.gone() {
                    self.inner.lost(owner, generation, reason);
                    continue;
                }
                if pinned.is_some_and(|p| p != generation) {
                    return Err(BrowserError::Closed);
                }
                if let Some(p) = &lease.0 {
                    *crate::sync::lock(p) = Some(generation);
                }
                return Ok(session);
            }
            if pinned.is_some() {
                // This run's browser is gone: the run stops instead of going on in a new one.
                return Err(BrowserError::Closed);
            }
            let generation = self.inner.generations.fetch_add(1, Ordering::SeqCst) + 1;
            let l = self.inner.set(&mut slots, owner, Slot::Launching { generation });
            drop(slots);
            self.inner.tell(owner, l);
            let session = self.launch(owner, generation)?;
            if let Some(p) = &lease.0 {
                *crate::sync::lock(p) = Some(generation);
            }
            return Ok(session);
        }
    }

    /// Starts the browser of a `Launching` slot (no lock held meanwhile) and makes it ready, unless it was stopped.
    fn launch(&self, owner: &str, generation: u64) -> Result<Arc<Session>, BrowserError> {
        let started = self.start(owner);
        let mut slots = crate::sync::lock(&self.inner.slots);
        let launching = matches!(slots.get(owner), Some(Slot::Launching { generation: g }) if *g == generation);
        let stopped = matches!(slots.get(owner), Some(Slot::Closing { generation: g }) if *g == generation);
        match started {
            Ok(session) if launching => {
                let l = self.inner.set(
                    &mut slots,
                    owner,
                    Slot::Ready {
                        generation,
                        session: session.clone(),
                        kept: false,
                    },
                );
                drop(slots);
                self.watch(owner, generation, &session);
                self.inner.tell(owner, l);
                Ok(session)
            }
            Ok(session) => {
                // Stopped (or its window closed) while it started: it ends at once.
                let l = stopped.then(|| self.inner.set(&mut slots, owner, Slot::Gone { reason: GoneReason::Ended }));
                drop(slots);
                session.kill();
                drop(session);
                if let Some(l) = l {
                    self.inner.tell(owner, l);
                }
                Err(BrowserError::Failed("the browser was stopped while it started".into()))
            }
            Err(e) => {
                let to = if launching {
                    Slot::Gone {
                        reason: GoneReason::LaunchFailed,
                    }
                } else {
                    Slot::Gone { reason: GoneReason::Ended }
                };
                let l = (launching || stopped).then(|| self.inner.set(&mut slots, owner, to));
                drop(slots);
                if let Some(l) = l {
                    self.inner.tell(owner, l);
                }
                Err(e)
            }
        }
    }

    fn start(&self, owner: &str) -> Result<Arc<Session>, BrowserError> {
        let exe = crate::sync::read(&self.inner.sources).resolve().ok_or_else(|| {
            BrowserError::Failed(
                "no Chromium-based browser was found: install Edge, Chrome or Chromium, or download one in Settings ▸ Web browser".into(),
            )
        })?;
        let placer = crate::sync::read(&self.inner.placer).clone();
        let opts = LaunchOptions {
            profile: Profile::Throwaway,
            accent: Some(ACCENT),
            window: placer.and_then(|p| p(owner)),
            ..LaunchOptions::default()
        };
        let chromium = Chromium::launch(&exe, &opts).map_err(failed)?;
        let page = chromium.new_page().map_err(failed)?;
        Ok(Arc::new(Session {
            chromium: Mutex::new(chromium),
            page,
            handed_off: AtomicBool::new(false),
            last_url: Mutex::new(None),
        }))
    }

    /// Watches a ready browser until it is no longer this generation's: an end nobody reported (the user closed the
    /// window or quit the browser, it crashed, the connection dropped) moves it to gone at once, even while idle.
    fn watch(&self, owner: &str, generation: u64, session: &Arc<Session>) {
        let (inner, owner, session) = (Arc::downgrade(&self.inner), owner.to_string(), Arc::downgrade(session));
        let spawned = std::thread::Builder::new().name("ai-browser-watch".into()).spawn(move || {
            let mut tick = 0u32;
            loop {
                std::thread::sleep(WATCH_EVERY);
                tick = tick.wrapping_add(1);
                let (Some(inner), Some(session)) = (inner.upgrade(), session.upgrade()) else {
                    return;
                };
                let current = matches!(crate::sync::lock(&inner.slots).get(&owner),
                    Some(Slot::Ready { generation: g, .. }) if *g == generation);
                if !current {
                    return;
                }
                if let Some(reason) = session.watch(tick.is_multiple_of(PROBE_EVERY)) {
                    inner.lost(&owner, generation, reason);
                    return;
                }
            }
        });
        if let Err(e) = spawned {
            log::warn!("The AI browser is not watched ({e}); a closed window is noticed on its next use");
        }
    }

    /// The window's ready browser (nothing is started); a browser found gone is moved to gone first.
    pub fn current(&self, owner: &str) -> Option<Arc<Session>> {
        let (generation, session) = match crate::sync::lock(&self.inner.slots).get(owner) {
            Some(Slot::Ready { generation, session, .. }) => (*generation, session.clone()),
            _ => return None,
        };
        match session.gone() {
            Some(reason) => {
                self.inner.lost(owner, generation, reason);
                None
            }
            None => Some(session),
        }
    }

    /// Debrief ends the window's browser (`grace`: gracefully first; none: at once). Ready or launching: it goes
    /// through closing to gone; a launch in progress ends when it started. True when a browser was running or starting.
    fn end(&self, owner: &str, grace: Option<Duration>) -> bool {
        let mut slots = crate::sync::lock(&self.inner.slots);
        let (generation, session) = match slots.get(owner) {
            Some(Slot::Ready { generation, session, .. }) => (*generation, Some(session.clone())),
            Some(Slot::Launching { generation }) => (*generation, None),
            _ => return false,
        };
        let l = self.inner.set(&mut slots, owner, Slot::Closing { generation });
        drop(slots);
        self.inner.tell(owner, l);
        let Some(session) = session else { return true };
        match grace {
            Some(g) => session.close(g),
            None => session.kill(),
        }
        let mut slots = crate::sync::lock(&self.inner.slots);
        let l = matches!(slots.get(owner), Some(Slot::Closing { generation: g }) if *g == generation)
            .then(|| self.inner.set(&mut slots, owner, Slot::Gone { reason: GoneReason::Ended }));
        drop(slots);
        if let Some(l) = l {
            self.inner.tell(owner, l);
        }
        true
    }

    /// Ends the window's browser at once; true when one was running.
    pub fn kill(&self, owner: &str) -> bool {
        self.end(owner, None)
    }

    /// Research ended: closes the window's browser unless the user took part in it (then it is kept open, marked so).
    pub fn finish(&self, owner: &str) -> Finish {
        let Some(s) = self.current(owner) else { return Finish::NotRunning };
        if s.user_took_part() {
            let mut slots = crate::sync::lock(&self.inner.slots);
            let generation = match slots.get(owner) {
                Some(Slot::Ready { generation, session, .. }) if Arc::ptr_eq(session, &s) => Some(*generation),
                _ => None,
            };
            let l = generation.map(|generation| {
                self.inner.set(
                    &mut slots,
                    owner,
                    Slot::Ready {
                        generation,
                        session: s.clone(),
                        kept: true,
                    },
                )
            });
            drop(slots);
            let Some(l) = l else { return Finish::NotRunning };
            self.inner.tell(owner, l);
            return Finish::KeptOpen;
        }
        if self.end(owner, Some(CLOSE_GRACE)) {
            Finish::Closed
        } else {
            Finish::NotRunning
        }
    }

    /// Closes the window's browser whatever happened in it (the user asked); true when one was running.
    pub fn close(&self, owner: &str) -> bool {
        self.end(owner, Some(CLOSE_GRACE))
    }

    /// Debrief quits: every browser is closed (gracefully for `grace`, then by force).
    pub fn close_all(&self, grace: Duration) {
        let owners: Vec<String> = crate::sync::lock(&self.inner.slots).keys().cloned().collect();
        std::thread::scope(|s| {
            for o in &owners {
                s.spawn(move || self.end(o, Some(grace)));
            }
        });
    }

    pub fn kill_all(&self) {
        let owners: Vec<String> = crate::sync::lock(&self.inner.slots).keys().cloned().collect();
        for o in owners {
            self.end(&o, None);
        }
    }

    /// Waits (bounded) until the window's browser is in a phase `pred` accepts; the phase it ended in.
    #[cfg(test)]
    pub fn wait_for(&self, owner: &str, timeout: Duration, pred: impl Fn(Phase) -> bool) -> Phase {
        let end = Instant::now() + timeout;
        let mut slots = crate::sync::lock(&self.inner.slots);
        loop {
            let phase = slots.get(owner).map_or(Phase::None, |s| s.lifecycle().phase);
            let Some(left) = end.checked_duration_since(Instant::now()) else {
                return phase;
            };
            if pred(phase) {
                return phase;
            }
            slots = self.inner.changed.wait_timeout(slots, left).unwrap_or_else(|e| e.into_inner()).0;
        }
    }
}

impl Drop for Renderers {
    fn drop(&mut self) {
        self.kill_all();
    }
}

/// The machine's own browser when nothing is configured or installed. Under test only the pinned test browser
/// counts: a test run never launches the user's installed Chrome.
#[cfg(not(test))]
pub(super) fn fallback_browser() -> Option<chromium_driver::BrowserExe> {
    chromium_driver::find()
}

#[cfg(test)]
pub(super) fn fallback_browser() -> Option<chromium_driver::BrowserExe> {
    chromium_driver::find_for_tests()
}
