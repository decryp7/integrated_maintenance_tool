//! Recorded-shape ETW event sequences → NetTrace → the importer.

use super::*;
use crate::kernel::encode as k;
use crate::nettrace::{system_time, TraceHeader};
use imt_plugin_profiling::nettrace::writer::payload as clrp;
use imt_plugin_profiling::nettrace::NettraceImporter;
use imt_profile::{Capture, CaptureImporter, EventKind, FrameKind, Limits, MetricId, Profile};

const PID: u32 = 100;
const OTHER: u32 = 200;
const T1: u32 = 11;
const T2: u32 = 12;
/// 10 MHz clock: 1 tick = 100 ns.
const FREQ: i64 = 10_000_000;

fn header(ptr: u32) -> TraceHeader {
    TraceHeader {
        sync_time: system_time(1_792_000_000_000).unwrap(),
        sync_ticks: 1_000,
        ticks_per_second: FREQ,
        pointer_size: ptr,
        process_id: PID,
        processors: 4,
        sampling_ns: 1_000_000,
    }
}

struct Ev {
    provider: u128,
    id: u16,
    opcode: u8,
    pid: u32,
    tid: u32,
    ts: i64,
    ptr: u8,
    payload: Vec<u8>,
    stack: Vec<u64>,
}

fn kernel(provider: u128, opcode: u8, tid: u32, ts: i64, payload: Vec<u8>) -> Ev {
    Ev {
        provider,
        id: 0,
        opcode,
        pid: 0,
        tid,
        ts,
        ptr: 8,
        payload,
        stack: Vec::new(),
    }
}

fn clr_event(rundown: bool, id: u16, tid: u32, ts: i64, ptr: u8, payload: Vec<u8>, stack: Vec<u64>) -> Ev {
    Ev {
        provider: if rundown { clr::RUNDOWN_GUID } else { clr::RUNTIME_GUID },
        id,
        opcode: 0,
        pid: PID,
        tid,
        ts,
        ptr,
        payload,
        stack,
    }
}

const MAIN: u64 = 0x0100_1000;
const WORK: u64 = 0x0100_2000;
const NTDLL: u64 = 0x0770_0000;
const KADDR: u64 = 0xffff_f800_0000_1234;

/// A small session: threads, an image, two methods, samples with split stacks, a wait, an exception.
fn events(ptr: u8) -> Vec<Ev> {
    let s = 8;
    let mut v = vec![
        kernel(guids::THREAD, opcodes::DC_START, 0, 1_000, k::thread_group1(PID, T1)),
        kernel(guids::THREAD, opcodes::DC_START, 0, 1_000, k::thread_group1(OTHER, 99)),
        kernel(guids::THREAD, opcodes::START, 0, 1_001, k::thread_group1(PID, T2)),
        kernel(
            guids::IMAGE,
            opcodes::DC_START,
            0,
            1_002,
            k::image_load(NTDLL, 0x10_0000, PID, r"C:\Windows\SysWOW64\ntdll.dll", s),
        ),
        kernel(
            guids::IMAGE,
            opcodes::IMAGE_LOAD,
            0,
            1_002,
            k::image_load(NTDLL, 0x10_0000, OTHER, r"C:\x\other.dll", s),
        ),
        kernel(
            guids::THREAD,
            opcodes::THREAD_SET_NAME,
            0,
            1_003,
            k::thread_set_name(PID, T1, "Main thread"),
        ),
    ];
    // Ten samples: kernel part first, deferred user part later (same timestamp).
    for i in 0..10i64 {
        let ts = 2_000 + i * 10_000;
        v.push(kernel(
            guids::PERF_INFO,
            opcodes::SAMPLED_PROFILE,
            T1,
            ts,
            k::sampled_profile(KADDR, T1, s),
        ));
        v.push(kernel(
            guids::STACK_WALK,
            opcodes::STACK_WALK,
            T1,
            ts,
            k::stack_walk(ts, PID, T1, &[KADDR, KADDR + 1], s),
        ));
        v.push(kernel(
            guids::STACK_WALK,
            opcodes::STACK_WALK,
            T1,
            ts,
            k::stack_walk(ts, PID, T1, &[WORK + 0x10, MAIN + 0x20, NTDLL + 0x40], s),
        ));
    }
    // A sample on another process's thread is ignored.
    v.push(kernel(
        guids::PERF_INFO,
        opcodes::SAMPLED_PROFILE,
        99,
        3_000,
        k::sampled_profile(0x10, 99, s),
    ));
    // T2 blocks (switch-out with its stack) and is readied by T1 2 ms later.
    v.push(kernel(guids::THREAD, opcodes::CSWITCH, T2, 50_000, k::cswitch(T1, T2, 6, 5)));
    v.push(kernel(
        guids::STACK_WALK,
        opcodes::STACK_WALK,
        T2,
        50_000,
        k::stack_walk(50_000, PID, T2, &[KADDR, NTDLL + 0x99, MAIN + 0x30], s),
    ));
    v.push(kernel(guids::THREAD, opcodes::READY_THREAD, T1, 69_000, k::ready_thread(T2)));
    v.push(kernel(guids::THREAD, opcodes::CSWITCH, T2, 70_000, k::cswitch(T2, T1, 32, 1)));
    // T1 preempted for a very short time: below the minimum wait, not written.
    v.push(kernel(guids::THREAD, opcodes::CSWITCH, T1, 70_100, k::cswitch(T1, T2, 6, 5)));
    // CLR: rundown names, an exception with its stack (WOW64 64-bit frame dropped for x86), completion.
    let mut stack = vec![KADDR, WORK + 0x8, MAIN + 0x8];
    if ptr == 4 {
        stack.insert(1, 0x7ff8_0000_0000);
    }
    v.push(clr_event(
        false,
        80,
        T1,
        60_000,
        ptr,
        clrp::exception("System.InvalidOperationException", "boom"),
        stack,
    ));
    v.push(clr_event(
        true,
        144,
        T1,
        90_000,
        ptr,
        clrp::method_verbose(1, 5, MAIN, 0x100, "App.Program", "Main"),
        vec![],
    ));
    v.push(clr_event(
        true,
        144,
        T1,
        90_000,
        ptr,
        clrp::method_verbose(2, 5, WORK, 0x100, "App.Program", "Work"),
        vec![],
    ));
    v.push(clr_event(
        true,
        154,
        T1,
        90_000,
        ptr,
        clrp::module_load(5, r"C:\app\App.exe"),
        vec![],
    ));
    v.push(clr_event(true, 146, T1, 91_000, ptr, vec![0; 4], vec![]));
    // CLR events of another process are ignored.
    let mut other = clr_event(false, 80, 99, 60_000, ptr, clrp::exception("X", "y"), vec![]);
    other.pid = OTHER;
    v.push(other);
    v
}

fn convert(ptr: u8, evs: &[Ev]) -> (Vec<u8>, Stats, Milestones) {
    let mut c = Converter::new(Vec::new(), header(u32::from(ptr))).unwrap();
    for e in evs {
        let raw = RawEvent {
            provider: e.provider,
            id: e.id,
            version: 0,
            opcode: e.opcode,
            process_id: e.pid,
            thread_id: e.tid,
            timestamp: e.ts,
            pointer_size: if e.provider == clr::RUNTIME_GUID || e.provider == clr::RUNDOWN_GUID {
                e.ptr
            } else {
                8
            },
            payload: &e.payload,
            stack: &e.stack,
        };
        c.event(&raw).unwrap();
    }
    let m = c.milestones();
    assert!(c.bytes() > 0);
    let (bytes, stats) = c.finish().unwrap();
    (bytes, stats, m)
}

fn import(bytes: &[u8]) -> Profile {
    match NettraceImporter.import("app.nettrace", bytes, &Limits::default()).unwrap() {
        Capture::Profile(p) => p,
        Capture::Heap(_) => panic!("a profile"),
    }
}

#[test]
fn converts_samples_waits_and_clr_events_for_x86_and_x64_targets() {
    for ptr in [4u8, 8] {
        let (bytes, stats, m) = convert(ptr, &events(ptr));
        assert_eq!(stats.samples, 10, "{ptr}");
        assert_eq!(stats.waits, 1, "the short wait is skipped");
        assert_eq!(stats.clr_events, 5, "the other process's exception is ignored");
        assert_eq!(stats.images, 1);
        assert!(m.rundown_complete);
        assert!(!m.heap_walk_done);
        let p = import(&bytes);
        let (k, n) = (p.metric_index(MetricId::Samples).unwrap(), p.metrics.len());
        assert_eq!((0..p.samples.len()).filter(|i| p.samples.values[i * n + k] > 0).count(), 10);
        let names: Vec<&str> = p.frames.iter().map(|f| f.function.as_str()).collect();
        assert!(names.contains(&"App.Program.Main"), "{names:?}");
        assert!(names.contains(&"App.Program.Work"));
        assert!(names.contains(&"ntdll.dll+0x40"), "native frames as module+offset: {names:?}");
        assert!(p.frames.iter().all(|f| f.kind != FrameKind::Kernel));
        assert!(!names.iter().any(|n| n.starts_with("0xffff")), "kernel frames are dropped");
        let wait = p.events.iter().find(|e| e.kind == EventKind::Wait).expect("a wait event");
        assert_eq!(wait.dur_ns, 2_000_000);
        assert!(wait.label.contains("Blocked waiting for an object"), "{}", wait.label);
        assert!(wait.label.contains(&format!("readied by thread {T1}")), "{}", wait.label);
        assert!(wait.stack.is_some());
        assert!(p
            .events
            .iter()
            .any(|e| e.kind == EventKind::Exception && e.label == "System.InvalidOperationException"));
        assert!(p.threads.iter().any(|t| t.name == "Main thread"), "{:?}", p.threads);
        assert!(p.metrics.iter().any(|m| m.id == MetricId::WaitNs));
    }
}

#[test]
fn heap_walk_milestones_and_damaged_payloads() {
    let evs = vec![
        clr_event(false, 1, T1, 10, 8, clrp::gc_start(1, 2, 1), vec![]),
        clr_event(false, 18, T1, 11, 8, vec![0; 10], vec![]),
        clr_event(false, 2, T1, 12, 8, clrp::gc_end(1, 2), vec![]),
        kernel(guids::PERF_INFO, opcodes::SAMPLED_PROFILE, T1, 13, vec![1, 2]),
        kernel(guids::STACK_WALK, opcodes::STACK_WALK, T1, 13, vec![1]),
        kernel(guids::THREAD, opcodes::CSWITCH, T1, 13, vec![1]),
        kernel(guids::THREAD, opcodes::START, T1, 13, vec![1]),
        kernel(guids::IMAGE, opcodes::IMAGE_LOAD, T1, 13, vec![1]),
    ];
    let (_, stats, m) = convert(8, &evs);
    assert!(m.heap_nodes && m.heap_walk_done);
    assert_eq!(stats.damaged, 5);
}

#[test]
fn samples_without_a_walk_keep_the_sampled_instruction_and_pending_queues_are_bounded() {
    let mut evs = vec![kernel(guids::THREAD, opcodes::START, 0, 1, k::thread_group1(PID, T1))];
    for i in 0..20 {
        evs.push(kernel(
            guids::PERF_INFO,
            opcodes::SAMPLED_PROFILE,
            T1,
            100 + i,
            k::sampled_profile(MAIN + 4, T1, 8),
        ));
    }
    evs.push(kernel(guids::THREAD, opcodes::END, 0, 500, k::thread_group1(PID, T1)));
    // After the thread ended, its samples are no longer the target's.
    evs.push(kernel(
        guids::PERF_INFO,
        opcodes::SAMPLED_PROFILE,
        T1,
        600,
        k::sampled_profile(MAIN, T1, 8),
    ));
    let (bytes, stats, _) = convert(8, &evs);
    assert_eq!(stats.samples, 20);
    let p = import(&bytes);
    assert!(p.frames.iter().any(|f| f.function == format!("0x{:x}", MAIN + 4)));
}

#[test]
fn seeded_threads_count_before_any_thread_event() {
    let mut c = Converter::new(Vec::new(), header(8)).unwrap();
    c.seed_threads([T1]);
    let p = k::sampled_profile(MAIN, T1, 8);
    c.event(&RawEvent {
        provider: guids::PERF_INFO,
        id: 0,
        version: 2,
        opcode: opcodes::SAMPLED_PROFILE,
        process_id: 0,
        thread_id: T1,
        timestamp: 5,
        pointer_size: 8,
        payload: &p,
        stack: &[],
    })
    .unwrap();
    assert_eq!(c.finish().unwrap().1.samples, 1);
}

#[test]
fn heap_events_symbols_and_addresses_for_native_captures() {
    use crate::heap::encode as h;
    use imt_plugin_profiling::nettrace::ext::{HeapOp, NativeSymbol};
    const EXE: u64 = 0x0140_0000;
    for ptr in [4u8, 8] {
        let s = usize::from(ptr);
        let options = Options {
            heap: true,
            max_addresses: 4,
        };
        let mut c = Converter::with_options(Vec::new(), header(u32::from(ptr)), options).unwrap();
        c.seed_threads([T1]);
        let mut push = |provider: u128, opcode: u8, pid: u32, ts: i64, payload: &[u8], stack: &[u64]| {
            c.event(&RawEvent {
                provider,
                id: 0,
                version: 2,
                opcode,
                process_id: pid,
                thread_id: T1,
                timestamp: ts,
                pointer_size: ptr,
                payload,
                stack,
            })
            .unwrap();
        };
        let img = k::image_load(EXE, 0x10_0000, PID, r"C:\app\busy.exe", s);
        push(guids::IMAGE, opcodes::IMAGE_LOAD, 0, 10, &img, &[]);
        let alloc = h::op(
            &HeapOp::Alloc {
                heap: 1,
                address: 0x5000,
                size: 64,
            },
            s,
        );
        let grow = h::op(
            &HeapOp::ReAlloc {
                heap: 1,
                new_address: 0x6000,
                old_address: 0x5000,
                size: 256,
            },
            s,
        );
        let free = h::op(&HeapOp::Free { heap: 1, address: 0x7000 }, s);
        // A kernel frame in the heap stack is dropped.
        push(heap::PROVIDER, alloc.0, PID, 20, &alloc.1, &[KADDR, EXE + 0x10, EXE + 0x20]);
        push(heap::PROVIDER, grow.0, PID, 30, &grow.1, &[EXE + 0x30, EXE + 0x20]);
        push(heap::PROVIDER, free.0, PID, 40, &free.1, &[EXE + 0x40]);
        // Other processes' heap events and damaged ones.
        push(heap::PROVIDER, alloc.0, OTHER, 50, &alloc.1, &[]);
        push(heap::PROVIDER, alloc.0, PID, 60, &alloc.1[..3], &[]);
        push(heap::PROVIDER, heap::opcodes::DESTROY, PID, 70, &[0; 8], &[]);
        // Samples whose addresses exceed the address budget.
        for i in 0..3u64 {
            let sp = k::sampled_profile(EXE + 0x100 + i, T1, s);
            push(guids::PERF_INFO, opcodes::SAMPLED_PROFILE, 0, 80 + i as i64, &sp, &[]);
        }
        assert_eq!(c.images().len(), 1);
        assert_eq!(c.addresses(), [EXE + 0x10, EXE + 0x20, EXE + 0x30], "free stacks are not written");
        let named = NativeSymbol {
            address: EXE + 0x10,
            start: EXE,
            line: 7,
            function: "busy::grow".into(),
            file: r"C:\src\busy.rs".into(),
        };
        c.symbols([&named]).unwrap();
        let (bytes, stats) = c.finish().unwrap();
        assert_eq!(stats.heap_ops, 3);
        assert_eq!(stats.damaged, 1);
        assert!(stats.addresses_dropped >= 2, "{stats:?}");
        let p = import(&bytes);
        assert!(p.frames.iter().any(|f| f.function == "busy::grow" && f.line == Some(7)));
        let live = p.metric_index(MetricId::LiveBytes).unwrap();
        let n = p.metrics.len();
        let total: u64 = (0..p.samples.len()).map(|i| p.samples.values[i * n + live]).sum();
        assert_eq!(total, 256, "the grown block is unfreed; the old one was reallocated");
    }
    // Without the heap option, heap events are ignored.
    let mut c = Converter::new(Vec::new(), header(8)).unwrap();
    let alloc = crate::heap::encode::op(
        &imt_plugin_profiling::nettrace::ext::HeapOp::Alloc {
            heap: 1,
            address: 2,
            size: 3,
        },
        8,
    );
    c.event(&RawEvent {
        provider: heap::PROVIDER,
        id: 0,
        version: 0,
        opcode: alloc.0,
        process_id: PID,
        thread_id: T1,
        timestamp: 5,
        pointer_size: 8,
        payload: &alloc.1,
        stack: &[],
    })
    .unwrap();
    assert_eq!(c.stats().heap_ops, 0);
    assert!(c.addresses().is_empty());
    assert_eq!(ext_name(99), "");
    assert_eq!(ext_name(ext::ids::HEAP_FREE), "HeapFree");
}
