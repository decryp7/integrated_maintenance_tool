//! Unit tests over writer-generated traces: every block type, symbolisation,
//! runtime events, version handling and hostile input.

use super::runtime::{ids, RUNDOWN, RUNTIME, SAMPLE_PROFILER};
use super::writer::{payload as p, Meta, Row, TraceHeader, Writer};
use super::*;
use std::io::Write as _;

const MAIN: u64 = 0x7f00_0000_1000;
const PLACE: u64 = 0x7f00_0000_2000;
const SAVE: u64 = 0x7f00_0000_3000;
const UNKNOWN: u64 = 0xdead_beef;

const M_SAMPLE: u32 = 1;
const M_LOAD: u32 = 2;
const M_DCEND: u32 = 3;
const M_MODULE: u32 = 4;
const M_GC_START: u32 = 5;
const M_GC_END: u32 = 6;
const M_EXC: u32 = 7;
const M_CONT_START: u32 = 8;
const M_CONT_STOP: u32 = 9;
const M_JIT: u32 = 10;
const M_OTHER: u32 = 11;

fn metas() -> Vec<Meta<'static>> {
    vec![
        Meta {
            id: M_SAMPLE,
            provider: SAMPLE_PROFILER,
            event_id: ids::THREAD_SAMPLE,
            name: "ThreadSample",
        },
        Meta {
            id: M_LOAD,
            provider: RUNTIME,
            event_id: ids::METHOD_LOAD_VERBOSE,
            name: "",
        },
        Meta {
            id: M_DCEND,
            provider: RUNDOWN,
            event_id: ids::METHOD_DC_END_VERBOSE,
            name: "",
        },
        Meta {
            id: M_MODULE,
            provider: RUNDOWN,
            event_id: ids::MODULE_DC_END,
            name: "",
        },
        Meta {
            id: M_GC_START,
            provider: RUNTIME,
            event_id: ids::GC_START,
            name: "",
        },
        Meta {
            id: M_GC_END,
            provider: RUNTIME,
            event_id: ids::GC_END,
            name: "",
        },
        Meta {
            id: M_EXC,
            provider: RUNTIME,
            event_id: ids::EXCEPTION_THROWN,
            name: "",
        },
        Meta {
            id: M_CONT_START,
            provider: RUNTIME,
            event_id: ids::CONTENTION_START,
            name: "",
        },
        Meta {
            id: M_CONT_STOP,
            provider: RUNTIME,
            event_id: ids::CONTENTION_STOP,
            name: "",
        },
        Meta {
            id: M_JIT,
            provider: RUNTIME,
            event_id: ids::METHOD_JITTING_STARTED,
            name: "",
        },
        Meta {
            id: M_OTHER,
            provider: "MyCompany-Shop",
            event_id: 1,
            name: "OrderPlaced",
        },
    ]
}

fn row(metadata_id: u32, thread: u64, stack_id: u32, ts: i64, payload: Vec<u8>) -> Row {
    Row {
        metadata_id,
        sequence: 0,
        thread,
        capture_thread: thread,
        processor: 0,
        stack_id,
        ts,
        sorted: false,
        payload,
    }
}

/// QPC ticks (10 MHz) for `ms` after the sync time (1_000_000).
fn at(ms: i64) -> i64 {
    1_000_000 + ms * 10_000
}

/// A complete trace: modules, JIT, samples on two threads, GC, an exception,
/// contention, a sequence point, then rundown.
fn trace(compressed: bool) -> Vec<u8> {
    let mut w = Writer::new(&TraceHeader::default());
    w.metadata(compressed, &metas());
    w.stacks(1, &[vec![PLACE + 4, MAIN + 8], vec![UNKNOWN, SAVE + 1, PLACE, MAIN]]);
    let mut seq = 0u32;
    let mut rows = vec![
        row(
            M_JIT,
            10,
            0,
            at(0),
            p::jitting_started(3, 50, "Shop.Orders.OrderRepository", "Save"),
        ),
        row(
            M_LOAD,
            10,
            0,
            at(2),
            p::method_verbose(3, 50, SAVE, 0x80, "Shop.Orders.OrderRepository", "Save"),
        ),
        row(M_SAMPLE, 10, 1, at(1), p::sample(true)),
        row(M_SAMPLE, 10, 1, at(2), p::sample(true)),
        row(M_SAMPLE, 11, 2, at(3), p::sample(false)),
        row(M_SAMPLE, 11, 0, at(4), p::sample(true)),
        row(M_GC_START, 11, 0, at(5), p::gc_start(1, 0, 0)),
        row(M_GC_END, 11, 0, at(7), p::gc_end(1, 0)),
        row(M_EXC, 10, 1, at(8), p::exception("Shop.Orders.InvalidOrderException", "bad order")),
        row(M_CONT_START, 11, 2, at(9), p::contention(false)),
        row(M_CONT_STOP, 11, 2, at(12), p::contention(false)),
        row(M_OTHER, 10, 0, at(12), vec![1, 2, 3]),
        row(99, 10, 0, at(12), vec![]),
    ];
    for r in &mut rows {
        seq += 1;
        r.sequence = seq;
    }
    rows[0].sorted = true;
    w.events(compressed, &rows);
    w.sequence_point(at(13), &[(10, 5), (11, 7)]);
    // Stack id 1 is reused for a different stack after the sequence point.
    w.stacks(1, &[vec![SAVE, PLACE, MAIN]]);
    w.events(compressed, &[row(M_SAMPLE, 10, 1, at(20), p::sample(true))]);
    let rundown = vec![
        row(M_MODULE, 12, 0, at(30), p::module_load(50, r"C:\app\Shop.dll")),
        row(
            M_DCEND,
            12,
            0,
            at(30),
            p::method_verbose(1, 50, MAIN, 0x100, "Shop.Program", "Main"),
        ),
        row(
            M_DCEND,
            12,
            0,
            at(30),
            p::method_verbose(2, 50, PLACE, 0x100, "Shop.Orders.OrderService", "PlaceOrder"),
        ),
        row(
            M_DCEND,
            12,
            0,
            at(30),
            p::method_verbose(3, 50, SAVE, 0x80, "Shop.Orders.OrderRepository", "Save"),
        ),
    ];
    w.events(compressed, &rundown);
    w.finish()
}

fn import(bytes: &[u8]) -> Result<Profile> {
    import_with(bytes, &Limits::default())
}

fn import_with(bytes: &[u8], limits: &Limits) -> Result<Profile> {
    match NettraceImporter.import("orders.nettrace", bytes, limits)? {
        Capture::Profile(p) => Ok(p),
        Capture::Heap(_) => panic!("a nettrace is a profile"),
    }
}

fn names(p: &Profile, stack: u32) -> Vec<String> {
    p.stack_frames(stack)
        .iter()
        .map(|&f| p.frames[f as usize].function.clone())
        .collect()
}

#[test]
fn imports_samples_symbols_and_runtime_events() {
    let p = import(&trace(true)).unwrap();
    assert_eq!(p.name, "orders");
    assert_eq!(p.source, "nettrace");
    assert!(p.has_time);
    let ids: Vec<MetricId> = p.metrics.iter().map(|m| m.id).collect();
    assert_eq!(ids, [MetricId::Samples, MetricId::TimeNs, MetricId::Exceptions, MetricId::WaitNs]);
    assert_eq!(p.metrics[0].measurement, Measurement::Sampled);
    assert_eq!(p.metrics[2].measurement, Measurement::Exact);
    // 5 CPU samples + one exception + one wait sample.
    assert_eq!(p.samples.len(), 7);
    let threads: Vec<(Option<u64>, &str)> = p.threads.iter().map(|t| (t.os_id, t.name.as_str())).collect();
    assert!(
        threads.contains(&(Some(10), "Thread 10")) && threads.contains(&(Some(11), "Thread 11")),
        "{threads:?}"
    );
    // Samples are in time order with nanoseconds since the sync time.
    assert_eq!(p.samples.time[0], 1_000_000);
    let first = p.samples.stack[0];
    assert_eq!(names(&p, first), ["Shop.Program.Main", "Shop.Orders.OrderService.PlaceOrder"]);
    let main = &p.frames[p.stack_frames(first)[0] as usize];
    assert_eq!(main.module.as_deref(), Some("Shop.dll"));
    assert_eq!(main.kind, FrameKind::Managed);
    // Thread 11's first sample has an unresolved leaf.
    let i = p.samples.time.iter().position(|&t| t == 3_000_000).unwrap();
    let s = names(&p, p.samples.stack[i]);
    assert_eq!(
        s,
        [
            "Shop.Program.Main",
            "Shop.Orders.OrderService.PlaceOrder",
            "Shop.Orders.OrderRepository.Save",
            "0xdeadbeef"
        ]
    );
    let leaf = *p.stack_frames(p.samples.stack[i]).last().unwrap();
    assert_eq!(p.frames[leaf as usize].kind, FrameKind::Native);
    // A sample without a stack.
    let i = p.samples.time.iter().position(|&t| t == 4_000_000).unwrap();
    assert_eq!(names(&p, p.samples.stack[i]), ["(no stack)"]);
    // After the sequence point stack id 1 means the new stack.
    let i = p.samples.time.iter().position(|&t| t == 20_000_000).unwrap();
    assert_eq!(names(&p, p.samples.stack[i]).len(), 3);
    // Values: one sample of 1 ms each.
    let stride = p.metrics.len();
    assert_eq!(&p.samples.values[..stride], &[1, 1_000_000, 0, 0]);

    let kinds: Vec<(EventKind, u64, u64, &str)> = p.events.iter().map(|e| (e.kind, e.start_ns, e.dur_ns, e.label.as_str())).collect();
    assert!(
        kinds.contains(&(EventKind::Jit, 0, 2_000_000, "Shop.Orders.OrderRepository.Save")),
        "{kinds:?}"
    );
    assert!(kinds.contains(&(EventKind::Gc, 5_000_000, 2_000_000, "GC #1 gen 0 (small object allocation)")));
    assert!(kinds.contains(&(EventKind::Exception, 8_000_000, 0, "Shop.Orders.InvalidOrderException")));
    assert!(kinds.contains(&(EventKind::LockWait, 9_000_000, 3_000_000, "Monitor lock contention")));
    let gc = p.events.iter().find(|e| e.kind == EventKind::Gc).unwrap();
    assert_eq!(gc.thread, None);
    let exc = p.events.iter().find(|e| e.kind == EventKind::Exception).unwrap();
    assert_eq!(
        names(&p, exc.stack.unwrap()),
        ["Shop.Program.Main", "Shop.Orders.OrderService.PlaceOrder"]
    );
    let wait = p.metric_index(MetricId::WaitNs).unwrap();
    let total_wait: u64 = p.samples.values.chunks(stride).map(|v| v[wait]).sum();
    assert_eq!(total_wait, 3_000_000);

    // 2026-10-17 09:30:00 UTC.
    assert_eq!(p.started_ms, Some(1_792_229_400_000));
    assert_eq!(p.duration_ns, 30_000_000);
    assert!(
        p.notes.iter().any(|n| n.starts_with("1 code addresses are not covered")),
        "{:?}",
        p.notes
    );
    assert!(
        p.notes.iter().any(|n| n.starts_with("1 events with undefined metadata")),
        "{:?}",
        p.notes
    );
}

#[test]
fn uncompressed_headers_give_the_same_profile() {
    let a = import(&trace(true)).unwrap();
    let b = import(&trace(false)).unwrap();
    assert_eq!(a, b);
}

#[test]
fn gzip_input_is_inflated() {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    e.write_all(&trace(true)).unwrap();
    let z = e.finish().unwrap();
    assert_eq!(NettraceImporter.sniff("x.nettrace.gz", &z), 90);
    assert_eq!(NettraceImporter.sniff("x.gz", &z), 0);
    assert_eq!(import(&z).unwrap().samples.len(), 7);
}

#[test]
fn sniffs_the_magic_only() {
    let t = trace(true);
    assert_eq!(NettraceImporter.sniff("a.bin", &t), 100);
    assert_eq!(NettraceImporter.sniff("a.nettrace", b"Nettrac"), 0);
    assert_eq!(NettraceImporter.sniff("a.nettrace", b"{}"), 0);
    assert_eq!(NettraceImporter.id(), "nettrace");
    assert_eq!(NettraceImporter.name(), ".NET nettrace (EventPipe)");
}

fn header(f: impl FnOnce(&mut TraceHeader)) -> TraceHeader {
    let mut h = TraceHeader::default();
    f(&mut h);
    h
}

fn minimal(h: &TraceHeader) -> Vec<u8> {
    let mut w = Writer::new(h);
    w.metadata(true, &metas());
    w.events(true, &[row(M_SAMPLE, 1, 0, h.sync_qpc + 100, p::sample(true))]);
    w.finish()
}

fn err(bytes: &[u8]) -> String {
    import(bytes).unwrap_err().to_string()
}

#[test]
fn versions_are_checked() {
    assert!(import(&minimal(&header(|h| h.version = 5))).is_ok(), "V5");
    for v in [3, 6] {
        let e = import(&minimal(&header(|h| h.version = v))).unwrap_err();
        assert!(matches!(e, ProfileError::Unsupported { .. }), "{e}");
        assert!(e.to_string().contains(&format!("version {v}")), "{e}");
    }
    let e = import(&minimal(&header(|h| h.min_version = 6))).unwrap_err();
    assert!(matches!(e, ProfileError::Unsupported { .. }));
    // A NetTrace V7 stream header (V6 is read: see `reads_nettrace_v6`).
    let mut v7 = b"Nettrace".to_vec();
    v7.extend_from_slice(&[0, 0, 0, 0, 7, 0, 0, 0, 0, 0, 0, 0]);
    let e = import(&v7).unwrap_err();
    assert!(
        matches!(e, ProfileError::Unsupported { .. }) && e.to_string().contains("version 7"),
        "{e}"
    );
    assert!(import(b"Nettrace\0\0\0\0").is_err());
    // A block that needs a newer reader.
    let mut w = Writer::new(&TraceHeader::default());
    w.block("EventBlock", 3, &[20, 0, 0, 0]);
    let e = import(&w.finish()).unwrap_err();
    assert!(e.to_string().contains("EventBlock version 3"), "{e}");
}

#[test]
fn rejects_bad_headers() {
    assert!(err(b"").contains("Nettrace signature"));
    assert!(err(b"NetPerf!").contains("Nettrace signature"));
    let mut bad = b"Nettrace".to_vec();
    bad.extend_from_slice(&20u32.to_le_bytes());
    bad.extend_from_slice(b"!FastSerialization.2");
    assert!(err(&bad).contains("FastSerialization header"));
    assert!(err(&minimal(&header(|h| h.qpc_freq = 0))).contains("frequency"));
    assert!(err(&minimal(&header(|h| h.pointer_size = 3))).contains("pointer size"));
    // No Trace object at all.
    let t = minimal(&TraceHeader::default());
    let mut only_end = t[..32].to_vec();
    only_end.push(1);
    assert!(err(&only_end).contains("no Trace object"));
    // A block before the Trace object.
    let mut early = t[..32].to_vec();
    early.extend_from_slice(&[5, 5, 1, 2, 0, 0, 0, 2, 0, 0, 0, 7, 0, 0, 0]);
    early.extend_from_slice(b"SPBlock");
    early.push(6);
    assert!(err(&early).contains("must come first"));
    // Unexpected tags.
    let mut w = Writer::new(&TraceHeader::default());
    w.raw(&[9]);
    assert!(err(&w.finish()).contains("unexpected tag 9"));
    let mut w = Writer::new(&TraceHeader::default());
    w.raw(&[5, 1]);
    assert!(err(&w.finish()).contains("expected tag 5, found 1"));
    // Type name length.
    let mut w = Writer::new(&TraceHeader::default());
    w.raw(&[5, 5, 1, 2, 0, 0, 0, 2, 0, 0, 0, 0xff, 0xff, 0, 0]);
    assert!(err(&w.finish()).contains("too long"));
    // A second Trace object.
    let mut w = Writer::new(&TraceHeader::default());
    w.raw(&t[32..32 + 23 + 48 + 1]);
    assert!(err(&w.finish()).contains("second Trace"));
}

#[test]
fn every_truncation_is_an_error() {
    let t = trace(true);
    for cut in 0..t.len() {
        assert!(import(&t[..cut]).is_err(), "cut at {cut} of {}", t.len());
    }
    let u = trace(false);
    for cut in (0..u.len()).step_by(7) {
        assert!(import(&u[..cut]).is_err());
    }
}

#[test]
fn hostile_sizes_and_counts_are_errors() {
    let lim = Limits::default();
    // Block size larger than the file.
    let mut w = Writer::new(&TraceHeader::default());
    w.raw(&[5, 5, 1, 2, 0, 0, 0, 2, 0, 0, 0, 10, 0, 0, 0]);
    w.raw(b"EventBlock");
    w.raw(&[6, 0xff, 0xff, 0xff, 0x7f]);
    assert!(err(&w.finish()).contains("ends unexpectedly"));
    // Negative block size.
    let mut w = Writer::new(&TraceHeader::default());
    w.raw(&[5, 5, 1, 2, 0, 0, 0, 2, 0, 0, 0, 10, 0, 0, 0]);
    w.raw(b"EventBlock");
    w.raw(&[6, 0xff, 0xff, 0xff, 0xff]);
    assert!(err(&w.finish()).contains("negative block size"));
    // A stack block claiming four billion stacks.
    let mut w = Writer::new(&TraceHeader::default());
    let mut sb = 1u32.to_le_bytes().to_vec();
    sb.extend_from_slice(&u32::MAX.to_le_bytes());
    sb.extend_from_slice(&8u32.to_le_bytes());
    sb.extend_from_slice(&[0; 8]);
    w.block("StackBlock", 2, &sb);
    assert!(err(&w.finish()).contains("declares 4294967295 stacks"));
    // A stack larger than its block.
    let mut w = Writer::new(&TraceHeader::default());
    let mut sb = 1u32.to_le_bytes().to_vec();
    sb.extend_from_slice(&1u32.to_le_bytes());
    sb.extend_from_slice(&0x7fff_ffffu32.to_le_bytes());
    w.block("StackBlock", 2, &sb);
    assert!(err(&w.finish()).contains("ends unexpectedly"));
    // A sequence point with too many threads.
    let mut w = Writer::new(&TraceHeader::default());
    let mut sp = 0i64.to_le_bytes().to_vec();
    sp.extend_from_slice(&u32::MAX.to_le_bytes());
    w.block("SPBlock", 2, &sp);
    assert!(err(&w.finish()).contains("4294967295 threads"));
    // Event payload length larger than the block.
    let mut w = Writer::new(&TraceHeader::default());
    let mut eb = vec![20, 0, 1, 0];
    eb.extend_from_slice(&[0; 16]);
    eb.extend_from_slice(&[0x81, 1, 0, 0xff, 0xff, 0xff, 0xff, 0x0f]);
    w.block("EventBlock", 2, &eb);
    assert!(err(&w.finish()).contains("ends unexpectedly"));
    // Block header size too small, or bigger than the block.
    for hs in [2u16, 400] {
        let mut w = Writer::new(&TraceHeader::default());
        let mut eb = hs.to_le_bytes().to_vec();
        eb.extend_from_slice(&[1, 0, 0, 0]);
        w.block("EventBlock", 2, &eb);
        assert!(import_with(&w.finish(), &lim).is_err());
    }
    // An uncompressed event shorter than its header.
    let mut w = Writer::new(&TraceHeader::default());
    let mut eb = vec![20, 0, 0, 0];
    eb.extend_from_slice(&[0; 16]);
    eb.extend_from_slice(&8u32.to_le_bytes());
    eb.extend_from_slice(&[0; 8]);
    w.block("EventBlock", 2, &eb);
    assert!(err(&w.finish()).contains("shorter than its header"));
    // An uncompressed payload larger than the event.
    let mut w = Writer::new(&TraceHeader::default());
    let mut eb = super::writer::rows_block(false, &[row(1, 1, 0, 5, vec![])]);
    let n = eb.len();
    eb[n - 4..].copy_from_slice(&1000u32.to_le_bytes());
    w.block("EventBlock", 2, &eb);
    assert!(err(&w.finish()).contains("ends unexpectedly"));
    // Metadata with an unterminated provider name.
    let mut w = Writer::new(&TraceHeader::default());
    let mut r = row(0, 0, 0, 0, vec![1, 0, 0, 0, 0x41, 0]);
    r.sorted = true;
    w.block("MetadataBlock", 2, &super::writer::rows_block(true, &[r]));
    assert!(err(&w.finish()).contains("no terminating NUL"));
}

#[test]
fn compressed_rows_can_repeat_headers_without_hanging() {
    // Rows of two bytes (flags 0 + timestamp delta 0) reuse the previous header.
    let mut w = Writer::new(&TraceHeader::default());
    w.metadata(true, &metas());
    let mut eb = super::writer::rows_block(true, &[row(M_SAMPLE, 1, 0, 2_000_000, vec![])]);
    eb.extend(std::iter::repeat_n([0u8, 0u8], 5000).flatten());
    w.block("EventBlock", 2, &eb);
    let p = import(&w.finish()).unwrap();
    assert_eq!(p.samples.len(), 5001);
    let small = Limits {
        max_samples: 100,
        ..Limits::default()
    };
    let mut w = Writer::new(&TraceHeader::default());
    w.metadata(true, &metas());
    w.block("EventBlock", 2, &eb);
    assert!(matches!(
        import_with(&w.finish(), &small),
        Err(ProfileError::LimitExceeded { what: "samples", .. })
    ));
}

#[test]
fn limits_are_enforced() {
    let t = trace(true);
    let tiny = Limits {
        max_input_bytes: 10,
        ..Limits::default()
    };
    assert!(matches!(
        import_with(&t, &tiny),
        Err(ProfileError::LimitExceeded { what: "file size", .. })
    ));
    let few_stacks = Limits {
        max_stack_nodes: 1,
        ..Limits::default()
    };
    assert!(matches!(import_with(&t, &few_stacks), Err(ProfileError::LimitExceeded { .. })));
    let few_events = Limits {
        max_events: 1,
        ..Limits::default()
    };
    let p = import_with(&t, &few_events).unwrap();
    assert!(p.notes.iter().any(|n| n.contains("beyond the event limit")), "{:?}", p.notes);
    let few_frames = Limits {
        max_frames: 1,
        ..Limits::default()
    };
    assert!(matches!(import_with(&t, &few_frames), Err(ProfileError::LimitExceeded { .. })));
}

#[test]
fn method_and_module_tables_are_bounded() {
    let mut w = Writer::new(&TraceHeader::default());
    w.metadata(true, &metas());
    let mut rows = vec![row(M_SAMPLE, 1, 0, at(1), p::sample(true))];
    for i in 0..5u64 {
        rows.push(row(M_DCEND, 1, 0, at(2), p::method_verbose(i, 1, 0x1000 * (i + 1), 16, "N", "M")));
        rows.push(row(M_MODULE, 1, 0, at(2), p::module_load(i, "m.dll")));
    }
    w.events(true, &rows);
    let lim = Limits {
        max_frames: 3,
        ..Limits::default()
    };
    let p = import_with(&w.finish(), &lim).unwrap();
    assert!(p.notes.iter().any(|n| n.starts_with("4 method or module records")), "{:?}", p.notes);
}

#[test]
fn damaged_payloads_unknown_objects_and_missing_stacks_are_noted() {
    let mut w = Writer::new(&TraceHeader::default());
    w.metadata(true, &metas());
    w.block("FutureBlock", 2, &[1, 2, 3]);
    w.events(
        true,
        &[
            row(M_SAMPLE, 1, 77, at(1), p::sample(true)),
            row(M_GC_START, 1, 0, at(2), vec![1]),
            row(M_GC_END, 1, 0, at(2), vec![]),
            row(M_EXC, 1, 0, at(2), vec![0x41]),
            row(M_CONT_START, 1, 0, at(2), vec![]),
            row(M_JIT, 1, 0, at(2), vec![1]),
            row(M_LOAD, 1, 0, at(2), vec![1]),
            row(M_MODULE, 1, 0, at(2), vec![1]),
        ],
    );
    let p = import(&w.finish()).unwrap();
    for n in ["1 events referred to stacks", "7 runtime events had damaged", "1 unknown objects"] {
        assert!(p.notes.iter().any(|x| x.starts_with(n)), "{n}: {:?}", p.notes);
    }
    assert_eq!(names(&p, p.samples.stack[0]), ["(no stack)"]);
}

#[test]
fn traces_without_samples_or_methods() {
    // Only runtime events: an exception (metric) and unpaired starts (instants).
    let mut w = Writer::new(&TraceHeader::default());
    w.metadata(true, &metas());
    w.events(
        true,
        &[
            row(M_GC_START, 1, 0, at(1), p::gc_start(4, 2, 1)),
            row(M_JIT, 1, 0, at(2), p::jitting_started(9, 1, "", "Lambda")),
            row(M_LOAD, 2, 0, at(3), p::method_verbose(8, 1, 0x10, 0, "A", "B")),
            row(M_CONT_START, 1, 0, at(4), p::contention(true)),
            row(M_CONT_STOP, 2, 0, at(5), p::contention(true)),
            row(M_GC_END, 1, 0, at(6), p::gc_end(5, 0)),
            row(M_CONT_START, 3, 0, at(7), p::contention(true)),
            row(M_CONT_STOP, 3, 0, at(8), p::contention(true)),
        ],
    );
    let p = import(&w.finish()).unwrap();
    assert_eq!(p.metrics.iter().map(|m| m.id).collect::<Vec<_>>(), [MetricId::WaitNs]);
    assert_eq!(p.samples.values, [1_000_000]);
    let labels: Vec<(&str, u64)> = p.events.iter().map(|e| (e.label.as_str(), e.dur_ns)).collect();
    assert_eq!(
        labels,
        [
            ("GC #4 gen 2 (induced)", 0),
            ("Lambda", 0),
            ("A.B", 0),
            ("Native lock contention", 1_000_000)
        ]
    );

    // Only unpaired events: a placeholder metric.
    let mut w = Writer::new(&TraceHeader::default());
    w.metadata(true, &metas());
    w.events(true, &[row(M_GC_START, 1, 0, at(1), p::gc_start(4, 2, 1))]);
    let p = import(&w.finish()).unwrap();
    assert_eq!(p.metrics.len(), 1);
    assert!(p.samples.is_empty());

    // Exceptions only.
    let mut w = Writer::new(&TraceHeader::default());
    w.metadata(true, &metas());
    w.events(true, &[row(M_EXC, 1, 0, at(1), p::exception("E", ""))]);
    let p = import(&w.finish()).unwrap();
    assert_eq!(p.metrics.iter().map(|m| m.id).collect::<Vec<_>>(), [MetricId::Exceptions]);
    assert_eq!(names(&p, p.samples.stack[0]), ["(no stack)"]);

    // Samples with stacks but no rundown at all; 32-bit pointers; odd sampling rate.
    let h = header(|h| {
        h.pointer_size = 4;
        h.sampling_ns = 0;
    });
    let mut w = Writer::new(&h);
    w.metadata(false, &metas());
    w.stacks(1, &[vec![0x1234, 0x5678]]);
    w.events(false, &[row(M_SAMPLE, 1, 1, h.sync_qpc + 10, p::sample(true))]);
    let p = import(&w.finish()).unwrap();
    assert_eq!(names(&p, p.samples.stack[0]), ["0x5678", "0x1234"]);
    assert_eq!(p.samples.values, [1, DEFAULT_INTERVAL_NS]);
    assert!(p.notes.iter().any(|n| n.contains("no method load or rundown")), "{:?}", p.notes);

    // The same stack in two sequence-point regions is interned once.
    let mut w = Writer::new(&h);
    w.metadata(true, &metas());
    for k in 0..2 {
        w.stacks(1, &[vec![0x1234, 0x5678]]);
        w.events(true, &[row(M_SAMPLE, 1, 1, h.sync_qpc + 10 + k, p::sample(true))]);
        w.sequence_point(h.sync_qpc + 20 + k, &[]);
    }
    let p = import(&w.finish()).unwrap();
    assert_eq!(p.samples.stack[0], p.samples.stack[1]);

    // Nothing usable.
    let mut w = Writer::new(&TraceHeader::default());
    w.metadata(true, &metas());
    w.events(true, &[row(M_OTHER, 1, 0, at(1), vec![])]);
    assert!(err(&w.finish()).contains("no CPU samples"));
}

#[test]
fn clock_and_dates() {
    let c = Clock {
        origin: 100,
        freq: 1_000_000_000,
    };
    assert_eq!(c.ns(50), 0);
    assert_eq!(c.ns(150), 50);
    let c = Clock { origin: i64::MIN, freq: 1 };
    assert_eq!(c.ns(i64::MAX), u64::MAX, "saturates");
    assert_eq!(Clock { origin: 0, freq: 0 }.ns(5), 0);
    assert_eq!(system_time_ms([1970, 1, 4, 1, 0, 0, 0, 0]), Some(0));
    assert_eq!(system_time_ms([2000, 3, 3, 1, 0, 0, 1, 5]), Some(951_868_801_005));
    assert_eq!(system_time_ms([1601, 1, 1, 1, 0, 0, 0, 0]), Some(-11_644_473_600_000));
    for bad in [
        [1600, 1, 0, 1, 0, 0, 0, 0],
        [2000, 13, 0, 1, 0, 0, 0, 0],
        [2000, 1, 0, 0, 0, 0, 0, 0],
        [2000, 1, 0, 1, 24, 0, 0, 0],
        [2000, 1, 0, 1, 0, 60, 0, 0],
        [2000, 1, 0, 1, 0, 0, 60, 0],
        [2000, 1, 0, 1, 0, 0, 0, 1000],
    ] {
        assert_eq!(system_time_ms(bad), None, "{bad:?}");
    }
    // Events before the sync time move the origin (and the start) back.
    let h = TraceHeader::default();
    let mut w = Writer::new(&h);
    w.metadata(true, &metas());
    w.events(
        true,
        &[
            row(M_SAMPLE, 1, 0, h.sync_qpc - 10_000, p::sample(true)),
            row(M_SAMPLE, 1, 0, h.sync_qpc, p::sample(true)),
        ],
    );
    let p = NettraceImporter.import("", &w.finish(), &Limits::default()).unwrap();
    let Capture::Profile(p) = p else { panic!() };
    assert_eq!(p.samples.time, [0, 1_000_000]);
    assert_eq!(p.started_ms, Some(1_792_229_400_000 - 1));
    assert_eq!(p.name, "Process 4242");
}

#[test]
fn symbol_lookup_handles_edges() {
    let m = |start, size| runtime::MethodInfo {
        method_id: 0,
        module_id: 0,
        start,
        size,
        name: String::new(),
    };
    let methods = [m(100, 10), m(100, 10), m(200, 0), m(u64::MAX - 1, 100)];
    let s = Ranges::new(methods.iter().map(|m| (m.start, u64::from(m.size))));
    assert_eq!(s.ranges.len(), 3, "duplicates removed");
    assert_eq!(s.lookup(99), None);
    assert_eq!(s.lookup(100), Some(0));
    assert_eq!(s.lookup(109), Some(0));
    assert_eq!(s.lookup(110), None);
    assert_eq!(s.lookup(200), Some(2), "size 0 covers its start");
    assert_eq!(s.lookup(201), None);
    assert_eq!(s.lookup(u64::MAX - 1), Some(3));
}

fn meta(id: u32, provider: &'static str, event_id: u32) -> Meta<'static> {
    Meta {
        id,
        provider,
        event_id,
        name: "",
    }
}

#[test]
fn allocation_ticks_become_alloc_samples_with_the_type_as_leaf() {
    for ptr in [4u32, 8] {
        let mut w = Writer::new(&TraceHeader {
            pointer_size: ptr,
            ..TraceHeader::default()
        });
        w.metadata(
            true,
            &[
                meta(1, RUNTIME, ids::GC_ALLOCATION_TICK),
                meta(2, RUNDOWN, ids::METHOD_DC_END_VERBOSE),
            ],
        );
        const MAIN32: u64 = 0x1000_1000;
        w.stacks(1, &[vec![MAIN32 + 4]]);
        w.events(
            true,
            &[
                row(1, 10, 1, at(1), p::alloc_tick(100_000, 0, "System.String", ptr as usize)),
                row(1, 10, 1, at(2), p::alloc_tick(200_000, 1, "System.Byte[]", ptr as usize)),
                row(2, 10, 0, at(3), p::method_verbose(1, 50, MAIN32, 0x100, "Shop.Program", "Main")),
            ],
        );
        let p = import(&w.finish()).unwrap();
        assert_eq!(p.kind, ProfileKind::Alloc);
        let bytes = p.metric_index(MetricId::AllocBytes).unwrap();
        let n = p.metrics.len();
        let total: u64 = (0..p.samples.len()).map(|i| p.samples.values[i * n + bytes]).sum();
        assert_eq!(total, 300_000);
        let leafs: Vec<Vec<String>> = (0..p.samples.len()).map(|i| names(&p, p.samples.stack[i])).collect();
        assert!(
            leafs.iter().any(|s| s.last().map(String::as_str) == Some("System.String")),
            "{leafs:?}"
        );
        assert!(leafs
            .iter()
            .any(|s| s.last().map(String::as_str) == Some("System.Byte[] (large object heap)")));
        assert!(leafs.iter().all(|s| s.first().map(String::as_str) == Some("Shop.Program.Main")));
    }
}

#[test]
fn etw_extension_events_and_pool_starvation() {
    let mut w = Writer::new(&TraceHeader::default());
    w.metadata(
        true,
        &[
            meta(1, ext::PROVIDER, ext::ids::IMAGE_LOAD),
            meta(2, ext::PROVIDER, ext::ids::THREAD_WAIT),
            meta(3, ext::PROVIDER, ext::ids::THREAD_NAME),
            meta(4, SAMPLE_PROFILER, ids::THREAD_SAMPLE),
            meta(5, RUNTIME, ids::THREAD_POOL_ADJUSTMENT),
            meta(6, RUNTIME, ids::THREAD_POOL_WORKER_START),
        ],
    );
    w.stacks(1, &[vec![0x7ff0_0010, UNKNOWN]]);
    let image = ext::image_load(&ext::ImageLoad {
        base: 0x7ff0_0000,
        size: 0x1000,
        path: r"C:\Windows\System32\KERNELBASE.dll".into(),
    });
    let wait = ext::thread_wait(&ext::ThreadWait {
        duration: 30_000,
        reason: 4,
        readied_by: 0,
    });
    w.events(
        true,
        &[
            row(1, 10, 0, at(0), image),
            row(3, 10, 0, at(0), ext::thread_name("UI thread")),
            row(6, 11, 0, at(0), vec![0; 10]),
            row(4, 10, 1, at(1), p::sample(false)),
            row(2, 10, 1, at(2), wait),
            row(5, 11, 0, at(3), p::pool_adjust(9, 6)),
            row(5, 11, 0, at(4), p::pool_adjust(10, 3)),
            row(2, 10, 0, at(5), vec![1]),
        ],
    );
    let p = import(&w.finish()).unwrap();
    let s = names(&p, p.samples.stack[0]);
    assert!(s.contains(&"KERNELBASE.dll+0x10".to_string()), "{s:?}");
    assert!(p.notes.iter().any(|n| n.contains("module+offset")), "{:?}", p.notes);
    let wait = p.events.iter().find(|e| e.kind == EventKind::Wait).unwrap();
    assert_eq!(wait.dur_ns, 3_000_000);
    assert_eq!(wait.label, "Sleeping (delay)");
    assert!(p.metrics.iter().any(|m| m.id == MetricId::WaitNs));
    let starving: Vec<_> = p.events.iter().filter(|e| e.kind == EventKind::Marker).collect();
    assert_eq!(starving.len(), 1, "only the starvation adjustment is a marker");
    assert!(starving[0].label.contains("starvation"));
    assert!(p.threads.iter().any(|t| t.name == "UI thread"));
    assert!(p.threads.iter().any(|t| t.name == ".NET ThreadPool Worker"));
    assert!(
        p.notes.iter().any(|n| n.contains("damaged")),
        "the short wait payload: {:?}",
        p.notes
    );
}

#[test]
fn heap_walks_import_as_heap_snapshots() {
    use super::heap::{ids as h, payload as hp};
    for ptr in [4usize, 8] {
        let mut w = Writer::new(&TraceHeader {
            pointer_size: ptr as u32,
            ..TraceHeader::default()
        });
        w.metadata(
            true,
            &[
                meta(1, RUNTIME, h::BULK_TYPE),
                meta(2, RUNTIME, h::BULK_NODE),
                meta(3, RUNTIME, h::BULK_EDGE),
                meta(4, RUNTIME, h::BULK_ROOT_EDGE),
                meta(5, RUNTIME, ids::GC_START),
                meta(6, RUNTIME, ids::GC_END),
            ],
        );
        w.events(
            true,
            &[
                row(5, 1, 0, at(0), p::gc_start(1, 2, 1)),
                row(1, 1, 0, at(1), hp::bulk_type(&[(7, "Shop.Order", &[]), (8, "System.String", &[])])),
                row(2, 1, 0, at(1), hp::bulk_node(0, &[(0x1000, 32, 7, 1), (0x2000, 20, 8, 0)], ptr)),
                row(3, 1, 0, at(1), hp::bulk_edge(0, &[0x2000], ptr)),
                row(4, 1, 0, at(1), hp::bulk_root_edge(0, &[(0x1000, 0, 0)], ptr)),
                row(6, 1, 0, at(2), p::gc_end(1, 2)),
            ],
        );
        let Capture::Heap(s) = NettraceImporter
            .import("app.gcdump.nettrace", &w.finish(), &Limits::default())
            .unwrap()
        else {
            panic!("a heap snapshot");
        };
        assert_eq!(s.name, "app");
        assert!(s.taken_ms.is_some());
        let g = s.graph.unwrap();
        assert_eq!(g.len(), 3, "the stack-roots group and two objects");
        assert_eq!(g.roots.len(), 1);
        assert!(s.types.iter().any(|t| t.name == "Shop.Order" && t.bytes == 32));
    }
}

#[test]
fn reads_nettrace_v6() {
    use super::writer::v6::{V6Writer, LABEL_LIST};
    for (compressed, ptr) in [(true, 8u32), (false, 8), (true, 4)] {
        let main = if ptr == 4 { 0x1000_1000 } else { MAIN };
        let mut w = V6Writer::new(6, ptr, 4242, 2_000_000);
        w.metadata(&[
            (1, SAMPLE_PROFILER, ids::THREAD_SAMPLE, "ThreadSample"),
            (2, RUNDOWN, ids::METHOD_DC_END_VERBOSE, "MethodDCEndVerbose"),
            (3, RUNTIME, ids::EXCEPTION_THROWN, "ExceptionThrown"),
        ]);
        w.threads(&[(1, 7001, "Main"), (2, 7002, "")]);
        w.block(LABEL_LIST, &[1, 0, 0, 0, 0, 0, 0, 0]);
        w.block(99, &[1, 2, 3]);
        w.stacks(1, &[vec![main + 4]]);
        let exc = p::exception("System.TimeoutException", "late");
        let sample = p::sample(true);
        w.events(
            compressed,
            &[(1, 1, 1, at(1), &sample), (1, 2, 1, at(2), &sample), (3, 1, 1, at(3), &exc)],
        );
        w.sequence_point(at(4));
        w.stacks(1, &[vec![main + 8, main]]);
        let dc = p::method_verbose(1, 50, main, 0x100, "Shop.Program", "Main");
        w.events(compressed, &[(1, 1, 1, at(5), &sample), (2, 1, 0, at(6), &dc)]);
        let p = import(&w.finish()).unwrap();
        assert_eq!(p.samples.len(), 4, "3 samples + the exception");
        assert!(p.frames.iter().any(|f| f.function == "Shop.Program.Main"), "{compressed} {ptr}");
        assert!(
            p.threads.iter().any(|t| t.name == "Main" && t.os_id == Some(7001)),
            "{:?}",
            p.threads
        );
        assert!(p.threads.iter().any(|t| t.os_id == Some(7002)));
        assert!(p.events.iter().any(|e| e.label == "System.TimeoutException"));
        assert!(
            p.notes.iter().any(|n| n.contains("unknown objects")),
            "the unknown block is counted: {:?}",
            p.notes
        );
    }
}

#[test]
fn v6_errors_and_truncation() {
    use super::writer::v6::V6Writer;
    let err = |b: &[u8]| import(b).unwrap_err().to_string();
    assert!(err(&V6Writer::new(7, 8, 1, 0).finish()).contains("version 7"));
    assert!(err(&V6Writer::new(5, 8, 1, 0).finish()).contains("version 5"));
    // Cut before any event: no samples or events.
    let mut w = V6Writer::new(6, 8, 1, 0);
    w.metadata(&[(1, SAMPLE_PROFILER, 0, "ThreadSample")]);
    let sample = p::sample(true);
    w.events(true, &[(1, 1, 0, at(1), &sample)]);
    // A stream without EndOfStream keeps what it read.
    let p = import(&w.unfinished()).unwrap();
    assert_eq!(p.samples.len(), 1);
    let mut w = V6Writer::new(6, 8, 1, 0);
    w.metadata(&[(1, SAMPLE_PROFILER, 0, "ThreadSample")]);
    let bytes = w.finish();
    // A block longer than the file is an error, not a panic.
    let mut cut = bytes.clone();
    cut.truncate(bytes.len() - 6);
    assert!(import(&cut).is_err());
    // An event block before the Trace block.
    let mut raw = b"Nettrace".to_vec();
    for v in [0u32, 6, 0] {
        raw.extend_from_slice(&v.to_le_bytes());
    }
    raw.extend_from_slice(&((2u32 << 24) | 4).to_le_bytes());
    raw.extend_from_slice(&[4, 0, 0, 0]);
    assert!(err(&raw).contains("Trace block must come first"));
    // Only the header.
    let mut head = b"Nettrace".to_vec();
    for v in [0u32, 6, 0] {
        head.extend_from_slice(&v.to_le_bytes());
    }
    assert!(err(&head).contains("Trace block"));
}

#[test]
fn native_symbols_name_frames_and_heap_events_pair_by_address() {
    let mut w = Writer::new(&TraceHeader::default());
    w.metadata(
        true,
        &[
            meta(1, ext::PROVIDER, ext::ids::IMAGE_LOAD),
            meta(2, ext::PROVIDER, ext::ids::NATIVE_SYMBOL),
            meta(3, SAMPLE_PROFILER, ids::THREAD_SAMPLE),
            meta(4, ext::PROVIDER, ext::ids::HEAP_ALLOC),
            meta(5, ext::PROVIDER, ext::ids::HEAP_FREE),
            meta(6, ext::PROVIDER, ext::ids::HEAP_REALLOC),
        ],
    );
    const BASE: u64 = 0x7ff6_0000_0000;
    const SPIN: u64 = BASE + 0x1010;
    const MAIN_FN: u64 = BASE + 0x2020;
    const GROW: u64 = BASE + 0x3030;
    w.stacks(1, &[vec![SPIN, MAIN_FN], vec![GROW, MAIN_FN], vec![SPIN + 4, MAIN_FN]]);
    let image = ext::image_load(&ext::ImageLoad {
        base: BASE,
        size: 0x10_000,
        path: r"C:\app\busy.exe".into(),
    });
    let sym = |address: u64, start: u64, function: &str, line: u32| {
        ext::native_symbol(&ext::NativeSymbol {
            address,
            start,
            line,
            function: function.into(),
            file: r"C:\src\busy.rs".into(),
        })
    };
    let heap = |op: ext::HeapOp| ext::heap_op(&op).1;
    w.events(
        true,
        &[
            row(1, 10, 0, at(0), image),
            row(2, 10, 0, at(0), sym(SPIN, BASE + 0x1000, "busy::spin", 12)),
            row(2, 10, 0, at(0), sym(SPIN + 4, BASE + 0x1000, "busy::spin", 12)),
            row(2, 10, 0, at(0), sym(MAIN_FN, BASE + 0x2000, "busy::main", 0)),
            row(3, 10, 1, at(1), p::sample(false)),
            row(3, 10, 3, at(2), p::sample(false)),
            // Two allocations: one freed, one reallocated (grown) and kept; a free of older memory.
            row(
                4,
                11,
                2,
                at(3),
                heap(ext::HeapOp::Alloc {
                    heap: 1,
                    address: 0x500,
                    size: 100,
                }),
            ),
            row(
                4,
                11,
                2,
                at(4),
                heap(ext::HeapOp::Alloc {
                    heap: 1,
                    address: 0x600,
                    size: 40,
                }),
            ),
            row(5, 11, 0, at(5), heap(ext::HeapOp::Free { heap: 1, address: 0x500 })),
            row(
                6,
                11,
                2,
                at(6),
                heap(ext::HeapOp::ReAlloc {
                    heap: 1,
                    new_address: 0x700,
                    old_address: 0x600,
                    size: 80,
                }),
            ),
            row(5, 11, 0, at(7), heap(ext::HeapOp::Free { heap: 1, address: 0x999 })),
            row(4, 11, 0, at(8), vec![1, 2]),
        ],
    );
    let p = import(&w.finish()).unwrap();
    assert_eq!(p.kind, ProfileKind::Mixed);
    let s = names(&p, p.samples.stack[0]);
    assert_eq!(s, ["busy::main", "busy::spin"], "{s:?}");
    // Both addresses of busy::spin are one frame (one function, its first line).
    let spins: Vec<_> = p.frames.iter().filter(|f| f.function == "busy::spin").collect();
    assert_eq!(spins.len(), 1);
    assert_eq!(spins[0].kind, FrameKind::Native);
    assert_eq!(spins[0].line, Some(12));
    assert!(spins[0].file.as_deref().is_some_and(|f| f.ends_with("busy.rs")));
    assert_eq!(spins[0].module.as_deref(), Some("busy.exe"));
    assert!(p.notes.iter().any(|n| n.contains("named from symbols")), "{:?}", p.notes);
    let total = |m: MetricId| {
        let k = p.metric_index(m).unwrap();
        let n = p.metrics.len();
        (0..p.samples.len()).map(|i| p.samples.values[i * n + k]).sum::<u64>()
    };
    assert_eq!(total(MetricId::AllocBytes), 220);
    assert_eq!(total(MetricId::AllocCount), 3);
    assert_eq!(total(MetricId::LiveBytes), 80, "only the reallocated block is left");
    assert_eq!(
        p.metrics.iter().find(|m| m.id == MetricId::AllocBytes).unwrap().measurement,
        Measurement::Exact
    );
    assert!(p.notes.iter().any(|n| n.contains("80 bytes")), "{:?}", p.notes);
    assert!(p.notes.iter().any(|n| n.contains("allocated before the capture")), "{:?}", p.notes);
    assert!(p.notes.iter().any(|n| n.contains("damaged")), "{:?}", p.notes);
}
