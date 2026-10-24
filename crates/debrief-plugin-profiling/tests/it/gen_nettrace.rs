//! The committed `.nettrace` sample: how it is generated and what it contains.
//!
//! Regenerate it (and the fuzz seeds) with
//! `DEBRIEF_REGENERATE_SAMPLES=1 cargo test -p debrief-plugin-profiling --no-default-features --test it gen_nettrace:: -- --ignored`.
//! The trace is deterministic (a fixed LCG, no clock), so regenerating gives the same bytes.

use debrief_plugin_profiling::nettrace::writer::{ids, payload as p, Meta, Row, TraceHeader, Writer, RUNDOWN, RUNTIME, SAMPLE_PROFILER};
use debrief_plugin_profiling::nettrace::NettraceImporter;
use debrief_profile::{Capture, CaptureImporter, EventKind, Limits, MetricId, Profile};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn sample_path() -> PathBuf {
    root().join("samples/profiling/orders.nettrace")
}

/// A tiny deterministic generator (PCG-style LCG).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as u32
    }

    fn below(&mut self, n: u32) -> u32 {
        self.next() % n
    }
}

const FREQ: i64 = 10_000_000;
const SYNC: i64 = 0x0000_0017_8000_0000;
const MAIN_THREAD: u64 = 0x1f04;
const WORKER_1: u64 = 0x2a10;
const WORKER_2: u64 = 0x2a14;
const RUNDOWN_THREAD: u64 = 0x3300;
const NATIVE: u64 = 0x7ffd_4a21_5c30;

/// (module id, IL path).
const MODULES: [(u64, &str); 4] = [
    (0x7ffd_0001_0000, r"C:\apps\OrderService\Shop.dll"),
    (
        0x7ffd_0002_0000,
        r"C:\Program Files\dotnet\shared\Microsoft.NETCore.App\8.0.8\System.Data.SqlClient.dll",
    ),
    (
        0x7ffd_0003_0000,
        r"C:\Program Files\dotnet\shared\Microsoft.NETCore.App\8.0.8\System.Text.Json.dll",
    ),
    (
        0x7ffd_0004_0000,
        r"C:\Program Files\dotnet\shared\Microsoft.NETCore.App\8.0.8\System.Private.CoreLib.dll",
    ),
];

/// (module index, namespace, name).
const METHODS: [(usize, &str, &str); 11] = [
    (0, "Shop.Program", "Main"),
    (0, "Shop.Orders.OrderWorker", "Run"),
    (0, "Shop.Orders.OrderService", "PlaceOrder"),
    (0, "Shop.Orders.OrderService", "Validate"),
    (0, "Shop.Orders.OrderRepository", "Save"),
    (0, "Shop.Orders.PriceCalculator", "Total"),
    (1, "System.Data.SqlClient.SqlCommand", "ExecuteReader"),
    (1, "System.Data.SqlClient.TdsParser", "Run"),
    (2, "System.Text.Json.JsonSerializer", "Serialize"),
    (3, "System.Threading.Monitor", "Enter"),
    (3, "System.Threading.Thread", "StartCallback"),
];

fn method_start(i: usize) -> u64 {
    0x7ffd_8000_0000 + (i as u64) * 0x400
}

/// Call paths, root first, as method indexes (`usize::MAX` = the native leaf).
fn paths() -> Vec<Vec<usize>> {
    vec![
        vec![10, 1, 2, 4, 6, 7],          // 0 SQL
        vec![10, 1, 2, 8],                // 1 JSON
        vec![10, 1, 2, 3],                // 2 validate
        vec![10, 1, 2, 5],                // 3 pricing
        vec![10, 1, 2, 4, 9, usize::MAX], // 4 lock wait (native leaf)
        vec![0, 2, 8],                    // 5 main: JSON
        vec![0, 2, 3],                    // 6 main: validate
    ]
}

/// Addresses leaf first, slightly inside each method.
fn addresses(path: &[usize], lcg: &mut Lcg) -> Vec<u64> {
    path.iter()
        .rev()
        .map(|&m| {
            if m == usize::MAX {
                NATIVE
            } else {
                method_start(m) + 0x10 + u64::from(lcg.below(0x200))
            }
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Id {
    Sample = 1,
    JitStart,
    MethodLoad,
    MethodDcEnd,
    ModuleDcEnd,
    GcStart,
    GcEnd,
    Exception,
    ContentionStart,
    ContentionStop,
}

fn metas() -> Vec<Meta<'static>> {
    let m = |id: Id, provider, event_id, name| Meta {
        id: id as u32,
        provider,
        event_id,
        name,
    };
    vec![
        m(Id::Sample, SAMPLE_PROFILER, ids::THREAD_SAMPLE, "ThreadSample"),
        m(Id::JitStart, RUNTIME, ids::METHOD_JITTING_STARTED, ""),
        m(Id::MethodLoad, RUNTIME, ids::METHOD_LOAD_VERBOSE, ""),
        m(Id::MethodDcEnd, RUNDOWN, ids::METHOD_DC_END_VERBOSE, ""),
        m(Id::ModuleDcEnd, RUNDOWN, ids::MODULE_DC_END, ""),
        m(Id::GcStart, RUNTIME, ids::GC_START, ""),
        m(Id::GcEnd, RUNTIME, ids::GC_END, ""),
        m(Id::Exception, RUNTIME, ids::EXCEPTION_THROWN, ""),
        m(Id::ContentionStart, RUNTIME, ids::CONTENTION_START, ""),
        m(Id::ContentionStop, RUNTIME, ids::CONTENTION_STOP, ""),
    ]
}

/// One event before it is written: (ts, thread, metadata, path or none, payload).
struct Ev {
    ts: i64,
    thread: u64,
    id: Id,
    path: Option<usize>,
    payload: Vec<u8>,
}

fn ticks(ms: f64) -> i64 {
    SYNC + (ms * (FREQ as f64) / 1000.0) as i64
}

fn method_payload(i: usize) -> (u64, u64, &'static str, &'static str) {
    let (module, ns, name) = METHODS[i];
    (0x7ffd_9000_0000 + i as u64 * 8, MODULES[module].0, ns, name)
}

/// The events of the "OrderService" trace, in time order.
fn events() -> Vec<Ev> {
    let mut lcg = Lcg(0x5eed_0f0d_e125);
    let mut out = Vec::new();
    // JIT of every Shop method on the main thread at startup.
    for i in 0..6 {
        let (mid, module, ns, name) = method_payload(i);
        let t0 = 1.0 + i as f64 * 4.0;
        out.push(Ev {
            ts: ticks(t0),
            thread: MAIN_THREAD,
            id: Id::JitStart,
            path: None,
            payload: p::jitting_started(mid, module, ns, name),
        });
        out.push(Ev {
            ts: ticks(t0 + 0.4 + f64::from(lcg.below(25)) / 10.0),
            thread: MAIN_THREAD,
            id: Id::MethodLoad,
            path: None,
            payload: p::method_verbose(mid, module, method_start(i), 0x400, ns, name),
        });
    }
    // Main thread: busy for the first 600 ms.
    let mut t = 30.0;
    while t < 630.0 {
        let path = if lcg.below(3) == 0 { 6 } else { 5 };
        out.push(sample(t, MAIN_THREAD, path));
        t += 1.0 + f64::from(lcg.below(20)) / 100.0;
    }
    // Two workers for three seconds; worker 2 waits on a lock now and then.
    for (thread, start) in [(WORKER_1, 40.0), (WORKER_2, 55.0)] {
        let mut t = start;
        let mut exceptions = 0;
        while t < 3000.0 {
            let r = lcg.below(100);
            if thread == WORKER_2 && r < 3 {
                // A lock wait: contention events around a burst of samples in Monitor.Enter.
                let burst = 4 + lcg.below(12);
                out.push(Ev {
                    ts: ticks(t),
                    thread,
                    id: Id::ContentionStart,
                    path: Some(4),
                    payload: p::contention(false),
                });
                for k in 0..burst {
                    out.push(sample(t + f64::from(k) + 0.3, thread, 4));
                }
                t += f64::from(burst) + 0.2;
                out.push(Ev {
                    ts: ticks(t),
                    thread,
                    id: Id::ContentionStop,
                    path: Some(4),
                    payload: p::contention(false),
                });
                t += 0.5;
                continue;
            }
            let path = match r {
                0..=44 => 0,
                45..=64 => 1,
                65..=76 => 2,
                _ => 3,
            };
            if path == 2 && thread == WORKER_1 && exceptions < 4 && lcg.below(40) == 0 {
                exceptions += 1;
                out.push(Ev {
                    ts: ticks(t + 0.5),
                    thread,
                    id: Id::Exception,
                    path: Some(2),
                    payload: p::exception("Shop.Orders.InvalidOrderException", "Order has no lines"),
                });
            }
            out.push(sample(t, thread, path));
            t += 1.0 + f64::from(lcg.below(150)) / 100.0;
        }
    }
    // Garbage collections.
    for (n, at) in [210.0, 520.0, 805.0, 1130.0, 1490.0, 1810.0, 2200.0, 2610.0, 2900.0]
        .iter()
        .enumerate()
    {
        let generation = match n {
            4 => 1,
            7 => 2,
            _ => 0,
        };
        let count = n as u32 + 1;
        out.push(Ev {
            ts: ticks(*at),
            thread: WORKER_1,
            id: Id::GcStart,
            path: None,
            payload: p::gc_start(count, generation, 0),
        });
        out.push(Ev {
            ts: ticks(at + 1.5 + f64::from(generation) * 4.0 + f64::from(lcg.below(10)) / 10.0),
            thread: WORKER_1,
            id: Id::GcEnd,
            path: None,
            payload: p::gc_end(count, generation),
        });
    }
    out.sort_by_key(|e| e.ts);
    out
}

fn sample(ms: f64, thread: u64, path: usize) -> Ev {
    Ev {
        ts: ticks(ms),
        thread,
        id: Id::Sample,
        path: Some(path),
        payload: p::sample(path != 4),
    }
}

/// Writes the OrderService trace: stacks and events per 500 ms sequence-point region,
/// event blocks per 100 ms, rundown at the end.
fn orders_trace() -> Vec<u8> {
    let header = TraceHeader {
        sync_time: [2026, 10, 6, 17, 8, 15, 42, 250],
        sync_qpc: SYNC,
        qpc_freq: FREQ,
        process_id: 18_244,
        processors: 8,
        ..TraceHeader::default()
    };
    let mut w = Writer::new(&header);
    w.metadata(true, &metas());
    let mut lcg = Lcg(42);
    let paths = paths();
    let mut seq: HashMap<u64, u32> = HashMap::new();
    let events = events();
    let mut i = 0;
    let mut region_end = ticks(500.0);
    while i < events.len() {
        // Stack ids restart after each sequence point; write this region's stacks.
        let stacks: Vec<Vec<u64>> = paths.iter().map(|p| addresses(p, &mut lcg)).collect();
        w.stacks(1, &stacks);
        while i < events.len() && events[i].ts < region_end {
            let block_end = (events[i].ts + ticks(100.0) - SYNC).min(region_end);
            let mut rows = Vec::new();
            while i < events.len() && events[i].ts < block_end {
                let e = &events[i];
                let s = seq.entry(e.thread).or_insert(0);
                *s += 1;
                rows.push(Row {
                    metadata_id: e.id as u32,
                    sequence: *s,
                    thread: e.thread,
                    capture_thread: e.thread,
                    processor: (e.thread % 8) as u32,
                    stack_id: e.path.map_or(0, |p| p as u32 + 1),
                    ts: e.ts,
                    sorted: rows.is_empty(),
                    payload: e.payload.clone(),
                });
                i += 1;
            }
            w.events(true, &rows);
        }
        let threads: Vec<(u64, u32)> = [MAIN_THREAD, WORKER_1, WORKER_2]
            .iter()
            .map(|t| (*t, seq.get(t).copied().unwrap_or(0)))
            .collect();
        w.sequence_point(region_end, &threads);
        region_end += ticks(500.0) - SYNC;
    }
    // Rundown: modules and every method.
    let end = ticks(3010.0);
    let mut rows: Vec<Row> = MODULES
        .iter()
        .map(|(id, path)| Row {
            metadata_id: Id::ModuleDcEnd as u32,
            thread: RUNDOWN_THREAD,
            capture_thread: RUNDOWN_THREAD,
            ts: end,
            payload: p::module_load(*id, path),
            ..Row::default()
        })
        .collect();
    for i in 0..METHODS.len() {
        let (mid, module, ns, name) = method_payload(i);
        rows.push(Row {
            metadata_id: Id::MethodDcEnd as u32,
            thread: RUNDOWN_THREAD,
            capture_thread: RUNDOWN_THREAD,
            ts: end + 10,
            payload: p::method_verbose(mid, module, method_start(i), 0x400, ns, name),
            ..Row::default()
        });
    }
    let mut n = 0;
    for r in &mut rows {
        n += 1;
        r.sequence = n;
    }
    w.events(true, &rows);
    w.finish()
}

/// Small valid traces for the fuzz corpus.
fn seeds() -> Vec<(&'static str, Vec<u8>)> {
    let small = |compressed: bool, pointer_size: u32| {
        let h = TraceHeader {
            pointer_size,
            ..TraceHeader::default()
        };
        let mut w = Writer::new(&h);
        w.metadata(compressed, &metas());
        w.stacks(1, &[vec![0x2004, 0x1008]]);
        let row = |id: Id, stack_id, ts, payload| Row {
            metadata_id: id as u32,
            sequence: 1,
            thread: 7,
            capture_thread: 7,
            stack_id,
            ts,
            payload,
            ..Row::default()
        };
        let (mid, module, ns, name) = method_payload(2);
        w.events(
            compressed,
            &[
                row(Id::Sample, 1, h.sync_qpc + 10_000, p::sample(true)),
                row(Id::GcStart, 0, h.sync_qpc + 20_000, p::gc_start(1, 0, 0)),
                row(Id::GcEnd, 0, h.sync_qpc + 30_000, p::gc_end(1, 0)),
                row(Id::Exception, 1, h.sync_qpc + 40_000, p::exception("E", "m")),
                row(Id::ContentionStart, 1, h.sync_qpc + 50_000, p::contention(false)),
                row(Id::ContentionStop, 1, h.sync_qpc + 60_000, p::contention(false)),
            ],
        );
        w.sequence_point(h.sync_qpc + 70_000, &[(7, 6)]);
        w.events(
            compressed,
            &[
                row(Id::ModuleDcEnd, 0, h.sync_qpc + 80_000, p::module_load(module, "Shop.dll")),
                row(
                    Id::MethodDcEnd,
                    0,
                    h.sync_qpc + 80_000,
                    p::method_verbose(mid, module, 0x2000, 64, ns, name),
                ),
            ],
        );
        w.finish()
    };
    vec![
        ("compressed.nettrace", small(true, 8)),
        ("uncompressed.nettrace", small(false, 8)),
        ("pointer32.nettrace", small(true, 4)),
    ]
}

fn import(bytes: &[u8]) -> Profile {
    match NettraceImporter.import("orders.nettrace", bytes, &Limits::default()).unwrap() {
        Capture::Profile(p) => p,
        Capture::Heap(_) => panic!("not a profile"),
    }
}

#[test]
#[ignore = "writes samples/profiling/orders.nettrace and the fuzz seeds; run with DEBRIEF_REGENERATE_SAMPLES=1"]
fn regenerate_sample() {
    if std::env::var("DEBRIEF_REGENERATE_SAMPLES").as_deref() != Ok("1") {
        eprintln!("set DEBRIEF_REGENERATE_SAMPLES=1 to write the sample");
        return;
    }
    let path = sample_path();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, orders_trace()).unwrap();
    let corpus = root().join("fuzz/corpus/fuzz_prof_nettrace");
    std::fs::create_dir_all(&corpus).unwrap();
    for (name, bytes) in seeds() {
        std::fs::write(corpus.join(name), bytes).unwrap();
    }
}

#[test]
fn committed_sample_is_up_to_date() {
    let committed = std::fs::read(sample_path()).expect("samples/profiling/orders.nettrace exists (see regenerate_sample)");
    assert!(
        committed == orders_trace(),
        "the sample is stale: regenerate it (see the module docs)"
    );
    assert!(committed.len() < 300 * 1024, "{} bytes", committed.len());
}

#[test]
fn committed_sample_has_the_expected_shape() {
    let p = import(&std::fs::read(sample_path()).unwrap());
    assert_eq!(p.name, "orders");
    assert_eq!(p.threads.len(), 3, "{:?}", p.threads);
    let cpu = p.metric_index(MetricId::Samples).unwrap();
    let stride = p.metrics.len();
    let samples: u64 = p.samples.values.chunks(stride).map(|v| v[cpu]).sum();
    assert!((2_000..=7_000).contains(&samples), "{samples} samples");
    assert!(p.duration_ns > 2_900_000_000 && p.duration_ns < 3_100_000_000, "{}", p.duration_ns);
    assert!(p.metric_index(MetricId::Exceptions).is_some() && p.metric_index(MetricId::WaitNs).is_some());

    // Self samples per function (leaf frames).
    let mut by_leaf: HashMap<&str, u64> = HashMap::new();
    for (i, &s) in p.samples.stack.iter().enumerate() {
        let leaf = *p.stack_frames(s).last().unwrap();
        *by_leaf.entry(p.frames[leaf as usize].function.as_str()).or_default() += p.samples.values[i * stride + cpu];
    }
    let mut top: Vec<(&str, u64)> = by_leaf.into_iter().collect();
    top.sort_by_key(|(n, c)| (std::cmp::Reverse(*c), *n));
    assert_eq!(top[0].0, "System.Data.SqlClient.TdsParser.Run", "{top:?}");
    assert!(top.iter().any(|(n, _)| *n == "System.Text.Json.JsonSerializer.Serialize"));
    let lock: u64 = top.iter().filter(|(n, _)| n.starts_with("0x")).map(|(_, c)| c).sum();
    let share = lock as f64 / samples as f64;
    assert!((0.05..0.2).contains(&share), "lock wait share {share}");
    let place = p
        .frames
        .iter()
        .find(|f| f.function == "Shop.Orders.OrderService.PlaceOrder")
        .unwrap();
    assert_eq!(place.module.as_deref(), Some("Shop.dll"));
    assert!(!place.is_framework);
    let corelib = p.frames.iter().find(|f| f.function == "System.Threading.Monitor.Enter").unwrap();
    assert_eq!(corelib.module.as_deref(), Some("System.Private.CoreLib.dll"));

    for kind in [EventKind::Gc, EventKind::Jit, EventKind::Exception, EventKind::LockWait] {
        assert!(p.events.iter().any(|e| e.kind == kind), "{kind:?}");
    }
    let exceptions = p.events.iter().filter(|e| e.kind == EventKind::Exception).count();
    assert!((1..=4).contains(&exceptions));
    assert!(p.events.iter().any(|e| e.label == "Shop.Orders.InvalidOrderException"));
    assert!(p.events.iter().filter(|e| e.kind == EventKind::Gc).all(|e| e.dur_ns > 0));
    assert_eq!(p.notes.len(), 1, "only the native lock frame is unresolved: {:?}", p.notes);
    assert_eq!(p.started_ms, Some(1_792_224_942_250));
}

#[test]
fn fuzz_seeds_import() {
    for (name, bytes) in seeds() {
        let p = import(&bytes);
        assert_eq!(p.samples.len(), 3, "{name}");
        assert_eq!(
            p.frames
                .iter()
                .filter(|f| f.function == "Shop.Orders.OrderService.PlaceOrder")
                .count(),
            1,
            "{name}"
        );
    }
}

/// Imports a real trace (e.g. one of PerfView's test inputs) for manual checks:
/// `DEBRIEF_NETTRACE_FILE=path cargo test ... --test it -- --ignored --nocapture gen_nettrace::real_trace`.
#[test]
#[ignore = "needs DEBRIEF_NETTRACE_FILE"]
fn real_trace() {
    let Ok(path) = std::env::var("DEBRIEF_NETTRACE_FILE") else { return };
    let bytes = std::fs::read(&path).unwrap();
    let c = NettraceImporter.import(&path, &bytes, &Limits::default());
    let p = match c {
        Ok(Capture::Profile(p)) => p,
        other => panic!("{other:?}"),
    };
    println!(
        "threads {} samples {} frames {} events {} duration {} ms started {:?}",
        p.threads.len(),
        p.samples.len(),
        p.frames.len(),
        p.events.len(),
        p.duration_ns / 1_000_000,
        p.started_ms
    );
    println!("metrics {:?}", p.metrics.iter().map(|m| m.id).collect::<Vec<_>>());
    println!("notes {:?}", p.notes);
    let mut by_leaf: HashMap<&str, usize> = HashMap::new();
    for &s in &p.samples.stack {
        let leaf = *p.stack_frames(s).last().unwrap();
        *by_leaf.entry(p.frames[leaf as usize].function.as_str()).or_default() += 1;
    }
    let mut top: Vec<_> = by_leaf.into_iter().collect();
    top.sort_by_key(|(n, c)| (std::cmp::Reverse(*c), *n));
    for (n, c) in top.iter().take(15) {
        println!("{c:>6} {n}");
    }
    let mut kinds: HashMap<String, usize> = HashMap::new();
    for e in &p.events {
        *kinds.entry(format!("{:?}", e.kind)).or_default() += 1;
    }
    println!("events {kinds:?}");
    if let Some(s) = p.samples.stack.iter().max_by_key(|&&s| p.stack_frames(s).len()) {
        for f in p.stack_frames(*s) {
            let f = &p.frames[f as usize];
            println!("  {} [{:?}] {:?}", f.function, f.module, f.kind);
        }
    }
}
