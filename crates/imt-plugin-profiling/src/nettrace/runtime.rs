//! The events IMT understands and their payload layouts (from the CLR event
//! manifest, `ClrEtwAll.man`, as also used by TraceEvent's `ClrTraceEventParser`).
//! Payload fields are packed little-endian, strings are NUL-terminated UTF-16.
//! A payload that is too short for its fields decodes to `None` (the event is
//! skipped and counted), never to a panic.

use super::cursor::Cursor;

/// Providers.
pub const SAMPLE_PROFILER: &str = "Microsoft-DotNETCore-SampleProfiler";
pub const RUNTIME: &str = "Microsoft-Windows-DotNETRuntime";
pub const RUNDOWN: &str = "Microsoft-Windows-DotNETRuntimeRundown";

/// Event ids.
pub mod ids {
    /// SampleProfiler `ThreadSample`.
    pub const THREAD_SAMPLE: u32 = 0;
    /// Runtime `GCStart`.
    pub const GC_START: u32 = 1;
    /// Runtime `GCEnd`.
    pub const GC_END: u32 = 2;
    /// Runtime `ExceptionThrown_V1`.
    pub const EXCEPTION_THROWN: u32 = 80;
    /// Runtime `ContentionStart_V1`.
    pub const CONTENTION_START: u32 = 81;
    /// Runtime `ContentionStop`.
    pub const CONTENTION_STOP: u32 = 91;
    /// Runtime `MethodLoadVerbose`; Rundown `MethodDCStartVerbose`.
    pub const METHOD_LOAD_VERBOSE: u32 = 143;
    /// Rundown `MethodDCEndVerbose`.
    pub const METHOD_DC_END_VERBOSE: u32 = 144;
    /// Runtime `MethodJittingStarted`.
    pub const METHOD_JITTING_STARTED: u32 = 145;
    /// Runtime `ModuleLoad_V2`.
    pub const MODULE_LOAD: u32 = 152;
    /// Rundown `ModuleDCStart_V2`.
    pub const MODULE_DC_START: u32 = 153;
    /// Rundown `ModuleDCEnd_V2`.
    pub const MODULE_DC_END: u32 = 154;
}

/// What an event (by its metadata) means to the importer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Known {
    Sample,
    /// A method's code range; `jitted` for the runtime's load event (end of a JIT).
    Method {
        jitted: bool,
    },
    JitStart,
    Module,
    GcStart,
    GcEnd,
    Exception,
    ContentionStart,
    ContentionStop,
}

/// Classifies an event type by provider and event id (or name for the sampler).
pub(super) fn classify(provider: &str, event_id: u32, event_name: &str) -> Option<Known> {
    use ids::*;
    if provider.eq_ignore_ascii_case(SAMPLE_PROFILER) {
        return (event_id == THREAD_SAMPLE || event_name == "ThreadSample").then_some(Known::Sample);
    }
    if provider.eq_ignore_ascii_case(RUNTIME) {
        return match event_id {
            GC_START => Some(Known::GcStart),
            GC_END => Some(Known::GcEnd),
            EXCEPTION_THROWN => Some(Known::Exception),
            CONTENTION_START => Some(Known::ContentionStart),
            CONTENTION_STOP => Some(Known::ContentionStop),
            METHOD_LOAD_VERBOSE => Some(Known::Method { jitted: true }),
            METHOD_JITTING_STARTED => Some(Known::JitStart),
            MODULE_LOAD => Some(Known::Module),
            _ => None,
        };
    }
    if provider.eq_ignore_ascii_case(RUNDOWN) {
        return match event_id {
            METHOD_LOAD_VERBOSE | METHOD_DC_END_VERBOSE => Some(Known::Method { jitted: false }),
            MODULE_DC_START | MODULE_DC_END => Some(Known::Module),
            _ => None,
        };
    }
    None
}

/// `MethodLoadVerbose` / `MethodDC*Verbose`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MethodInfo {
    pub method_id: u64,
    pub module_id: u64,
    pub start: u64,
    pub size: u32,
    /// `Namespace.Name` (the namespace field holds the full type name).
    pub name: String,
}

pub(super) fn method(payload: &[u8], max_name: usize) -> Option<MethodInfo> {
    let mut c = Cursor::new(payload, 0);
    let method_id = c.u64().ok()?;
    let module_id = c.u64().ok()?;
    let start = c.u64().ok()?;
    let size = c.u32().ok()?;
    let _token = c.u32().ok()?;
    let _flags = c.u32().ok()?;
    let namespace = c.utf16z(max_name).ok()?;
    let name = c.utf16z(max_name).ok()?;
    Some(MethodInfo {
        method_id,
        module_id,
        start,
        size,
        name: qualified(&namespace, &name),
    })
}

/// `MethodJittingStarted`: (method id, qualified name).
pub(super) fn jitting_started(payload: &[u8], max_name: usize) -> Option<(u64, String)> {
    let mut c = Cursor::new(payload, 0);
    let method_id = c.u64().ok()?;
    let _module_id = c.u64().ok()?;
    let _token = c.u32().ok()?;
    let _il_size = c.u32().ok()?;
    let namespace = c.utf16z(max_name).ok()?;
    let name = c.utf16z(max_name).ok()?;
    Some((method_id, qualified(&namespace, &name)))
}

fn qualified(namespace: &str, name: &str) -> String {
    if namespace.is_empty() {
        name.to_string()
    } else {
        format!("{namespace}.{name}")
    }
}

/// `ModuleLoad_V2` / `ModuleDC*_V2`: (module id, file name of the IL path).
pub(super) fn module(payload: &[u8], max_name: usize) -> Option<(u64, String)> {
    let mut c = Cursor::new(payload, 0);
    let module_id = c.u64().ok()?;
    let _assembly_id = c.u64().ok()?;
    let _flags = c.u32().ok()?;
    let _reserved = c.u32().ok()?;
    let il_path = c.utf16z(max_name.saturating_mul(4)).ok()?;
    let file = imt_profile::frames::basename(&il_path);
    Some((module_id, imt_profile::limits::clip(file, max_name)))
}

/// `GCStart`: (count, generation, reason).
pub(super) fn gc_start(payload: &[u8]) -> Option<(u32, u32, u32)> {
    let mut c = Cursor::new(payload, 0);
    Some((c.u32().ok()?, c.u32().ok()?, c.u32().ok()?))
}

/// `GCEnd`: the count it ends.
pub(super) fn gc_end(payload: &[u8]) -> Option<u32> {
    Cursor::new(payload, 0).u32().ok()
}

/// Text for a `GCStart` reason.
pub(super) fn gc_reason(reason: u32) -> &'static str {
    match reason {
        0 => "small object allocation",
        1 => "induced",
        2 => "low memory",
        3 => "empty",
        4 => "large object allocation",
        5 => "out of space (SOH)",
        6 => "out of space (LOH)",
        7 => "induced, not forced",
        8 => "internal tuning",
        9 => "induced, low memory",
        10 => "induced, compacting",
        11 => "low memory (host)",
        12 => "PM full GC",
        13 => "low memory (host, blocking)",
        _ => "other",
    }
}

/// `ExceptionThrown_V1`: the exception type.
pub(super) fn exception(payload: &[u8], max_name: usize) -> Option<String> {
    Cursor::new(payload, 0).utf16z(max_name).ok()
}

/// `ContentionStart_V1` / `ContentionStop`: whether the lock is native (flags bit 0).
pub(super) fn contention_native(payload: &[u8]) -> Option<bool> {
    Some(Cursor::new(payload, 0).u8().ok()? & 1 == 1)
}

#[cfg(test)]
mod tests {
    use super::super::writer::payload as p;
    use super::*;

    #[test]
    fn classifies_by_provider_and_id() {
        assert_eq!(classify(SAMPLE_PROFILER, 0, ""), Some(Known::Sample));
        assert_eq!(
            classify("microsoft-dotnetcore-sampleprofiler", 9, "ThreadSample"),
            Some(Known::Sample)
        );
        assert_eq!(classify(SAMPLE_PROFILER, 1, "ThreadStackWalk"), None);
        assert_eq!(classify(RUNTIME, 143, ""), Some(Known::Method { jitted: true }));
        assert_eq!(classify(RUNDOWN, 143, ""), Some(Known::Method { jitted: false }));
        assert_eq!(classify(RUNDOWN, 144, ""), Some(Known::Method { jitted: false }));
        assert_eq!(classify(RUNDOWN, 154, ""), Some(Known::Module));
        assert_eq!(classify(RUNDOWN, 1, ""), None);
        for (id, k) in [
            (1, Known::GcStart),
            (2, Known::GcEnd),
            (80, Known::Exception),
            (81, Known::ContentionStart),
            (91, Known::ContentionStop),
            (145, Known::JitStart),
            (152, Known::Module),
        ] {
            assert_eq!(classify(RUNTIME, id, ""), Some(k));
        }
        assert_eq!(classify(RUNTIME, 999, ""), None);
        assert_eq!(classify("MyEventSource", 1, "x"), None);
    }

    #[test]
    fn decodes_payloads_and_rejects_short_ones() {
        let m = p::method_verbose(7, 9, 0x1000, 0x40, "Shop.Orders.OrderService", "PlaceOrder");
        let info = method(&m, 100).unwrap();
        assert_eq!(
            info,
            MethodInfo {
                method_id: 7,
                module_id: 9,
                start: 0x1000,
                size: 0x40,
                name: "Shop.Orders.OrderService.PlaceOrder".into()
            }
        );
        assert_eq!(method(&p::method_verbose(1, 1, 1, 1, "", "Main"), 100).unwrap().name, "Main");
        assert!(method(&m[..30], 100).is_none());
        assert_eq!(jitting_started(&p::jitting_started(7, 9, "A", "B"), 100), Some((7, "A.B".into())));
        assert!(jitting_started(&[0; 10], 100).is_none());
        assert_eq!(module(&p::module_load(9, r"C:\app\Shop.dll"), 100), Some((9, "Shop.dll".into())));
        assert_eq!(
            module(&p::module_load(9, "/usr/share/dotnet/System.Private.CoreLib.dll"), 6)
                .unwrap()
                .1,
            "System"
        );
        assert!(module(&[0; 23], 100).is_none());
        assert_eq!(gc_start(&p::gc_start(3, 2, 1)), Some((3, 2, 1)));
        assert!(gc_start(&[0; 11]).is_none());
        assert_eq!(gc_end(&p::gc_end(3, 2)), Some(3));
        assert!(gc_end(&[]).is_none());
        assert_eq!(
            exception(&p::exception("System.Exception", "boom"), 100).as_deref(),
            Some("System.Exception")
        );
        assert!(exception(&[0x41], 100).is_none());
        assert_eq!(contention_native(&p::contention(true)), Some(true));
        assert_eq!(contention_native(&p::contention(false)), Some(false));
        assert!(contention_native(&[]).is_none());
        for r in 0..20 {
            assert!(!gc_reason(r).is_empty());
        }
    }
}
