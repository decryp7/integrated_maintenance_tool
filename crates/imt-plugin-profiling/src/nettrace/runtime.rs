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
    /// Runtime `GCAllocationTick` (V1..V4).
    pub const GC_ALLOCATION_TICK: u32 = 10;
    /// Runtime `ThreadPoolWorkerThreadStart`.
    pub const THREAD_POOL_WORKER_START: u32 = 50;
    /// Runtime `ThreadPoolWorkerThreadAdjustmentAdjustment`.
    pub const THREAD_POOL_ADJUSTMENT: u32 = 55;
    /// Rundown `DCEndComplete_V1` (the end of a rundown).
    pub const DC_END_COMPLETE: u32 = 146;
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
    /// `GCAllocationTick`.
    Alloc,
    /// A thread became a thread-pool worker.
    PoolWorker,
    /// The thread pool changed its worker count.
    PoolAdjust,
    /// A heap-walk event.
    Heap(super::heap::HeapEvent),
    /// IMT extension events (ETW captures).
    Image,
    Wait,
    ThreadName,
    NativeSymbol,
    /// `HeapAlloc`, `HeapFree` or `HeapReAlloc` (the extension event id).
    NativeHeap(u32),
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
            GC_ALLOCATION_TICK => Some(Known::Alloc),
            THREAD_POOL_WORKER_START => Some(Known::PoolWorker),
            THREAD_POOL_ADJUSTMENT => Some(Known::PoolAdjust),
            id => super::heap::HeapEvent::from_id(id).map(Known::Heap),
        };
    }
    if provider.eq_ignore_ascii_case(super::ext::PROVIDER) {
        return match event_id {
            super::ext::ids::IMAGE_LOAD => Some(Known::Image),
            super::ext::ids::THREAD_WAIT => Some(Known::Wait),
            super::ext::ids::THREAD_NAME => Some(Known::ThreadName),
            super::ext::ids::NATIVE_SYMBOL => Some(Known::NativeSymbol),
            id @ (super::ext::ids::HEAP_ALLOC | super::ext::ids::HEAP_FREE | super::ext::ids::HEAP_REALLOC) => Some(Known::NativeHeap(id)),
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

/// `GCAllocationTick`: what one tick (about 100 KB of allocations) reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AllocTick {
    /// Bytes allocated since the previous tick.
    pub bytes: u64,
    /// 0 small object heap, 1 large object heap, 2 pinned object heap.
    pub kind: u32,
    /// The type of the object whose allocation crossed the threshold (V2+).
    pub type_name: Option<String>,
}

/// `GCAllocationTick_V1..V4` (V1 has no type; V2 adds the 64-bit amount, type id and name).
/// `TypeID` is pointer-sized.
pub(super) fn alloc_tick(payload: &[u8], ptr: usize, max_name: usize) -> Option<AllocTick> {
    let mut c = Cursor::new(payload, 0);
    let amount = c.u32().ok()?;
    let kind = c.u32().ok()?;
    let _clr = c.u16().ok()?;
    let mut tick = AllocTick {
        bytes: u64::from(amount),
        kind,
        type_name: None,
    };
    if let Ok(amount64) = c.u64() {
        tick.bytes = amount64;
        if c.take(ptr).is_ok() {
            tick.type_name = c.utf16z(max_name).ok().filter(|t| !t.is_empty());
        }
    }
    Some(tick)
}

/// `ThreadPoolWorkerThreadAdjustmentAdjustment`: (new worker count, reason).
pub(super) fn pool_adjust(payload: &[u8]) -> Option<(u32, u32)> {
    let mut c = Cursor::new(payload, 0);
    let _throughput = c.u64().ok()?;
    Some((c.u32().ok()?, c.u32().ok()?))
}

/// The thread pool's adjustment reason for starvation.
pub(super) const POOL_STARVATION: u32 = 6;

#[cfg(test)]
mod tests {
    use super::super::writer::payload as p;
    use super::*;

    #[test]
    fn decodes_allocation_ticks_of_every_version_and_bitness() {
        let mut v1 = Vec::new();
        v1.extend_from_slice(&100_000u32.to_le_bytes());
        v1.extend_from_slice(&1u32.to_le_bytes());
        v1.extend_from_slice(&0u16.to_le_bytes());
        assert_eq!(
            alloc_tick(&v1, 8, 100),
            Some(AllocTick {
                bytes: 100_000,
                kind: 1,
                type_name: None
            })
        );
        for ptr in [4usize, 8] {
            let t = p::alloc_tick(123_456, 0, "System.Byte[]", ptr);
            let d = alloc_tick(&t, ptr, 100).unwrap();
            assert_eq!(d.bytes, 123_456);
            assert_eq!(d.type_name.as_deref(), Some("System.Byte[]"));
        }
        assert!(alloc_tick(&v1[..9], 8, 100).is_none());
        let mut a = Vec::new();
        a.extend_from_slice(&1.5f64.to_le_bytes());
        a.extend_from_slice(&12u32.to_le_bytes());
        a.extend_from_slice(&POOL_STARVATION.to_le_bytes());
        assert_eq!(pool_adjust(&a), Some((12, POOL_STARVATION)));
        assert!(pool_adjust(&a[..10]).is_none());
        assert_eq!(classify(RUNTIME, 10, ""), Some(Known::Alloc));
        assert_eq!(classify(RUNTIME, 18, ""), Some(Known::Heap(super::super::heap::HeapEvent::Node)));
        assert_eq!(classify(super::super::ext::PROVIDER, 2, ""), Some(Known::Wait));
        assert_eq!(classify(super::super::ext::PROVIDER, 4, ""), Some(Known::NativeSymbol));
        assert_eq!(classify(super::super::ext::PROVIDER, 6, ""), Some(Known::NativeHeap(6)));
        assert_eq!(classify(super::super::ext::PROVIDER, 9, ""), None);
    }

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
