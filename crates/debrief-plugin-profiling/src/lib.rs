//! Debrief plugin: profile and heap snapshot importers.
//!
//! Declares the `profiling.importers` extension point (trait
//! [`debrief_profile::CaptureImporter`]) and contributes the built-in importers
//! to it. Other plugins (for example a .NET helper converting `.etl` or
//! `.gcdump` files) contribute their importers to the same point; the host
//! reaches all of them as guarded services.
//!
//! | Importer | Files |
//! |---|---|
//! | [`speedscope::SpeedscopeImporter`] | speedscope JSON (evented and sampled) |
//! | [`folded::FoldedImporter`] | collapsed/folded stacks (`stackcollapse-*`, `perf script \| stackcollapse-perf.pl`) |
//! | [`chrome::CpuProfileImporter`] | Chrome / Node `.cpuprofile` |
//! | [`chrome::TraceEventImporter`] | Chrome trace event JSON (CPU profile chunks, duration events) |
//! | [`pprof::PprofImporter`] | pprof `profile.proto` (gzip or plain) |
//! | [`perf::PerfScriptImporter`] | `perf script` text output |
//! | [`nettrace::NettraceImporter`] | .NET EventPipe `.nettrace` (sample profiler, GC, exceptions, contention) |
//! | [`heapsnapshot::HeapSnapshotImporter`] | V8 / Chrome / Node `.heapsnapshot` (object graph) |
//!
//! It runs in its own process (`debrief-plugins --plugin net.decryptology.profiling`, ADR 0017)
//! (feature `plugin`, on by default). Without that feature the crate is just
//! the importers, which is what the fuzz targets build.

pub mod chrome;
pub mod folded;
pub mod heapsnapshot;
pub mod nettrace;
pub mod perf;
pub mod pprof;
pub mod speedscope;
pub mod util;

use debrief_profile::CaptureImporter;
use std::sync::Arc;

#[cfg(feature = "plugin")]
mod plugin;
#[cfg(feature = "plugin")]
pub use plugin::ProfilingPlugin;

/// Plugin id.
pub const PLUGIN_ID: &str = "net.decryptology.profiling";

/// Every built-in importer, most specific first.
pub fn builtin_importers() -> Vec<Arc<dyn CaptureImporter>> {
    vec![
        Arc::new(nettrace::NettraceImporter),
        Arc::new(pprof::PprofImporter),
        Arc::new(heapsnapshot::HeapSnapshotImporter),
        Arc::new(speedscope::SpeedscopeImporter),
        Arc::new(chrome::CpuProfileImporter),
        Arc::new(chrome::TraceEventImporter),
        Arc::new(perf::PerfScriptImporter),
        Arc::new(folded::FoldedImporter),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn importers_have_unique_ids_and_refuse_empty_files() {
        let all = builtin_importers();
        let mut ids: Vec<&str> = all.iter().map(|i| i.id()).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n, "importer ids are unique");
        for i in builtin_importers() {
            assert!(!i.name().is_empty());
            assert_eq!(i.sniff("empty", b""), 0, "{} claims an empty file", i.id());
        }
    }
}
