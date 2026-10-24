//! Update checks for the inference runtimes and models (`debrief_ai::updates` decides; this runs it). Plain Rust,
//! no Tauri: the clock, "is a chat running" and the effects (start a job, post a message, tell the windows)
//! are injected, so every rule is tested with fakes (`update_commands.rs` is the Tauri glue).
//!
//! - A check per kind runs when due (Settings → AI: Automatic / Ask / Manual and the interval), or on
//!   "Check now". Failures (offline, proxy, TLS interception) are quiet: logged, shown in Settings and retried
//!   with backoff.
//! - Automatic installs run as download jobs, never while a chat or another AI request runs (they wait and
//!   start at the next tick). Ask posts "Update / Later / Skip this version" once per version.
//! - An install keeps the previous version until the new one is verified: a runtime build goes to the pinned
//!   release's folder while the worker keeps using the older one; a model file is downloaded and verified
//!   next to the old one, which is replaced (and deleted) only afterwards, when no chat runs.

use super::core::AiCore;
use super::error::{AiCmdError, AiErrorCode, AiResult};
use debrief_ai::models::source::Progress;
use debrief_ai::runtimes::builds::is_build_id;
use debrief_ai::setup::SetupStage;
use debrief_ai::updates::record::is_update_key;
use debrief_ai::updates::{
    check_due, confirm_model, decide, model_candidates, runtime_candidates, Decision, Situation, UpdateCandidate, UpdateKind, UpdatePolicy,
    UpdatesRecord,
};
use debrief_ai::AiError;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

pub const RECORD_FILE: &str = "updates.json";
/// How often a model update that waits for a chat to end looks again.
const IDLE_POLL: Duration = Duration::from_millis(500);

/// Whether the user's work would be interrupted (a chat turn or another AI request runs).
pub type BusyProbe = Arc<dyn Fn() -> bool + Send + Sync>;

/// What a check or a decision does outside this service.
pub trait UpdateEffects: Send + Sync {
    /// Starts the install job of `update`; `Err` when it cannot start now (another install runs).
    fn install(&self, update: &UpdateCandidate) -> AiResult<()>;
    /// Posts the "Update / Later / Skip this version" message.
    fn announce(&self, update: &UpdateCandidate);
    /// The status changed (to every window).
    fn changed(&self, status: &AiUpdatesStatus);
}

/// One update as Settings shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateView {
    #[serde(flatten)]
    pub update: UpdateCandidate,
    pub skipped: bool,
    pub installing: bool,
    /// An automatic install waits for the running chat to end.
    pub waiting: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KindStatus {
    pub kind: UpdateKind,
    pub policy: UpdatePolicy,
    /// Epoch ms.
    pub last_check: Option<i64>,
    pub last_success: Option<i64>,
    /// Why the last check or update failed (quiet: shown here only).
    pub last_error: Option<String>,
    /// Offer "Download manually" (certificate interception, proxy sign-in, blocked download).
    pub manual_suggested: bool,
    /// Epoch ms of the next automatic check (none with Manual).
    pub next_check: Option<i64>,
    pub checking: bool,
    pub updates: Vec<UpdateView>,
}

/// What Settings → AI → Updates shows (`debrief://ai-updates`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiUpdatesStatus {
    pub interval_hours: u32,
    pub kinds: Vec<KindStatus>,
}

#[derive(Default)]
struct Inner {
    record: UpdatesRecord,
    found: BTreeMap<UpdateKind, Vec<UpdateCandidate>>,
    checking: Vec<UpdateKind>,
    /// The key of the update being installed (one at a time).
    installing: Option<String>,
    /// Automatic installs waiting for a chat to end.
    waiting: Vec<String>,
    /// Runtime builds an update replaced, deleted once the engine no longer runs them: (tag, id).
    retire: Vec<(String, String)>,
}

pub struct AiUpdates {
    core: Arc<AiCore>,
    record_path: Option<PathBuf>,
    busy: BusyProbe,
    now: Box<dyn Fn() -> i64 + Send + Sync>,
    effects: Box<dyn UpdateEffects>,
    inner: Mutex<Inner>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Failures the user can get around by downloading the files in the browser ("Download manually").
fn suggests_manual(code: AiErrorCode) -> bool {
    matches!(
        code,
        AiErrorCode::Tls | AiErrorCode::ProxyAuth | AiErrorCode::Proxy | AiErrorCode::HttpStatus
    )
}

/// The kind an update key belongs to.
pub fn kind_of_key(key: &str) -> Option<UpdateKind> {
    UpdateKind::parse(key.split(':').next()?)
}

/// Epoch seconds now.
pub fn system_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

impl AiUpdates {
    /// `record_dir` keeps `updates.json`; `now` is epoch seconds.
    pub fn new(
        core: Arc<AiCore>,
        record_dir: Option<PathBuf>,
        busy: BusyProbe,
        now: Box<dyn Fn() -> i64 + Send + Sync>,
        effects: Box<dyn UpdateEffects>,
    ) -> AiUpdates {
        let record_path = record_dir.map(|d| d.join(RECORD_FILE));
        let record = match &record_path {
            Some(p) => {
                let (r, warning) = UpdatesRecord::load(p);
                if let Some(w) = warning {
                    log::warn!("AI updates: {w}");
                }
                r
            }
            None => UpdatesRecord::default(),
        };
        AiUpdates {
            core,
            record_path,
            busy,
            now,
            effects,
            inner: Mutex::new(Inner {
                record,
                ..Inner::default()
            }),
        }
    }

    fn save(&self, inner: &Inner) {
        if let Some(p) = &self.record_path {
            if let Err(e) = inner.record.save(p) {
                log::warn!("AI updates: the update record could not be saved: {e}");
            }
        }
    }

    fn status_of(&self, inner: &Inner) -> AiUpdatesStatus {
        let s = self.core.settings();
        let interval = s.update_interval_secs() as i64;
        let kinds = UpdateKind::ALL
            .into_iter()
            .map(|kind| {
                let r = inner.record.kind(kind);
                let policy = s.update_policy(kind);
                let next_check = (policy != UpdatePolicy::Manual).then(|| {
                    r.next_retry
                        .or(r.last_check.map(|t| t.saturating_add(interval)))
                        .unwrap_or_else(|| (self.now)())
                });
                let updates = inner
                    .found
                    .get(&kind)
                    .into_iter()
                    .flatten()
                    .map(|u| UpdateView {
                        skipped: r.is_skipped(&u.key),
                        installing: inner.installing.as_deref() == Some(u.key.as_str()),
                        waiting: inner.waiting.contains(&u.key),
                        update: u.clone(),
                    })
                    .collect();
                KindStatus {
                    kind,
                    policy,
                    last_check: r.last_check.map(|t| t.saturating_mul(1000)),
                    last_success: r.last_success.map(|t| t.saturating_mul(1000)),
                    last_error: r.last_error.clone(),
                    manual_suggested: r.manual_suggested,
                    next_check: next_check.map(|t| t.saturating_mul(1000)),
                    checking: inner.checking.contains(&kind),
                    updates,
                }
            })
            .collect();
        AiUpdatesStatus {
            interval_hours: s.update_check_hours,
            kinds,
        }
    }

    pub fn status(&self) -> AiUpdatesStatus {
        self.status_of(&lock(&self.inner))
    }

    fn emit(&self) {
        let status = self.status();
        self.effects.changed(&status);
    }

    /// Looks for updates of `kind` now and acts on them by policy. Blocks (a model check asks the model host):
    /// call off the UI thread. A check of the same kind already running is not repeated.
    pub fn check(&self, kind: UpdateKind) -> AiUpdatesStatus {
        {
            let mut inner = lock(&self.inner);
            if inner.checking.contains(&kind) {
                return self.status_of(&inner);
            }
            inner.checking.push(kind);
        }
        self.emit();
        let found = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.core.update_candidates(kind)))
            .unwrap_or_else(|_| Err(AiCmdError::new(AiErrorCode::Engine, "the update check stopped unexpectedly")));
        let settings = self.core.settings();
        let policy = settings.update_policy(kind);
        let busy = (self.busy)();
        let now = (self.now)();
        let mut install = Vec::new();
        let mut announce = Vec::new();
        {
            let mut inner = lock(&self.inner);
            inner.checking.retain(|k| *k != kind);
            match found {
                Ok(list) => {
                    log::info!(Kind = kind.as_str(), Found = list.len(); "AI update check done");
                    let keys: Vec<String> = list.iter().map(|u| u.key.clone()).collect();
                    let installing = inner.installing.clone();
                    let rec = inner.record.kind_mut(kind);
                    rec.checked(now);
                    rec.retain_announced(&keys);
                    for u in &list {
                        if installing.as_deref() == Some(u.key.as_str()) {
                            continue;
                        }
                        let situation = Situation {
                            skipped: rec.is_skipped(&u.key),
                            announced: rec.is_announced(&u.key),
                            busy,
                        };
                        match decide(policy, situation) {
                            // `start` lets a deferred one wait for the chat to end.
                            Decision::Install | Decision::Defer => install.push(u.clone()),
                            Decision::Announce => {
                                rec.announce(&u.key);
                                announce.push(u.clone());
                            }
                            Decision::List => {}
                        }
                    }
                    inner.waiting.retain(|k| keys.contains(k));
                    inner.found.insert(kind, list);
                }
                Err(e) => {
                    // Quiet: offline, proxy and certificate problems are expected on some networks.
                    log::info!(Kind = kind.as_str(), Code = format!("{:?}", e.code).as_str(); "AI update check failed: {}", e.message);
                    let interval = settings.update_interval_secs();
                    inner
                        .record
                        .kind_mut(kind)
                        .failed(now, &e.message, suggests_manual(e.code), interval);
                }
            }
            self.save(&inner);
        }
        for u in &announce {
            self.effects.announce(u);
        }
        for u in &install {
            self.start(u, busy);
        }
        self.emit();
        self.status()
    }

    /// Starts installing `u`, or lets it wait while `busy` (a chat) or another install runs.
    fn start(&self, u: &UpdateCandidate, busy: bool) {
        {
            let mut inner = lock(&self.inner);
            if busy || inner.installing.is_some() {
                if !inner.waiting.contains(&u.key) {
                    inner.waiting.push(u.key.clone());
                }
                return;
            }
            inner.waiting.retain(|k| *k != u.key);
            inner.installing = Some(u.key.clone());
        }
        if let Err(e) = self.effects.install(u) {
            log::info!(Update = u.key.as_str(); "AI update waits: {}", e.message);
            let mut inner = lock(&self.inner);
            inner.installing = None;
            if !inner.waiting.contains(&u.key) {
                inner.waiting.push(u.key.clone());
            }
        }
    }

    fn find(&self, key: &str) -> AiResult<UpdateCandidate> {
        if !is_update_key(key) {
            return Err(AiCmdError::invalid("that is not an AI update"));
        }
        let inner = lock(&self.inner);
        inner
            .found
            .values()
            .flatten()
            .find(|u| u.key == key)
            .cloned()
            .ok_or_else(|| AiCmdError::not_found("that update is no longer offered; check for updates again"))
    }

    /// "Update": installs `key` now (a model switch still waits for a running chat).
    pub fn apply(&self, key: &str) -> AiResult<AiUpdatesStatus> {
        let u = self.find(key)?;
        if lock(&self.inner).installing.as_deref() == Some(key) {
            return Ok(self.status());
        }
        {
            let mut inner = lock(&self.inner);
            if inner.installing.is_some() {
                return Err(AiCmdError::busy("Another AI update is being installed; wait for it first."));
            }
            inner.installing = Some(u.key.clone());
            inner.waiting.retain(|k| *k != u.key);
        }
        if let Err(e) = self.effects.install(&u) {
            lock(&self.inner).installing = None;
            return Err(e);
        }
        self.emit();
        Ok(self.status())
    }

    /// "Skip this version": `key` is never announced or installed automatically again.
    pub fn skip(&self, key: &str) -> AiResult<AiUpdatesStatus> {
        self.with_record(key, |inner, kind| {
            inner.record.kind_mut(kind).skip(key);
            inner.waiting.retain(|k| k != key);
        })
    }

    /// "Later": announce `key` again at the next check.
    pub fn later(&self, key: &str) -> AiResult<AiUpdatesStatus> {
        self.with_record(key, |inner, kind| inner.record.kind_mut(kind).later(key))
    }

    fn with_record(&self, key: &str, f: impl FnOnce(&mut Inner, UpdateKind)) -> AiResult<AiUpdatesStatus> {
        let kind = kind_of_key(key)
            .filter(|_| is_update_key(key))
            .ok_or_else(|| AiCmdError::invalid("that is not an AI update"))?;
        {
            let mut inner = lock(&self.inner);
            f(&mut inner, kind);
            self.save(&inner);
        }
        self.emit();
        Ok(self.status())
    }

    /// An install job ended. A failure (other than a cancel) keeps the installed version, is shown in Settings
    /// and retried after the backoff; a replaced runtime build is retired once the engine no longer runs it.
    pub fn finished(&self, u: &UpdateCandidate, result: &AiResult<()>) {
        let now = (self.now)();
        let interval = self.core.settings().update_interval_secs();
        {
            let mut inner = lock(&self.inner);
            if inner.installing.as_deref() == Some(u.key.as_str()) {
                inner.installing = None;
            }
            let rec = inner.record.kind_mut(u.kind);
            match result {
                Ok(()) => {
                    rec.last_error = None;
                    rec.manual_suggested = false;
                    if let Some(list) = inner.found.get_mut(&u.kind) {
                        list.retain(|c| c.key != u.key);
                    }
                    if u.kind == UpdateKind::Runtime {
                        inner.retire.push((u.from.clone(), u.id.clone()));
                    }
                }
                Err(e) if e.code == AiErrorCode::Cancelled => {}
                Err(e) => {
                    log::warn!(Update = u.key.as_str(), Code = format!("{:?}", e.code).as_str(); "AI update failed: {}", e.message);
                    let message = format!(
                        "Updating {} failed: {} The installed version is kept.",
                        u.name,
                        sentence(&e.message)
                    );
                    rec.failed(now, &message, suggests_manual(e.code), interval);
                }
            }
            self.save(&inner);
        }
        self.retire();
        self.emit();
    }

    /// Deletes runtime builds an update replaced, once the engine does not run a model (it may use them).
    fn retire(&self) {
        let pending = std::mem::take(&mut lock(&self.inner).retire);
        let mut left = Vec::new();
        for (tag, id) in pending {
            if !self.core.retire_runtime(&tag, &id) {
                left.push((tag, id));
            }
        }
        lock(&self.inner).retire.extend(left);
    }

    /// The scheduler's beat (every minute): due checks, waiting automatic installs, builds to retire.
    pub fn tick(&self) {
        // While Pitwall answers, the built-in engine's runtime and models are not updated (no second copy, ADR 0059).
        if self.core.built_in_availability().is_err() || self.core.uses_pitwall() {
            return;
        }
        let s = self.core.settings();
        let now = (self.now)();
        for kind in UpdateKind::ALL {
            let (last, retry) = {
                let inner = lock(&self.inner);
                let r = inner.record.kind(kind);
                (r.last_check, r.next_retry)
            };
            if check_due(s.update_policy(kind), now, last, retry, s.update_interval_secs()) {
                self.check(kind);
            }
        }
        let waiting = std::mem::take(&mut lock(&self.inner).waiting);
        if !waiting.is_empty() {
            let busy = (self.busy)();
            for key in waiting {
                match self.find(&key) {
                    Ok(u) if !lock(&self.inner).record.kind(u.kind).is_skipped(&key) => self.start(&u, busy),
                    _ => {}
                }
            }
            self.emit();
        }
        self.retire();
    }
}

/// `s` as a sentence (capitalised, ending in a full stop).
fn sentence(s: &str) -> String {
    let s = s.trim().trim_end_matches('.');
    let mut c = s.chars();
    match c.next() {
        Some(f) => format!("{}{}.", f.to_uppercase(), c.as_str()),
        None => String::new(),
    }
}

impl AiCore {
    /// The updates of `kind` the pinned catalogs offer for what is installed. A model update is confirmed with
    /// the model host first (no download): a network failure fails the check (retried later); a host that serves
    /// another file than the pinned one drops that update.
    pub fn update_candidates(&self, kind: UpdateKind) -> AiResult<Vec<UpdateCandidate>> {
        self.built_in_availability()?;
        match kind {
            UpdateKind::Runtime => {
                let rt = &self.deps.runtimes;
                Ok(runtime_candidates(&rt.store.list(&rt.catalog.tag), &rt.catalog, &rt.platform))
            }
            UpdateKind::Model => {
                let list = model_candidates(&self.installed_files(), self.catalog());
                if list.is_empty() {
                    return Ok(list);
                }
                let source = (self.deps.source)(&self.settings())?;
                let mut out = Vec::new();
                for u in list {
                    let Some(m) = self.catalog().find(&u.id) else { continue };
                    match confirm_model(source.as_ref(), m) {
                        Ok(()) => out.push(u),
                        Err(AiError::Integrity(why)) => log::warn!(Model = u.id.as_str(); "AI model update not offered: {why}"),
                        Err(e) => return Err(e.into()),
                    }
                }
                Ok(out)
            }
        }
    }

    /// Installs a model update: downloads and verifies the new file next to the old one (which stays usable
    /// when anything fails), then, when the old one is the active model, switches to the new one once no chat
    /// runs (`busy`), and deletes the old file. Blocks; call between [`AiCore::begin_setup`] and
    /// [`AiCore::end_setup`] (`progress` cancels it).
    pub fn install_model_update(
        &self,
        u: &UpdateCandidate,
        progress: &Progress,
        on_stage: &mut dyn FnMut(SetupStage),
        busy: &dyn Fn() -> bool,
    ) -> AiResult<()> {
        let entry = self
            .catalog()
            .find(&u.id)
            .filter(|m| m.file == u.to && m.supersedes.contains(&u.from))
            .cloned()
            .ok_or_else(|| AiCmdError::not_found("that model update is no longer offered"))?;
        self.with_setup(|s| s.download(&entry, progress, on_stage))?;
        if self.settings().active_model.as_deref() == Some(u.from.as_str()) {
            while busy() {
                if progress.is_cancelled() {
                    return Err(AiError::Cancelled.into());
                }
                std::thread::sleep(IDLE_POLL);
            }
            self.update_settings(|s| s.active_model = Some(entry.file.clone()))?;
            self.install_active(None);
        }
        if let Err(e) = self.store().delete(&u.from) {
            log::warn!(Model = u.from.as_str(); "AI: the replaced model file was not deleted: {e}");
        }
        Ok(())
    }

    /// Deletes build `id` of the older release `tag` once an update replaced it. False (nothing done) while a
    /// model is loaded: the running engine may use it; it is tried again later.
    pub fn retire_runtime(&self, tag: &str, id: &str) -> bool {
        let rt = &self.deps.runtimes;
        if !is_build_id(tag) || !is_build_id(id) || tag == rt.catalog.tag {
            return true;
        }
        if self.engine_running() {
            return false;
        }
        match rt.store.delete(tag, id) {
            Ok(()) | Err(AiError::NotFound(_)) => true,
            Err(e) => {
                log::info!(Build = id, Tag = tag; "AI: the replaced runtime build is still in use: {e}");
                false
            }
        }
    }
}
