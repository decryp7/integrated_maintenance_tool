//! Crash containment for plugin code.
//!
//! Every contribution a plugin registers is wrapped in a guarded adapter
//! (decorator) that turns a panic into an error attributed to the plugin,
//! records it in [`Health`], and stops calling a plugin once it is
//! quarantined. Plugins need no changes to benefit.
//!
//! Calls are also *admitted*: a plugin that is inactive (a dependency is
//! unavailable, see [`crate::plugin`]), quarantined or hung is refused with a
//! message saying why. Each call kind has a time [`Budgets`] entry; a call
//! that exceeds it is detected by the watchdog
//! ([`crate::plugin::watchdog`]), which marks the plugin hung, counts a
//! failure and makes further calls fail fast until the stuck call returns.
//!
//! In-process guarding cannot pre-empt a running call, nor survive a stack
//! overflow, an abort or running out of memory: that needs the out-of-process
//! plugin host (ADR 0002). [`Guard::call_detached`] gets closest in process:
//! the call runs on a helper thread and is abandoned (its result discarded)
//! when it overruns.

use crate::api::WorkspaceSummary;
use crate::export::{EventSink, ExportRow, Exporter};
use crate::importers::{BinaryImporter, ImportCtx, Imported, Importer};
use crate::model::Record;
use crate::plugin::{Collector, RecordTransform, TransformCtx, Verdict, WorkspaceHook};
use crate::profile::{CompiledProfile, Profile};
use anyhow::{anyhow, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

/// Log target of guard events (contained crashes, quarantines, overruns).
pub const LOG_TARGET: &str = "imt_core::guard";

/// A plugin is quarantined (no longer called) after this many crashes.
pub const QUARANTINE_AFTER: u32 = 3;

/// Most notes kept per plugin.
pub const MAX_NOTES: usize = 20;

/// Runs `f`, turning a panic into an error ("{what} crashed: ...").
pub fn catch<T>(what: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(payload) => Err(anyhow!("{what} crashed: {}", panic_message(payload.as_ref()))),
    }
}

/// The message of a panic payload.
pub fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

/// What kind of plugin code a guarded call runs; selects its time budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallKind {
    Sniff,
    Import,
    /// Live tail: parsing appended text.
    Tail,
    /// One record through a pipeline transform.
    Transform,
    Hook,
    Export,
    Collector,
    /// One run of an annotator over the workspace.
    Annotate,
    /// A call through a [`crate::plugin::Service`] handle or contribution.
    Service,
    /// Delivery of one event to a subscriber.
    Event,
    /// `activate` / `deactivate`; allowed while the plugin is inactive.
    Lifecycle,
    /// One operation of a [`crate::plugin::tools::Tool`]; its budget is
    /// declared per operation by the tool, not in [`Budgets`].
    Tool,
}

impl CallKind {
    pub fn label(self) -> &'static str {
        match self {
            CallKind::Sniff => "sniff",
            CallKind::Import => "import",
            CallKind::Tail => "import_tail",
            CallKind::Transform => "transform",
            CallKind::Hook => "hook",
            CallKind::Export => "export",
            CallKind::Collector => "collector",
            CallKind::Annotate => "annotate",
            CallKind::Service => "service",
            CallKind::Event => "event",
            CallKind::Lifecycle => "lifecycle",
            CallKind::Tool => "tool",
        }
    }
}

/// Time budget per call kind; `None` means unlimited (exports stream for as
/// long as the data takes, collectors run until stopped).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budgets {
    pub sniff: Option<Duration>,
    pub import: Option<Duration>,
    pub tail: Option<Duration>,
    pub transform: Option<Duration>,
    pub hook: Option<Duration>,
    pub export: Option<Duration>,
    pub collector: Option<Duration>,
    pub annotate: Option<Duration>,
    pub service: Option<Duration>,
    pub event: Option<Duration>,
    pub lifecycle: Option<Duration>,
}

impl Default for Budgets {
    fn default() -> Self {
        let s = Duration::from_secs;
        Budgets {
            sniff: Some(s(5)),
            import: Some(s(120)),
            tail: Some(s(10)),
            // Per record; generous so that scheduling hiccups on a loaded
            // machine are not mistaken for a hang.
            transform: Some(Duration::from_millis(250)),
            hook: Some(s(5)),
            export: None,
            collector: None,
            annotate: Some(s(10)),
            service: Some(s(10)),
            event: Some(s(5)),
            lifecycle: Some(s(10)),
        }
    }
}

impl Budgets {
    /// No budget for any kind (the watchdog never fires).
    pub fn unlimited() -> Self {
        Budgets {
            sniff: None,
            import: None,
            tail: None,
            transform: None,
            hook: None,
            export: None,
            collector: None,
            annotate: None,
            service: None,
            event: None,
            lifecycle: None,
        }
    }

    pub fn get(&self, kind: CallKind) -> Option<Duration> {
        match kind {
            CallKind::Sniff => self.sniff,
            CallKind::Import => self.import,
            CallKind::Tail => self.tail,
            CallKind::Transform => self.transform,
            CallKind::Hook => self.hook,
            CallKind::Export => self.export,
            CallKind::Collector => self.collector,
            CallKind::Annotate => self.annotate,
            CallKind::Service => self.service,
            CallKind::Event => self.event,
            CallKind::Lifecycle => self.lifecycle,
            // Declared per operation by the tool (`OpSpec::budget`).
            CallKind::Tool => None,
        }
    }
}

/// "120 s", "1.5 s", "250 ms".
pub(crate) fn fmt_duration(d: Duration) -> String {
    if d >= Duration::from_secs(1) {
        let s = d.as_secs_f64();
        if s.fract() == 0.0 {
            format!("{s:.0} s")
        } else {
            format!("{s:.1} s")
        }
    } else {
        format!("{} ms", d.as_millis())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginHealth {
    pub failures: u32,
    pub last_error: Option<String>,
    pub quarantined: bool,
    /// A call is running past its time budget right now.
    pub hung: bool,
}

#[derive(Default)]
struct Ledger {
    plugins: HashMap<String, PluginHealth>,
    /// Calls past their budget that have not returned, per plugin.
    hung: HashMap<String, u32>,
    /// Plugins whose calls are refused, with the reason (inactive, disabled at run time).
    blocked: HashMap<String, String>,
    /// Non-fatal problems per plugin (e.g. refused marks), newest last.
    notes: HashMap<String, Vec<String>>,
    budgets: Budgets,
}

type Observer = Arc<dyn Fn() + Send + Sync>;

/// Failure counts per plugin id, shared by every guarded adapter, plus the
/// admission state the plugin host maintains (inactive plugins, budgets).
#[derive(Default)]
pub struct Health {
    state: Mutex<Ledger>,
    /// Told (outside the lock) when a quarantine starts or is lifted.
    observer: Mutex<Option<Observer>>,
}

impl std::fmt::Debug for Health {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.with(|l| {
            f.debug_struct("Health")
                .field("plugins", &l.plugins)
                .field("blocked", &l.blocked)
                .finish()
        })
    }
}

impl Health {
    fn with<T>(&self, f: impl FnOnce(&mut Ledger) -> T) -> T {
        let mut ledger = self.state.lock().unwrap_or_else(|p| p.into_inner());
        f(&mut ledger)
    }

    fn notify(&self) {
        let observer = self.observer.lock().unwrap_or_else(|p| p.into_inner()).clone();
        if let Some(o) = observer {
            o();
        }
    }

    /// Records a crash; quarantines the plugin after [`QUARANTINE_AFTER`] of them.
    pub fn fail(&self, plugin: &str, error: &str) {
        let quarantined = self.with(|l| Self::count(l, plugin, error));
        log::error!(target: LOG_TARGET, Plugin = plugin, Contained = true; "Plugin {plugin} crashed (contained): {error}");
        if quarantined {
            log::error!(target: LOG_TARGET, Plugin = plugin, Crashes = QUARANTINE_AFTER; "Plugin {plugin} quarantined after repeated crashes");
            self.notify();
        }
    }

    /// Counts a failure; true when it starts a quarantine.
    fn count(l: &mut Ledger, plugin: &str, error: &str) -> bool {
        let h = l.plugins.entry(plugin.to_string()).or_default();
        h.failures += 1;
        h.last_error = Some(error.chars().take(500).collect());
        let started = !h.quarantined && h.failures >= QUARANTINE_AFTER;
        h.quarantined |= started;
        started
    }

    pub fn is_quarantined(&self, plugin: &str) -> bool {
        self.with(|l| l.plugins.get(plugin).is_some_and(|h| h.quarantined))
    }

    pub fn get(&self, plugin: &str) -> PluginHealth {
        self.with(|l| {
            let mut h = l.plugins.get(plugin).cloned().unwrap_or_default();
            h.hung = l.hung.get(plugin).is_some_and(|n| *n > 0);
            h
        })
    }

    /// Forgets a plugin's failures and lifts its quarantine (and hung state).
    pub fn reset(&self, plugin: &str) {
        self.with(|l| {
            l.plugins.remove(plugin);
            l.hung.remove(plugin);
            l.notes.remove(plugin);
        });
        self.notify();
    }

    /// Records a non-fatal problem of a plugin (kept once, at most [`MAX_NOTES`]).
    pub fn note(&self, plugin: &str, note: &str) {
        self.with(|l| {
            let notes = l.notes.entry(plugin.to_string()).or_default();
            if !notes.iter().any(|n| n == note) {
                if notes.len() >= MAX_NOTES {
                    notes.remove(0);
                }
                notes.push(note.chars().take(500).collect());
            }
        });
    }

    /// Non-fatal problems recorded with [`Health::note`].
    pub fn notes(&self, plugin: &str) -> Vec<String> {
        self.with(|l| l.notes.get(plugin).cloned().unwrap_or_default())
    }

    /// The time budgets of guarded calls.
    pub fn budgets(&self) -> Budgets {
        self.with(|l| l.budgets)
    }

    pub fn set_budgets(&self, budgets: Budgets) {
        self.with(|l| l.budgets = budgets);
    }

    pub(crate) fn set_observer(&self, observer: Observer) {
        *self.observer.lock().unwrap_or_else(|p| p.into_inner()) = Some(observer);
    }

    /// Plugins whose calls are refused (other than quarantine), with the reason.
    pub(crate) fn set_blocked(&self, blocked: HashMap<String, String>) {
        self.with(|l| l.blocked = blocked);
    }

    /// A call of `plugin` overran its budget: counts a failure and refuses
    /// further calls until [`Health::unhang`].
    pub(crate) fn mark_hung(&self, plugin: &str, error: &str) {
        let quarantined = self.with(|l| {
            *l.hung.entry(plugin.to_string()).or_default() += 1;
            Self::count(l, plugin, error)
        });
        log::error!(target: LOG_TARGET, Plugin = plugin, Hung = true; "Plugin {plugin} overran its time budget: {error}");
        if quarantined {
            log::error!(target: LOG_TARGET, Plugin = plugin, Crashes = QUARANTINE_AFTER; "Plugin {plugin} quarantined after repeated failures");
            self.notify();
        }
    }

    /// A call that overran its budget has returned (or was abandoned and finished).
    pub(crate) fn unhang(&self, plugin: &str) {
        self.with(|l| {
            if let Some(n) = l.hung.get_mut(plugin) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    l.hung.remove(plugin);
                }
            }
        });
    }

    /// Whether `plugin` may be called now; `Err` says why not (quarantined,
    /// hung or inactive).
    pub fn availability(&self, plugin: &str) -> std::result::Result<(), String> {
        self.admit(plugin, CallKind::Service).map(|_| ()).map_err(|e| e.to_string())
    }

    /// Whether `plugin` may run a call of `kind`; its budget if so.
    fn admit(&self, plugin: &str, kind: CallKind) -> Result<Option<Duration>> {
        self.with(|l| {
            if l.plugins.get(plugin).is_some_and(|h| h.quarantined) {
                return Err(quarantined_error(plugin));
            }
            if l.hung.get(plugin).is_some_and(|n| *n > 0) {
                let why = l.plugins.get(plugin).and_then(|h| h.last_error.clone()).unwrap_or_default();
                return Err(anyhow!("plugin \"{plugin}\" is not responding: {why}"));
            }
            if kind != CallKind::Lifecycle {
                if let Some(reason) = l.blocked.get(plugin) {
                    return Err(anyhow!("plugin \"{plugin}\" is inactive: {reason}"));
                }
            }
            Ok(l.budgets.get(kind))
        })
    }
}

fn quarantined_error(plugin: &str) -> anyhow::Error {
    anyhow!("plugin \"{plugin}\" is disabled after repeated crashes (re-enable it in the Plugins panel)")
}

/// Owner and health shared by the adapters of one plugin.
#[derive(Clone)]
pub struct Guard {
    owner: Arc<str>,
    health: Arc<Health>,
}

/// Progress of a detached call, shared by the caller and the helper thread.
#[derive(PartialEq)]
enum Detached {
    Running,
    Abandoned,
    Finished,
}

impl Guard {
    pub fn new(owner: &str, health: Arc<Health>) -> Guard {
        Guard {
            owner: Arc::from(owner),
            health,
        }
    }

    /// The plugin id this guard attributes failures to.
    pub fn owner(&self) -> &str {
        &self.owner
    }

    pub fn health(&self) -> &Arc<Health> {
        &self.health
    }

    /// Runs plugin code: refused while quarantined or hung; a panic becomes
    /// an error ("plugin \"id\" (what) crashed: ...") and counts towards
    /// quarantine; ordinary errors pass through uncounted. No time budget and
    /// allowed while inactive (used for settings, which stay readable).
    pub(crate) fn run<T>(&self, what: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
        self.health.admit(&self.owner, CallKind::Lifecycle)?;
        self.catch(what, f)
    }

    /// Runs plugin code of `kind` on this thread: refused while the plugin is
    /// quarantined, hung or inactive; watched against the kind's budget; a
    /// panic becomes an error counted towards quarantine.
    pub fn call<T>(&self, kind: CallKind, what: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
        let budget = self.health.admit(&self.owner, kind)?;
        let _frame = budget.map(|b| crate::plugin::watchdog::enter(self, kind, b));
        self.catch(what, f)
    }

    /// Like [`Guard::call`], but runs `f` on a helper thread and stops
    /// waiting when the budget is exceeded: the plugin is marked hung (a
    /// failure), an error is returned, and the thread is left to finish with
    /// its result discarded. Without a budget it runs on this thread.
    pub fn call_detached<T: Send + 'static>(
        &self,
        kind: CallKind,
        what: &str,
        f: impl FnOnce() -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let budget = self.health.admit(&self.owner, kind)?;
        self.detached(kind, budget, what, f)
    }

    /// [`Guard::call_detached`] with an explicit budget instead of the
    /// kind's (used for tool operations, which declare their own).
    pub fn call_detached_within<T: Send + 'static>(
        &self,
        kind: CallKind,
        budget: Option<Duration>,
        what: &str,
        f: impl FnOnce() -> Result<T> + Send + 'static,
    ) -> Result<T> {
        self.health.admit(&self.owner, kind)?;
        self.detached(kind, budget, what, f)
    }

    fn detached<T: Send + 'static>(
        &self,
        kind: CallKind,
        budget: Option<Duration>,
        what: &str,
        f: impl FnOnce() -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let Some(budget) = budget else {
            return self.catch(what, f);
        };
        let (tx, rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(Detached::Running));
        let helper = self.clone();
        let what_owned = what.to_string();
        let shared = state.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("imt-plugin-{}", kind.label()))
            .spawn(move || {
                let result = helper.catch(&what_owned, f);
                let mut s = shared.lock().unwrap_or_else(|p| p.into_inner());
                if *s == Detached::Abandoned {
                    helper.health.unhang(&helper.owner);
                }
                *s = Detached::Finished;
                let _ = tx.send(result);
            });
        if let Err(e) = spawned {
            return Err(anyhow!("plugin \"{}\" ({what}): could not start a helper thread: {e}", self.owner));
        }
        match rx.recv_timeout(budget) {
            Ok(result) => result,
            Err(_) => {
                let mut s = state.lock().unwrap_or_else(|p| p.into_inner());
                if *s == Detached::Finished {
                    drop(s);
                    return rx
                        .try_recv()
                        .unwrap_or_else(|_| Err(anyhow!("plugin \"{}\" ({what}) ended without a result", self.owner)));
                }
                *s = Detached::Abandoned;
                drop(s);
                let message = format!(
                    "plugin \"{}\" ({what}) did not finish within {}; the call was abandoned",
                    self.owner,
                    fmt_duration(budget)
                );
                self.health.mark_hung(&self.owner, &message);
                Err(anyhow!(message))
            }
        }
    }

    fn catch<T>(&self, what: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
        match catch_unwind(AssertUnwindSafe(f)) {
            Ok(result) => result,
            Err(payload) => {
                let message = format!("plugin \"{}\" ({what}) crashed: {}", self.owner, panic_message(payload.as_ref()));
                self.health.fail(&self.owner, &message);
                Err(anyhow!(message))
            }
        }
    }

    fn quiet<T>(&self, kind: CallKind, what: &str, fallback: T, f: impl FnOnce() -> T) -> T {
        self.call(kind, what, || Ok(f())).unwrap_or(fallback)
    }
}

/// The profile source an import on a helper thread compiles for itself.
/// It drops the pipeline: importers never run it (the workspace does, after
/// import), and plugin transforms are not available there.
fn detachable_profile(profile: &CompiledProfile) -> Profile {
    let mut p = profile.profile.clone();
    p.pipeline.clear();
    p
}

/// Runs `import` with an [`ImportCtx`] rebuilt from owned parts.
fn import_owned<T>(
    profile: Profile,
    path: &std::path::Path,
    file_date: Option<chrono::NaiveDate>,
    import: impl FnOnce(&ImportCtx) -> Result<T>,
) -> Result<T> {
    let profile = CompiledProfile::compile(profile)?;
    import(&ImportCtx {
        profile: &profile,
        path,
        file_date,
    })
}

pub struct GuardedImporter {
    pub inner: Arc<dyn Importer>,
    pub guard: Guard,
}

impl Importer for GuardedImporter {
    fn id(&self) -> &'static str {
        self.inner.id()
    }
    fn sniff(&self, ctx: &ImportCtx, head: &str) -> u8 {
        self.guard.quiet(CallKind::Sniff, "sniff", 0, || self.inner.sniff(ctx, head))
    }
    /// Runs on a helper thread (abandoned past the import budget) when there
    /// is a budget; on this thread otherwise.
    fn import(&self, ctx: &ImportCtx, content: &str) -> Result<Imported> {
        // Refuse before paying for the owned copy.
        if self.guard.health.admit(&self.guard.owner, CallKind::Import)?.is_none() {
            return self.guard.call(CallKind::Import, "import", || self.inner.import(ctx, content));
        }
        let (inner, profile, path, file_date, content) = (
            self.inner.clone(),
            detachable_profile(ctx.profile),
            ctx.path.to_path_buf(),
            ctx.file_date,
            content.to_string(),
        );
        self.guard.call_detached(CallKind::Import, "import", move || {
            import_owned(profile, &path, file_date, |ctx| inner.import(ctx, &content))
        })
    }
    fn import_tail(&self, ctx: &ImportCtx, head: &str, tail: &str) -> Option<Result<Imported>> {
        match self
            .guard
            .call(CallKind::Tail, "import_tail", || Ok(self.inner.import_tail(ctx, head, tail)))
        {
            Ok(result) => result,
            Err(e) => Some(Err(e)),
        }
    }
    fn multiline(&self) -> bool {
        // When in doubt, re-read from the last record start (always correct).
        self.guard.quiet(CallKind::Tail, "multiline", true, || self.inner.multiline())
    }
}

pub struct GuardedBinaryImporter {
    pub inner: Arc<dyn BinaryImporter>,
    pub guard: Guard,
}

impl BinaryImporter for GuardedBinaryImporter {
    fn id(&self) -> &'static str {
        self.inner.id()
    }
    fn sniff(&self, ctx: &ImportCtx, head: &[u8]) -> u8 {
        self.guard.quiet(CallKind::Sniff, "sniff", 0, || self.inner.sniff(ctx, head))
    }
    fn import(&self, ctx: &ImportCtx, bytes: &[u8]) -> Result<Imported> {
        if self.guard.health.admit(&self.guard.owner, CallKind::Import)?.is_none() {
            return self.guard.call(CallKind::Import, "import", || self.inner.import(ctx, bytes));
        }
        let (inner, profile, path, file_date, bytes) = (
            self.inner.clone(),
            detachable_profile(ctx.profile),
            ctx.path.to_path_buf(),
            ctx.file_date,
            bytes.to_vec(),
        );
        self.guard.call_detached(CallKind::Import, "import", move || {
            import_owned(profile, &path, file_date, |ctx| inner.import(ctx, &bytes))
        })
    }
}

/// A crashing transform leaves the record unchanged (and stops being called
/// once quarantined) instead of failing the whole import.
pub struct GuardedTransform {
    pub inner: Arc<dyn RecordTransform>,
    pub guard: Guard,
}

impl RecordTransform for GuardedTransform {
    fn id(&self) -> &str {
        self.inner.id()
    }
    fn apply(&self, record: &mut Record, ctx: &TransformCtx) -> Verdict {
        let scope = crate::plugin::markers::MarkScope::new(self.guard.owner());
        let verdict = self.guard.quiet(CallKind::Transform, "transform", Verdict::Keep, || {
            self.inner.apply(record, &ctx.scoped(&scope))
        });
        let (marked, rejected) = scope.finish();
        record.markers |= marked;
        for kind in rejected {
            self.guard.health.note(
                self.guard.owner(),
                &format!("mark(\"{kind}\") was refused: not a marker kind of this plugin (or switched off by the profile)"),
            );
        }
        verdict
    }
    fn batched(&self) -> bool {
        self.guard.quiet(CallKind::Transform, "batched", false, || self.inner.batched())
    }
    /// One guarded call for the whole batch, budgeted like an import (a
    /// batch can hold a whole file). A crash or overrun keeps every record.
    fn apply_batch(&self, records: &mut [Record], ctx: &TransformCtx) -> Vec<Verdict> {
        if !self.inner.batched() {
            // One guarded call (and mark scope) per record, exactly like `apply`.
            return records.iter_mut().map(|r| self.apply(r, ctx)).collect();
        }
        let scope = crate::plugin::markers::MarkScope::new(self.guard.owner());
        let n = records.len();
        let mut verdicts = self.guard.quiet(CallKind::Import, "transform", Vec::new(), || {
            self.inner.apply_batch(records, &ctx.scoped(&scope))
        });
        verdicts.resize(n, Verdict::Keep);
        let (_, rejected) = scope.finish();
        for kind in rejected {
            self.guard.health.note(
                self.guard.owner(),
                &format!("mark(\"{kind}\") was refused: not a marker kind of this plugin (or switched off by the profile)"),
            );
        }
        verdicts
    }
}

pub struct GuardedExporter {
    pub inner: Arc<dyn Exporter>,
    pub guard: Guard,
}

impl Exporter for GuardedExporter {
    fn id(&self) -> &'static str {
        self.inner.id()
    }
    fn name(&self) -> &'static str {
        self.inner.name()
    }
    fn extension(&self) -> &'static str {
        self.inner.extension()
    }
    fn export(&self, rows: &mut dyn Iterator<Item = ExportRow<'_>>, out: &mut dyn std::io::Write) -> Result<usize> {
        self.guard.call(CallKind::Export, "export", || self.inner.export(rows, out))
    }
}

pub struct GuardedHook {
    pub inner: Arc<dyn WorkspaceHook>,
    pub guard: Guard,
}

impl WorkspaceHook for GuardedHook {
    fn on_loaded(&self, summary: &WorkspaceSummary) {
        self.guard.quiet(CallKind::Hook, "on_loaded", (), || self.inner.on_loaded(summary));
    }
    fn on_cleared(&self) {
        self.guard.quiet(CallKind::Hook, "on_cleared", (), || self.inner.on_cleared());
    }
    fn on_profile_changed(&self, profile: &Profile) {
        self.guard
            .quiet(CallKind::Hook, "on_profile_changed", (), || self.inner.on_profile_changed(profile));
    }
}

pub struct GuardedCollector {
    pub inner: Arc<dyn Collector>,
    pub guard: Guard,
}

impl Collector for GuardedCollector {
    fn id(&self) -> &str {
        self.inner.id()
    }
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn run(&self, config: &serde_json::Value, sink: &dyn EventSink, stop: &AtomicBool) -> Result<()> {
        self.guard
            .call(CallKind::Collector, "collector", || self.inner.run(config, sink, stop))
    }
}

#[cfg(test)]
mod tests;
