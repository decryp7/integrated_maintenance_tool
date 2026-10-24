//! Pitwall as Debrief's AI engine (ADR 0059), following Pitwall's app integration contract (Pitwall ADR 0009):
//! discovery, pairing with the user's approval, a key in the OS credential store, the OpenAI-compatible chat API.
//!
//! [`Pitwall`] keeps what Debrief knows about the server ([`PitwallView`]), runs the pairing, and makes the
//! [`PitwallProvider`] the features use. Every call it makes is blocking and bounded: callers run it off the UI thread.

pub mod client;
pub mod discovery;
pub mod fake;
pub mod http;
pub mod keys;
pub mod provider;

pub use client::{Client, PitwallError, API_VERSION};
pub use discovery::Locator;
pub use keys::{KeyStore, MemoryKeys};
pub use provider::{PitwallProvider, Priority};

use crate::error::{AiError, Result};
use client::PairPoll;
use discovery::Found;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The preset name a user can define in Pitwall to decide what serves Debrief (Pitwall ADR 0014).
pub const PRESET: &str = "debrief";

/// What Debrief found about Pitwall on this computer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum FoundKind {
    /// Never run here.
    #[default]
    Absent,
    /// Its data folder exists but no server answers.
    Installed,
    Running,
    /// Running, but it speaks another client contract version.
    Incompatible,
}

/// A pairing that waits for the user's approval in Pitwall.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingView {
    /// The six-digit code the user compares in Pitwall.
    pub code: String,
    pub expires_at_ms: u64,
}

/// Pitwall as the UI sees it (wire contract: `PitwallStatus` in `ai.types.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PitwallView {
    pub found: FoundKind,
    pub version: Option<String>,
    /// Debrief holds a key.
    pub paired: bool,
    pub pairing: Option<PairingView>,
    /// The model Pitwall picked for Debrief.
    pub model: Option<String>,
    /// Running, compatible, paired and a model fits.
    pub usable: bool,
    /// One plain sentence when something is wrong or just happened.
    pub problem: Option<String>,
}

/// What the owner hears.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PitwallEvent {
    /// The view changed.
    Changed,
    /// A pairing was approved; `use_for_debrief`: the user started it from "Use Pitwall".
    Paired { use_for_debrief: bool },
}

pub type Listener = Arc<dyn Fn(PitwallEvent) + Send + Sync>;

/// What Debrief asks of Pitwall's models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Needs {
    /// The context Debrief's prompts are sized for (Settings ▸ AI ▸ Context length).
    pub context_tokens: usize,
}

impl Default for Needs {
    fn default() -> Self {
        Needs { context_tokens: 8192 }
    }
}

struct Pairing {
    id: String,
    view: PairingView,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
struct Inner {
    view: PitwallView,
    port: Option<u16>,
    spec: Option<String>,
    context: usize,
    vision: bool,
    pairing: Option<Pairing>,
    /// Pitwall answered a request of this session.
    served: bool,
}

/// The tunables of the client (tests shorten them).
#[derive(Debug, Clone, Copy)]
pub struct Tuning {
    pub poll_every: Duration,
    pub chat: http::Timeouts,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning {
            poll_every: Duration::from_secs(1),
            chat: http::Timeouts::CHAT,
        }
    }
}

/// Debrief's side of Pitwall.
pub struct Pitwall {
    locator: Locator,
    keys: Arc<dyn KeyStore>,
    inner: Mutex<Inner>,
    listener: Mutex<Option<Listener>>,
    needs: Mutex<Needs>,
    tuning: Tuning,
    me: Weak<Pitwall>,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

impl Pitwall {
    pub fn new(locator: Locator, keys: Arc<dyn KeyStore>, tuning: Tuning) -> Arc<Pitwall> {
        Arc::new_cyclic(|me| Pitwall {
            locator,
            keys,
            inner: Mutex::default(),
            listener: Mutex::default(),
            needs: Mutex::default(),
            tuning,
            me: me.clone(),
        })
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn set_listener(&self, l: Listener) {
        *self.listener.lock().unwrap_or_else(|p| p.into_inner()) = Some(l);
    }

    pub fn set_needs(&self, n: Needs) {
        *self.needs.lock().unwrap_or_else(|p| p.into_inner()) = n;
    }

    fn tell(&self, e: PitwallEvent) {
        let l = self.listener.lock().unwrap_or_else(|p| p.into_inner()).clone();
        if let Some(l) = l {
            l(e);
        }
    }

    /// The last view (no network).
    pub fn view(&self) -> PitwallView {
        let i = self.inner();
        let mut v = i.view.clone();
        v.pairing = i.pairing.as_ref().map(|p| p.view.clone());
        v
    }

    /// Whether Pitwall answered a request in this session.
    pub fn served(&self) -> bool {
        self.inner().served
    }

    /// Looks at Pitwall now: discovery, health, the key and a model for Debrief. Blocking (a few quick loopback calls).
    pub fn probe(&self) -> PitwallView {
        let before = self.view();
        let (view, port, spec, context, vision) = self.look();
        {
            let mut i = self.inner();
            // A problem from a pairing or a failed request stays until the next success.
            let keep = i.view.problem.clone().filter(|_| !view.usable && view.problem.is_none());
            i.view = PitwallView {
                problem: view.problem.or(keep),
                ..view
            };
            (i.port, i.spec, i.context, i.vision) = (port, spec, context, vision);
        }
        let after = self.view();
        if after != before {
            self.tell(PitwallEvent::Changed);
        }
        after
    }

    #[allow(clippy::type_complexity)]
    fn look(&self) -> (PitwallView, Option<u16>, Option<String>, usize, bool) {
        let mut v = PitwallView::default();
        let (port, installed) = match self.locator.look() {
            Found::Absent => return (v, None, None, 0, false),
            Found::Discovered(d) if d.api_version != API_VERSION => {
                v.found = FoundKind::Incompatible;
                v.version = Some(d.version.clone());
                v.problem = Some(incompatible(&d.version, d.api_version));
                return (v, None, None, 0, false);
            }
            Found::Discovered(d) => (Some(d.port), true),
            Found::Guess { port, installed, .. } => (port, installed),
        };
        v.found = if installed { FoundKind::Installed } else { FoundKind::Absent };
        let Some(port) = port else { return (v, None, None, 0, false) };
        let anon = Client::new(port, None);
        let health = match anon.health() {
            Ok(h) => h,
            // Something else on the port, or nothing: not Pitwall.
            Err(_) => return (v, None, None, 0, false),
        };
        v.version = Some(health.version.chars().take(64).collect());
        if health.api_version != API_VERSION {
            v.found = FoundKind::Incompatible;
            v.problem = Some(incompatible(&health.version, health.api_version));
            return (v, None, None, 0, false);
        }
        v.found = FoundKind::Running;
        let Some(key) = self.keys.get() else {
            return (v, Some(port), None, 0, false);
        };
        v.paired = true;
        if !health.serving() {
            v.problem = Some(format!(
                "Pitwall is {} at the moment. Resume serving in Pitwall, or use the built-in engine.",
                if health.serving == "stopped" { "stopped" } else { "paused" }
            ));
            return (v, Some(port), None, 0, false);
        }
        let client = Client::new(port, Some(key));
        let needs = *self.needs.lock().unwrap_or_else(|p| p.into_inner());
        let found_model = client.model_ids().and_then(|ids| {
            let spec = if ids.iter().any(|i| i == PRESET) {
                PRESET.to_string()
            } else {
                format!("auto:chat,{}k", needs.context_tokens.div_ceil(1024).max(1))
            };
            client.select(&spec).map(|s| (spec, s.model))
        });
        match found_model {
            Ok((spec, model)) => {
                let info = client.models().ok().and_then(|ms| ms.into_iter().find(|m| m.id == model));
                let context = info
                    .as_ref()
                    .and_then(|m| m.context_max)
                    .map_or(needs.context_tokens, |c| (c as usize).min(needs.context_tokens).max(1024));
                let vision = info.is_some_and(|m| m.capabilities.iter().any(|c| c == "vision"));
                v.model = Some(model.chars().take(200).collect());
                v.usable = true;
                (v, Some(port), Some(spec), context, vision)
            }
            Err(e) => {
                if matches!(e, PitwallError::Unauthorized(_)) {
                    let _ = self.keys.delete();
                    v.paired = false;
                }
                v.problem = Some(sentence(&e.to_string()));
                (v, Some(port), None, 0, false)
            }
        }
    }

    /// The chat port for the features, when Pitwall is usable.
    pub fn provider(&self, priority: Priority) -> Result<PitwallProvider> {
        let i = self.inner();
        let (Some(port), Some(spec), true) = (i.port, i.spec.clone(), i.view.usable) else {
            return Err(AiError::Engine(i.view.problem.clone().unwrap_or_else(|| {
                "Pitwall is not ready for Debrief. Open Pitwall, or use the built-in engine (Settings ▸ AI).".into()
            })));
        };
        let me = self.me.clone();
        Ok(PitwallProvider {
            client: Client::new(port, self.keys.get()),
            auto: spec.starts_with("auto:"),
            spec,
            model: i.view.model.clone().unwrap_or_default(),
            context: i.context,
            vision: i.vision,
            priority,
            timeouts: self.tuning.chat,
            outcome: Some(Arc::new(move |r| {
                if let Some(p) = me.upgrade() {
                    p.outcome(r);
                }
            })),
        })
    }

    /// Records how a request went: served, or Pitwall is gone (stopped, key revoked).
    fn outcome(&self, r: std::result::Result<&client::StreamEnd, &PitwallError>) {
        let changed = {
            let mut i = self.inner();
            let before = i.view.clone();
            match r {
                Ok(end) => {
                    i.served = true;
                    if let Some(m) = &end.model {
                        i.view.model = Some(m.clone());
                    }
                    i.view.problem = None;
                }
                Err(e) if e.is_gone() || matches!(e, PitwallError::Unauthorized(_)) => {
                    i.view.usable = false;
                    i.view.problem = Some(sentence(&e.to_string()));
                    if matches!(e, PitwallError::Unauthorized(_)) {
                        let _ = self.keys.delete();
                        i.view.paired = false;
                    }
                }
                Err(_) => {}
            }
            i.view != before
        };
        if changed {
            self.tell(PitwallEvent::Changed);
        }
    }

    /// Starts pairing (after the user chose to connect): Pitwall shows a request with a code, Debrief shows the same
    /// code and polls in the background until the user approves or denies in Pitwall, it expires or [`Self::cancel_pairing`].
    /// Already paired and usable: nothing to do. Blocking for one quick call.
    pub fn connect(&self, use_for_debrief: bool) -> Result<PitwallView> {
        let view = self.probe();
        if view.usable {
            if use_for_debrief {
                self.tell(PitwallEvent::Paired { use_for_debrief });
            }
            return Ok(view);
        }
        if view.pairing.is_some() {
            return Ok(view);
        }
        let port = match (view.found, self.inner().port) {
            (FoundKind::Running, Some(p)) => p,
            (FoundKind::Incompatible, _) => return Err(AiError::Refused(view.problem.unwrap_or_default())),
            _ => {
                return Err(AiError::Engine(
                    "Pitwall is not running. Open Pitwall, then choose Connect again.".into(),
                ))
            }
        };
        // A stored key Pitwall no longer accepts is replaced by the new one.
        let start = Client::new(port, None).pair().map_err(AiError::from)?;
        let cancel = Arc::new(AtomicBool::new(false));
        let expires_at_ms = now_ms() + start.expires_in_secs.min(3600) * 1000;
        {
            let mut i = self.inner();
            i.view.problem = None;
            i.pairing = Some(Pairing {
                id: start.pairing_id.clone(),
                view: PairingView {
                    code: start.code.clone(),
                    expires_at_ms,
                },
                cancel: cancel.clone(),
            });
        }
        self.tell(PitwallEvent::Changed);
        let me = self.me.clone();
        let every = self.tuning.poll_every;
        std::thread::Builder::new()
            .name("pitwall-pairing".into())
            .spawn(move || {
                // The poll ends with an outcome, a cancel, the expiry or the owner going away.
                loop {
                    std::thread::sleep(every);
                    let Some(p) = me.upgrade() else { return };
                    if cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    let outcome = if now_ms() > expires_at_ms {
                        Some(Err(
                            "The pairing request expired before it was approved in Pitwall. Choose Connect to try again.".to_string(),
                        ))
                    } else {
                        match Client::new(port, None).poll(&start.pairing_id) {
                            Ok(PairPoll::Pending) => None,
                            Ok(PairPoll::Approved(Some(key))) => Some(p.keys.set(&key).map_err(|e| {
                                format!("Pitwall approved Debrief, but the key could not be stored in the credential store ({e}).")
                            })),
                            Ok(PairPoll::Approved(None)) => Some(Err(
                                "Pitwall says the key was already delivered. Choose Connect to pair again.".into(),
                            )),
                            Ok(PairPoll::Denied) => Some(Err(
                                "Pitwall denied Debrief's request (or it expired). Choose Connect to try again.".into(),
                            )),
                            Err(e) if e.is_gone() => Some(Err(sentence(&e.to_string()))),
                            Err(_) => None,
                        }
                    };
                    let Some(outcome) = outcome else { continue };
                    p.finish_pairing(&start.pairing_id, outcome, use_for_debrief);
                    return;
                }
            })
            .map_err(|e| AiError::Engine(format!("the pairing could not start: {e}")))?;
        Ok(self.view())
    }

    fn finish_pairing(&self, id: &str, outcome: std::result::Result<(), String>, use_for_debrief: bool) {
        {
            let mut i = self.inner();
            if i.pairing.as_ref().is_none_or(|p| p.id != id) {
                return;
            }
            i.pairing = None;
            if let Err(m) = &outcome {
                i.view.problem = Some(m.clone());
            }
        }
        if outcome.is_ok() {
            self.probe();
            self.tell(PitwallEvent::Paired { use_for_debrief });
        }
        self.tell(PitwallEvent::Changed);
    }

    /// Stops a waiting pairing; false when none waits.
    pub fn cancel_pairing(&self) -> bool {
        let p = self.inner().pairing.take();
        match p {
            Some(p) => {
                p.cancel.store(true, Ordering::Relaxed);
                self.tell(PitwallEvent::Changed);
                true
            }
            None => false,
        }
    }

    /// Deletes Debrief's key (Settings ▸ AI ▸ Forget Pitwall).
    pub fn forget(&self) -> Result<PitwallView> {
        self.keys.delete().map_err(AiError::Io)?;
        {
            let mut i = self.inner();
            i.view.problem = None;
        }
        Ok(self.probe())
    }
}

fn incompatible(version: &str, api: u32) -> String {
    if api > API_VERSION {
        format!("Pitwall {version} speaks a newer connection version ({api}) than this Debrief ({API_VERSION}). Update Debrief to use it.")
    } else {
        format!("Pitwall {version} speaks an older connection version ({api}) than this Debrief needs ({API_VERSION}). Update Pitwall to use it.")
    }
}

/// `s` as one sentence.
fn sentence(s: &str) -> String {
    let t = s.trim().trim_end_matches('.');
    let mut c = t.chars();
    match c.next() {
        Some(f) => format!("{}{}.", f.to_uppercase(), c.as_str()),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests;
