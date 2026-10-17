//! Crash containment for plugin code.
//!
//! Every contribution a plugin registers is wrapped in a guarded adapter
//! (decorator) that turns a panic into an error attributed to the plugin,
//! records it in [`Health`], and stops calling a plugin once it is
//! quarantined. Plugins need no changes to benefit.

use crate::api::WorkspaceSummary;
use crate::export::{EventSink, ExportRow, Exporter};
use crate::importers::{BinaryImporter, ImportCtx, Imported, Importer};
use crate::model::Record;
use crate::plugin::{Collector, RecordTransform, TransformCtx, Verdict, WorkspaceHook};
use crate::profile::Profile;
use anyhow::{anyhow, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

/// A plugin is quarantined (no longer called) after this many crashes.
pub const QUARANTINE_AFTER: u32 = 3;

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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginHealth {
    pub failures: u32,
    pub last_error: Option<String>,
    pub quarantined: bool,
}

/// Failure counts per plugin id, shared by every guarded adapter.
#[derive(Debug, Default)]
pub struct Health {
    plugins: Mutex<HashMap<String, PluginHealth>>,
}

impl Health {
    fn with<T>(&self, f: impl FnOnce(&mut HashMap<String, PluginHealth>) -> T) -> T {
        let mut map = self.plugins.lock().unwrap_or_else(|p| p.into_inner());
        f(&mut map)
    }

    /// Records a crash; quarantines the plugin after [`QUARANTINE_AFTER`] of them.
    pub fn fail(&self, plugin: &str, error: &str) {
        self.with(|m| {
            let h = m.entry(plugin.to_string()).or_default();
            h.failures += 1;
            h.last_error = Some(error.chars().take(500).collect());
            h.quarantined |= h.failures >= QUARANTINE_AFTER;
        });
    }

    pub fn is_quarantined(&self, plugin: &str) -> bool {
        self.with(|m| m.get(plugin).is_some_and(|h| h.quarantined))
    }

    pub fn get(&self, plugin: &str) -> PluginHealth {
        self.with(|m| m.get(plugin).cloned().unwrap_or_default())
    }

    /// Forgets a plugin's failures and lifts its quarantine.
    pub fn reset(&self, plugin: &str) {
        self.with(|m| m.remove(plugin));
    }
}

/// Owner and health shared by the adapters of one plugin.
#[derive(Clone)]
pub struct Guard {
    pub owner: String,
    pub health: Arc<Health>,
}

impl Guard {
    fn quarantined_error(&self) -> anyhow::Error {
        anyhow!(
            "plugin \"{}\" is disabled after repeated crashes (re-enable it in the Plugins panel)",
            self.owner
        )
    }

    fn run<T>(&self, what: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
        if self.health.is_quarantined(&self.owner) {
            return Err(self.quarantined_error());
        }
        catch(&format!("plugin \"{}\" ({what})", self.owner), f).inspect_err(|e| {
            if e.to_string().contains(" crashed: ") {
                self.health.fail(&self.owner, &e.to_string());
            }
        })
    }

    fn quiet<T>(&self, what: &str, fallback: T, f: impl FnOnce() -> T) -> T {
        self.run(what, || Ok(f())).unwrap_or(fallback)
    }
}

pub struct GuardedImporter {
    pub inner: Box<dyn Importer>,
    pub guard: Guard,
}

impl Importer for GuardedImporter {
    fn id(&self) -> &'static str {
        self.inner.id()
    }
    fn sniff(&self, ctx: &ImportCtx, head: &str) -> u8 {
        self.guard.quiet("sniff", 0, || self.inner.sniff(ctx, head))
    }
    fn import(&self, ctx: &ImportCtx, content: &str) -> Result<Imported> {
        self.guard.run("import", || self.inner.import(ctx, content))
    }
}

pub struct GuardedBinaryImporter {
    pub inner: Box<dyn BinaryImporter>,
    pub guard: Guard,
}

impl BinaryImporter for GuardedBinaryImporter {
    fn id(&self) -> &'static str {
        self.inner.id()
    }
    fn sniff(&self, ctx: &ImportCtx, head: &[u8]) -> u8 {
        self.guard.quiet("sniff", 0, || self.inner.sniff(ctx, head))
    }
    fn import(&self, ctx: &ImportCtx, bytes: &[u8]) -> Result<Imported> {
        self.guard.run("import", || self.inner.import(ctx, bytes))
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
        self.guard.quiet("transform", Verdict::Keep, || self.inner.apply(record, ctx))
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
        self.guard.run("export", || self.inner.export(rows, out))
    }
}

pub struct GuardedHook {
    pub inner: Arc<dyn WorkspaceHook>,
    pub guard: Guard,
}

impl WorkspaceHook for GuardedHook {
    fn on_loaded(&self, summary: &WorkspaceSummary) {
        self.guard.quiet("on_loaded", (), || self.inner.on_loaded(summary));
    }
    fn on_cleared(&self) {
        self.guard.quiet("on_cleared", (), || self.inner.on_cleared());
    }
    fn on_profile_changed(&self, profile: &Profile) {
        self.guard
            .quiet("on_profile_changed", (), || self.inner.on_profile_changed(profile));
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
        self.guard.run("collector", || self.inner.run(config, sink, stop))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catch_reports_panics_and_passes_results() {
        assert_eq!(catch("x", || Ok(5)).unwrap(), 5);
        assert!(catch::<()>("x", || Err(anyhow!("plain"))).unwrap_err().to_string() == "plain");
        let e = catch::<()>("parser", || panic!("boom {}", 1)).unwrap_err().to_string();
        assert_eq!(e, "parser crashed: boom 1");
        let e = catch::<()>("p", || std::panic::panic_any(42u8)).unwrap_err().to_string();
        assert!(e.ends_with("unknown panic"));
    }

    #[test]
    fn health_quarantines_after_repeated_crashes() {
        let h = Health::default();
        for _ in 0..QUARANTINE_AFTER - 1 {
            h.fail("p", "x");
        }
        assert!(!h.is_quarantined("p"));
        h.fail("p", &"long ".repeat(200));
        assert!(h.is_quarantined("p"));
        assert_eq!(h.get("p").failures, QUARANTINE_AFTER);
        assert_eq!(h.get("p").last_error.unwrap().chars().count(), 500);
        h.reset("p");
        assert_eq!(h.get("p"), PluginHealth::default());
    }
}
