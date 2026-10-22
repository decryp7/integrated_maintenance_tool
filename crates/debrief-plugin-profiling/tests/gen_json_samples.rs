//! The committed JSON profiling samples: how they are generated and what they contain.
//!
//! Regenerate them (and the fuzz seeds) with
//! `DEBRIEF_REGENERATE_SAMPLES=1 cargo test -p debrief-plugin-profiling --no-default-features --test gen_json_samples -- --ignored`.
//! Everything is deterministic (a fixed SplitMix64 generator, no clock), so
//! regenerating gives the same bytes; `committed_samples_are_up_to_date` checks that.
//!
//! The fictional app is the OrderService of the log samples:
//!
//! | File | Contents |
//! |---|---|
//! | `samples/profiling/orders.speedscope.json` | sampled, 2 threads × 1000 samples (ms): JSON serialisation ~10%, `(garbage collector)` ~8%, `System.Threading.Monitor.Enter` ~6% |
//! | `samples/profiling/orders-regression.speedscope.json` | the same app after a regression: serialisation ~30%, new `Shop.Orders.AuditLog.Write` ~5% |
//! | `samples/profiling/orders-evented.speedscope.json` | evented (µs): 40 nested order requests and 20 outbox dispatches with exact timings |
//! | `samples/profiling/orders.cpuprofile` | Node.js version: `placeOrder`, `saveOrder`, `JSON.stringify`, `(garbage collector)` ~15%, `(idle)`, `(program)` |
//! | `samples/profiling/orders.trace.json` | Chrome trace: `Profile` + 3 `ProfileChunk`s for the main thread, `X` tasks on a worker, GC events, thread names |
//! | `samples/profiling/heap-{1,2,3}.heapsnapshot` | V8 heap snapshots: an `OrderCache` `Map` holding 50 / 100 / 150 `Order`s; `AuditLog`/`AuditEntry` appear from snapshot 2; a weak and a shortcut edge |

use debrief_plugin_profiling::chrome::{CpuProfileImporter, TraceEventImporter};
use debrief_plugin_profiling::heapsnapshot::HeapSnapshotImporter;
use debrief_plugin_profiling::speedscope::SpeedscopeImporter;
use debrief_profile::heap::HeapSnapshot;
use debrief_profile::{Capture, CaptureImporter, EventKind, Limits, Measurement, MetricId, Profile, ProfileKind};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// SplitMix64: tiny, deterministic.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.below(hi - lo + 1)
    }

    /// Index into `weights` drawn proportionally.
    fn pick(&mut self, weights: &[u64]) -> usize {
        let total: u64 = weights.iter().sum();
        let mut x = self.below(total);
        for (i, &w) in weights.iter().enumerate() {
            if x < w {
                return i;
            }
            x -= w;
        }
        weights.len() - 1
    }
}

// ---------------------------------------------------------------- speedscope

const SS_FRAMES: &[(&str, Option<&str>, Option<u32>)] = &[
    ("Shop.Orders.Program.Main", Some("src/Shop.Orders/Program.cs"), Some(12)),
    (
        "Shop.Orders.Api.OrdersController.PlaceOrder",
        Some("src/Shop.Orders/Api/OrdersController.cs"),
        Some(41),
    ),
    (
        "Shop.Orders.OrderService.PlaceOrder",
        Some("src/Shop.Orders/OrderService.cs"),
        Some(58),
    ),
    (
        "Shop.Orders.OrderService.Validate",
        Some("src/Shop.Orders/OrderService.cs"),
        Some(102),
    ),
    (
        "Shop.Orders.Pricing.PriceCalculator.Calculate",
        Some("src/Shop.Orders/Pricing/PriceCalculator.cs"),
        Some(23),
    ),
    (
        "Shop.Orders.Data.OrderRepository.Save",
        Some("src/Shop.Orders/Data/OrderRepository.cs"),
        Some(77),
    ),
    ("System.Data.SqlClient.SqlCommand.ExecuteNonQuery", None, None),
    ("System.Text.Json.JsonSerializer.Serialize", None, None),
    ("System.Text.Json.Serialization.Converters.ObjectConverter.Write", None, None),
    ("(garbage collector)", None, None),
    (
        "Shop.Orders.Inventory.StockCache.Reserve",
        Some("src/Shop.Orders/Inventory/StockCache.cs"),
        Some(35),
    ),
    ("System.Threading.Monitor.Enter", None, None),
    ("System.Threading.ThreadPoolWorkQueue.Dispatch", None, None),
    (
        "Shop.Orders.Outbox.OutboxDispatcher.Dispatch",
        Some("src/Shop.Orders/Outbox/OutboxDispatcher.cs"),
        Some(64),
    ),
    ("System.Net.Http.HttpClient.SendAsync", None, None),
    ("Shop.Orders.AuditLog.Write", Some("src/Shop.Orders/AuditLog.cs"), Some(19)),
    (
        "Shop.Orders.Api.OrdersController.GetOrder",
        Some("src/Shop.Orders/Api/OrdersController.cs"),
        Some(88),
    ),
    (
        "Shop.Orders.Data.OrderRepository.Find",
        Some("src/Shop.Orders/Data/OrderRepository.cs"),
        Some(40),
    ),
];

const SERIALIZE: &str = "System.Text.Json.JsonSerializer.Serialize";
const GC: &str = "(garbage collector)";
const LOCK: &str = "System.Threading.Monitor.Enter";
const AUDIT: &str = "Shop.Orders.AuditLog.Write";

fn ss_frames() -> Value {
    Value::Array(
        SS_FRAMES
            .iter()
            .map(|(name, file, line)| {
                let mut f = json!({ "name": name });
                if let Some(file) = file {
                    f["file"] = json!(file);
                }
                if let Some(line) = line {
                    f["line"] = json!(line);
                }
                f
            })
            .collect(),
    )
}

/// Stacks (frame indices, root first) with their share in percent.
type Mix = Vec<(Vec<u32>, u64)>;

/// The stack mix of each thread, before and after the regression.
fn ss_mix(regression: bool) -> [(&'static str, Mix); 2] {
    let request = if regression {
        vec![
            (vec![0, 1, 2, 7, 8], 30),
            (vec![9], 8),
            (vec![0, 1, 2, 10, 11], 6),
            (vec![0, 1, 2, 3], 8),
            (vec![0, 1, 2, 4], 12),
            (vec![0, 1, 2, 5, 6], 18),
            (vec![0, 1, 2], 3),
            (vec![0, 16, 17, 6], 10),
            (vec![0, 1, 2, 15], 5),
        ]
    } else {
        vec![
            (vec![0, 1, 2, 7, 8], 10),
            (vec![9], 8),
            (vec![0, 1, 2, 10, 11], 6),
            (vec![0, 1, 2, 3], 12),
            (vec![0, 1, 2, 4], 20),
            (vec![0, 1, 2, 5, 6], 24),
            (vec![0, 1, 2], 5),
            (vec![0, 16, 17, 6], 15),
        ]
    };
    let outbox = if regression {
        vec![
            (vec![12, 13, 7, 8], 30),
            (vec![9], 8),
            (vec![12, 13, 11], 6),
            (vec![12, 13, 14], 40),
            (vec![12, 13], 11),
            (vec![12, 13, 15], 5),
        ]
    } else {
        vec![
            (vec![12, 13, 7, 8], 10),
            (vec![9], 8),
            (vec![12, 13, 11], 6),
            (vec![12, 13, 14], 50),
            (vec![12, 13], 26),
        ]
    };
    [("Request thread", request), ("Outbox worker", outbox)]
}

fn speedscope_sampled(regression: bool) -> Vec<u8> {
    let mut rng = Rng(if regression { 0x5eed_0002 } else { 0x5eed_0001 });
    let profiles: Vec<Value> = ss_mix(regression)
        .into_iter()
        .map(|(name, mix)| {
            let weights: Vec<u64> = mix.iter().map(|m| m.1).collect();
            let mut samples = Vec::new();
            let mut times = Vec::new();
            let mut total = 0.0;
            for _ in 0..1000 {
                let i = rng.pick(&weights);
                samples.push(json!(mix[i].0));
                // 1 ms sampling interval with ±10% jitter, in steps of 0.05 ms.
                let w = 0.9 + rng.below(5) as f64 * 0.05;
                let w = (w * 100.0).round() / 100.0;
                total += w;
                times.push(json!(w));
            }
            json!({
                "type": "sampled", "name": name, "unit": "milliseconds",
                "startValue": 0, "endValue": (total * 100.0_f64).round() / 100.0,
                "samples": samples, "weights": times,
            })
        })
        .collect();
    let file = json!({
        "$schema": "https://www.speedscope.app/file-format-schema.json",
        "name": if regression { "OrderService (after release 2.4)" } else { "OrderService" },
        "exporter": "debrief sample generator",
        "activeProfileIndex": 0,
        "shared": { "frames": ss_frames() },
        "profiles": profiles,
    });
    serde_json::to_vec(&file).unwrap()
}

fn speedscope_evented() -> Vec<u8> {
    let mut rng = Rng(0x5eed_0003);
    let mut events = Vec::new();
    let mut t = 0u64;
    let ev = |events: &mut Vec<Value>, kind: &str, frame: u32, at: u64| events.push(json!({"type": kind, "frame": frame, "at": at}));
    ev(&mut events, "O", 0, t);
    for _ in 0..40 {
        t += rng.range(50, 200);
        ev(&mut events, "O", 1, t);
        t += 10;
        ev(&mut events, "O", 2, t);
        for (frame, lo, hi) in [(3, 80, 120), (4, 150, 250)] {
            t += 5;
            ev(&mut events, "O", frame, t);
            t += rng.range(lo, hi);
            ev(&mut events, "C", frame, t);
        }
        t += 5;
        ev(&mut events, "O", 10, t);
        t += 5;
        ev(&mut events, "O", 11, t);
        t += rng.range(0, 60);
        ev(&mut events, "C", 11, t);
        t += 15;
        ev(&mut events, "C", 10, t);
        ev(&mut events, "O", 5, t);
        t += 20;
        ev(&mut events, "O", 6, t);
        t += rng.range(400, 900);
        ev(&mut events, "C", 6, t);
        t += 10;
        ev(&mut events, "C", 5, t);
        ev(&mut events, "O", 7, t);
        t += 10;
        ev(&mut events, "O", 8, t);
        t += rng.range(100, 160);
        ev(&mut events, "C", 8, t);
        ev(&mut events, "C", 7, t);
        t += 30;
        ev(&mut events, "C", 2, t);
        t += 10;
        ev(&mut events, "C", 1, t);
    }
    t += 100;
    ev(&mut events, "C", 0, t);
    let request_end = t;

    let mut outbox = Vec::new();
    let mut t = 0u64;
    ev(&mut outbox, "O", 12, t);
    for _ in 0..20 {
        t += rng.range(200, 600);
        ev(&mut outbox, "O", 13, t);
        t += 20;
        ev(&mut outbox, "O", 14, t);
        t += rng.range(500, 2500);
        ev(&mut outbox, "C", 14, t);
        t += 15;
        ev(&mut outbox, "C", 13, t);
    }
    t += 50;
    ev(&mut outbox, "C", 12, t);
    let file = json!({
        "$schema": "https://www.speedscope.app/file-format-schema.json",
        "name": "OrderService (instrumented)",
        "exporter": "debrief sample generator",
        "shared": { "frames": ss_frames() },
        "profiles": [
            {"type": "evented", "name": "Request thread", "unit": "microseconds", "startValue": 0, "endValue": request_end, "events": events},
            {"type": "evented", "name": "Outbox worker", "unit": "microseconds", "startValue": 0, "endValue": t, "events": outbox},
        ],
    });
    serde_json::to_vec(&file).unwrap()
}

// ---------------------------------------------------------------- V8 CPU profile

/// (id, function, url, 0-based line, parent id, percent of samples).
const JS_NODES: &[(u64, &str, &str, i64, u64, u64)] = &[
    (1, "(root)", "", -1, 0, 0),
    (2, "(program)", "", -1, 1, 5),
    (3, "(idle)", "", -1, 1, 20),
    (4, "(garbage collector)", "", -1, 1, 15),
    (5, "processTicksAndRejections", "node:internal/process/task_queues", 66, 1, 0),
    (6, "handleRequest", "file:///srv/orders/app.js", 40, 5, 3),
    (7, "placeOrder", "file:///srv/orders/orders.js", 11, 6, 4),
    (8, "validateOrder", "file:///srv/orders/orders.js", 57, 7, 8),
    (9, "priceOrder", "file:///srv/orders/pricing.js", 4, 7, 12),
    (10, "saveOrder", "file:///srv/orders/orders.js", 87, 7, 3),
    (11, "query", "file:///srv/orders/node_modules/pg/lib/client.js", 511, 10, 10),
    (12, "JSON.stringify", "", -1, 10, 6),
    (13, "JSON.stringify", "", -1, 6, 6),
    (14, "Writable.write", "node:internal/streams/writable", 335, 6, 4),
    (15, "JSON.parse", "", -1, 6, 4),
];

fn js_call_frame(n: &(u64, &str, &str, i64, u64, u64), numeric_script: bool) -> Value {
    let script = if n.2.is_empty() { 0 } else { 40 + n.0 };
    json!({
        "functionName": n.1,
        "scriptId": if numeric_script { json!(script) } else { json!(script.to_string()) },
        "url": n.2,
        "lineNumber": n.3,
        "columnNumber": if n.3 < 0 { -1 } else { 2 },
    })
}

/// Sampled node ids and their time deltas (µs).
fn js_samples(rng: &mut Rng, n: usize) -> (Vec<u64>, Vec<u64>) {
    let weights: Vec<u64> = JS_NODES.iter().map(|n| n.5).collect();
    (0..n).map(|_| (JS_NODES[rng.pick(&weights)].0, rng.range(400, 600))).unzip()
}

fn cpuprofile() -> Vec<u8> {
    let mut rng = Rng(0x5eed_0004);
    let (samples, deltas) = js_samples(&mut rng, 2000);
    let mut hits: HashMap<u64, u64> = HashMap::new();
    for s in &samples {
        *hits.entry(*s).or_default() += 1;
    }
    let nodes: Vec<Value> = JS_NODES
        .iter()
        .map(|n| {
            let children: Vec<u64> = JS_NODES.iter().filter(|c| c.4 == n.0).map(|c| c.0).collect();
            let mut v = json!({"id": n.0, "callFrame": js_call_frame(n, false), "hitCount": hits.get(&n.0).copied().unwrap_or(0)});
            if !children.is_empty() {
                v["children"] = json!(children);
            }
            v
        })
        .collect();
    let start = 81_234_567_890u64;
    let end = start + deltas.iter().sum::<u64>() + 400;
    let file = json!({"nodes": nodes, "startTime": start, "endTime": end, "samples": samples, "timeDeltas": deltas});
    serde_json::to_vec(&file).unwrap()
}

fn trace() -> Vec<u8> {
    let mut rng = Rng(0x5eed_0005);
    let (pid, main, worker) = (4242, 1, 7);
    let start = 5_000_000u64;
    let mut events = vec![
        json!({"ph": "M", "name": "process_name", "pid": pid, "tid": 0, "ts": 0, "args": {"name": "node orders"}}),
        json!({"ph": "M", "name": "thread_name", "pid": pid, "tid": main, "ts": 0, "args": {"name": "CrRendererMain"}}),
        json!({"ph": "M", "name": "thread_name", "pid": pid, "tid": worker, "ts": 0, "args": {"name": "ThreadPoolForegroundWorker"}}),
        json!({"ph": "P", "name": "Profile", "cat": "disabled-by-default-v8.cpu_profiler", "id": "0x1", "pid": pid, "tid": main, "ts": start,
               "args": {"data": {"startTime": start}}}),
    ];
    let parent: HashMap<u64, u64> = JS_NODES.iter().map(|n| (n.0, n.4)).collect();
    let mut introduced = std::collections::HashSet::new();
    let mut t = start;
    for chunk in 0..3 {
        let (samples, deltas) = js_samples(&mut rng, 200);
        let mut nodes = Vec::new();
        for s in &samples {
            // Introduce each sampled node's ancestors (root first) the first time they are used.
            let mut chain = vec![*s];
            while let Some(&p) = parent.get(chain.last().unwrap()).filter(|&&p| p != 0) {
                chain.push(p);
            }
            for id in chain.into_iter().rev() {
                if introduced.insert(id) {
                    let n = JS_NODES.iter().find(|n| n.0 == id).unwrap();
                    let mut v = json!({"id": id, "callFrame": js_call_frame(n, true)});
                    if n.4 != 0 {
                        v["parent"] = json!(n.4);
                    }
                    nodes.push(v);
                }
            }
        }
        t += deltas.iter().sum::<u64>();
        events.push(
            json!({"ph": "P", "name": "ProfileChunk", "cat": "disabled-by-default-v8.cpu_profiler", "id": "0x1", "pid": pid,
            "tid": 9 + chunk, "ts": t, "args": {"data": {"cpuProfile": {"nodes": nodes, "samples": samples}, "timeDeltas": deltas}}}),
        );
    }
    let mut w = start + 1_000;
    for i in 0..6 {
        let dur = rng.range(200, 800);
        events.push(
            json!({"ph": "X", "name": "ThreadControllerImpl::RunTask", "cat": "toplevel", "pid": pid, "tid": worker, "ts": w, "dur": dur}),
        );
        events.push(json!({"ph": "X", "name": "v8.compile", "cat": "v8", "pid": pid, "tid": worker, "ts": w + 20, "dur": dur / 3}));
        if i % 2 == 0 {
            events.push(json!({"ph": "i", "name": "MinorGC", "cat": "v8", "s": "t", "pid": pid, "tid": main, "ts": w + 100}));
        }
        w += dur + rng.range(5_000, 40_000);
    }
    events.push(json!({"ph": "X", "name": "MajorGC", "cat": "v8", "pid": pid, "tid": main, "ts": start + 150_000, "dur": 2_400}));
    // Written by hand to keep Chrome's key order (`traceEvents` first; serde_json sorts keys).
    let events: Vec<String> = events.iter().map(Value::to_string).collect();
    format!(
        "{{\"traceEvents\":[\n{}\n],\n\"metadata\":{{\"source\":\"debrief sample generator\"}}}}\n",
        events.join(",\n")
    )
    .into_bytes()
}

// ---------------------------------------------------------------- V8 heap snapshot

const NODE_TYPES: &[&str] = &[
    "hidden",
    "array",
    "string",
    "object",
    "code",
    "closure",
    "regexp",
    "number",
    "native",
    "synthetic",
    "concatenated string",
    "sliced string",
    "symbol",
    "bigint",
    "object shape",
];
const EDGE_TYPES: &[&str] = &["context", "element", "property", "internal", "hidden", "shortcut", "weak"];

/// An edge: (type, name_or_index, target node).
type HeapEdge = (u32, u32, usize);

/// A heap graph under construction, written in V8's layout.
#[derive(Default)]
struct Heap {
    strings: Vec<String>,
    string_index: HashMap<String, u32>,
    /// (type, name, self_size, edges)
    nodes: Vec<(u32, u32, u32, Vec<HeapEdge>)>,
}

impl Heap {
    fn s(&mut self, s: &str) -> u32 {
        if let Some(&i) = self.string_index.get(s) {
            return i;
        }
        self.strings.push(s.to_string());
        self.string_index.insert(s.to_string(), self.strings.len() as u32 - 1);
        self.strings.len() as u32 - 1
    }

    fn node(&mut self, ty: &str, name: &str, size: u32) -> usize {
        let t = NODE_TYPES.iter().position(|x| *x == ty).unwrap() as u32;
        let n = self.s(name);
        self.nodes.push((t, n, size, Vec::new()));
        self.nodes.len() - 1
    }

    /// An edge named by a string (`property`, `context`, `internal`, `weak`, `shortcut`).
    fn named(&mut self, from: usize, ty: &str, name: &str, to: usize) {
        let t = EDGE_TYPES.iter().position(|x| *x == ty).unwrap() as u32;
        let n = self.s(name);
        self.nodes[from].3.push((t, n, to));
    }

    /// An edge with an index (`element`, `hidden`).
    fn indexed(&mut self, from: usize, ty: &str, index: u32, to: usize) {
        let t = EDGE_TYPES.iter().position(|x| *x == ty).unwrap() as u32;
        self.nodes[from].3.push((t, index, to));
    }

    /// A JS array: the `Array` object and its backing store holding `items`.
    fn array(&mut self, from: usize, name: &str, items: &[usize]) {
        let a = self.node("object", "Array", 32);
        let store = self.node("array", "", 16 + 8 * items.len() as u32);
        self.named(from, "property", name, a);
        self.named(a, "internal", "elements", store);
        for (i, &it) in items.iter().enumerate() {
            self.indexed(store, "element", i as u32, it);
        }
    }

    fn write(&self) -> Vec<u8> {
        const NF: usize = 7;
        let edge_count: usize = self.nodes.iter().map(|n| n.3.len()).sum();
        let meta = json!({
            "node_fields": ["type", "name", "id", "self_size", "edge_count", "trace_node_id", "detachedness"],
            "node_types": [NODE_TYPES, "string", "number", "number", "number", "number", "number"],
            "edge_fields": ["type", "name_or_index", "to_node"],
            "edge_types": [EDGE_TYPES, "string_or_number", "node"],
            "trace_function_info_fields": ["function_id", "name", "script_name", "script_id", "line", "column"],
            "trace_node_fields": ["id", "function_info_index", "count", "size", "children"],
            "sample_fields": ["timestamp_us", "last_assigned_id"],
            "location_fields": ["object_index", "script_id", "line", "column"],
        });
        let mut out = format!(
            "{{\"snapshot\":{{\"meta\":{meta},\n\"node_count\":{},\n\"edge_count\":{edge_count},\n\"trace_function_count\":0}},\n\"nodes\":[",
            self.nodes.len()
        );
        for (i, (t, name, size, edges)) in self.nodes.iter().enumerate() {
            let sep = if i == 0 { "" } else { ",\n" };
            out.push_str(&format!("{sep}{t},{name},{},{size},{},0,0", 2 * i + 1, edges.len()));
        }
        out.push_str("],\n\"edges\":[");
        let mut first = true;
        for (_, _, _, edges) in &self.nodes {
            for (t, n, to) in edges {
                let sep = if first { "" } else { ",\n" };
                first = false;
                out.push_str(&format!("{sep}{t},{n},{}", to * NF));
            }
        }
        out.push_str("],\n\"trace_function_infos\":[],\n\"trace_tree\":[],\n\"samples\":[],\n\"locations\":[],\n\"strings\":[");
        for (i, s) in self.strings.iter().enumerate() {
            let sep = if i == 0 { "" } else { ",\n" };
            out.push_str(&format!("{sep}{}", serde_json::to_string(s).unwrap()));
        }
        out.push_str("]}\n");
        out.into_bytes()
    }
}

fn heap_snapshot(orders: usize, audit_entries: usize) -> Vec<u8> {
    let mut h = Heap::default();
    let root = h.node("synthetic", "", 0);
    let gc_roots = h.node("synthetic", "(GC roots)", 0);
    let strong = h.node("synthetic", "(Strong roots)", 0);
    let handles = h.node("synthetic", "(Global handles)", 0);
    let global = h.node("object", "global", 64);
    h.indexed(root, "element", 1, gc_roots);
    h.named(root, "shortcut", "global", global);
    h.indexed(gc_roots, "element", 0, strong);
    h.indexed(gc_roots, "element", 1, handles);
    h.indexed(strong, "element", 0, global);
    h.indexed(handles, "element", 0, global);
    let native_context = h.node("hidden", "system / NativeContext", 64);
    h.indexed(global, "hidden", 3, native_context);

    let logger = h.node("object", "Logger", 32);
    h.named(global, "property", "logger", logger);
    let prefix = h.node("string", "orders", 24);
    h.named(logger, "property", "prefix", prefix);
    let config = h.node("object", "Config", 40);
    h.named(global, "property", "config", config);
    for (k, v) in [("dbUrl", "postgres://db01/orders"), ("region", "eu-west")] {
        let s = h.node("string", v, 32);
        h.named(config, "property", k, s);
    }
    let weak_ref = h.node("object", "WeakRef", 24);
    h.named(global, "property", "configRef", weak_ref);
    h.named(weak_ref, "weak", "target", config);
    let pattern = h.node("regexp", "^order-\\d+$", 32);
    h.named(global, "property", "orderIdPattern", pattern);

    let mut handlers = Vec::new();
    for name in ["onOrder", "onRefund", "onHealth"] {
        let f = h.node("closure", name, 32);
        let ctx = h.node("hidden", "system / Context", 40);
        let code = h.node("code", name, 200);
        h.named(f, "internal", "context", ctx);
        h.named(f, "internal", "code", code);
        h.named(ctx, "context", "logger", logger);
        handlers.push(f);
    }
    h.array(global, "handlers", &handlers);

    let service = h.node("object", "OrderService", 48);
    h.named(global, "property", "orderService", service);
    let session = h.node("object", "Session", 48);
    h.named(service, "weak", "session", session);
    let customers: Vec<usize> = (0..10)
        .map(|i| {
            let c = h.node("object", "Customer", 40);
            let name = h.node("string", &format!("customer-{i:02}"), 32);
            h.named(c, "property", "name", name);
            c
        })
        .collect();
    h.array(service, "customers", &customers);

    let cache = h.node("object", "OrderCache", 40);
    h.named(service, "property", "cache", cache);
    let map = h.node("object", "Map", 32);
    h.named(cache, "property", "orders", map);
    let table = h.node("array", "", 16 + 16 * orders as u32);
    h.named(map, "internal", "table", table);
    for i in 0..orders {
        let order = h.node("object", "Order", 64);
        let id = h.node("string", &format!("order-{:04}", i + 1), 32);
        let total = h.node("number", "heap number", 16);
        h.indexed(table, "element", 2 * i as u32, id);
        h.indexed(table, "element", 2 * i as u32 + 1, order);
        h.named(order, "property", "id", id);
        h.named(order, "property", "total", total);
        h.named(order, "property", "customer", customers[i % customers.len()]);
        let lines: Vec<usize> = (0..2).map(|_| h.node("object", "OrderLine", 40)).collect();
        h.array(order, "lines", &lines);
    }
    if audit_entries > 0 {
        let audit = h.node("object", "AuditLog", 40);
        h.named(service, "property", "audit", audit);
        let entries: Vec<usize> = (0..audit_entries)
            .map(|i| {
                let e = h.node("object", "AuditEntry", 48);
                let msg = h.node("concatenated string", &format!("placed order-{:04}", i + 1), 32);
                h.named(e, "property", "message", msg);
                e
            })
            .collect();
        h.array(audit, "entries", &entries);
    }
    h.write()
}

// ---------------------------------------------------------------- files

fn samples() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("samples/profiling/orders.speedscope.json", speedscope_sampled(false)),
        ("samples/profiling/orders-regression.speedscope.json", speedscope_sampled(true)),
        ("samples/profiling/orders-evented.speedscope.json", speedscope_evented()),
        ("samples/profiling/orders.cpuprofile", cpuprofile()),
        ("samples/profiling/orders.trace.json", trace()),
        ("samples/profiling/heap-1.heapsnapshot", heap_snapshot(50, 0)),
        ("samples/profiling/heap-2.heapsnapshot", heap_snapshot(100, 20)),
        ("samples/profiling/heap-3.heapsnapshot", heap_snapshot(150, 40)),
    ]
}

/// Small valid inputs for the fuzz corpora (and the property tests).
fn seeds() -> Vec<(&'static str, String)> {
    let frames = r#""shared":{"frames":[{"name":"main","file":"app.js","line":1},{"name":"work"},{"name":"(garbage collector)"}]}"#;
    let nodes = r#""nodes":[{"id":1,"callFrame":{"functionName":"(root)","scriptId":"0","url":"","lineNumber":-1,"columnNumber":-1},"children":[2,3]},
{"id":2,"callFrame":{"functionName":"main","scriptId":"1","url":"file:///srv/app.js","lineNumber":0,"columnNumber":0},"children":[4]},
{"id":3,"callFrame":{"functionName":"(garbage collector)","scriptId":"0","url":"","lineNumber":-1,"columnNumber":-1}},
{"id":4,"callFrame":{"functionName":"","scriptId":"1","url":"file:///srv/app.js","lineNumber":9,"columnNumber":2}}]"#;
    let speedscope = "https://www.speedscope.app/file-format-schema.json";
    let mut small_heap = Heap::default();
    let r = small_heap.node("synthetic", "", 0);
    let g = small_heap.node("synthetic", "(GC roots)", 0);
    let o = small_heap.node("object", "Order", 64);
    let s = small_heap.node("string", "order-1", 32);
    small_heap.indexed(r, "element", 1, g);
    small_heap.named(g, "property", "order", o);
    small_heap.named(o, "property", "id", s);
    small_heap.named(o, "weak", "self", o);
    vec![
        (
            "fuzz/corpus/fuzz_prof_speedscope/sampled.json",
            format!(
                r#"{{"$schema":"{speedscope}",{frames},"profiles":[{{"type":"sampled","name":"t","unit":"milliseconds","startValue":0,"endValue":4,"samples":[[0,1],[0],[2]],"weights":[1,2,1]}}]}}"#
            ),
        ),
        (
            "fuzz/corpus/fuzz_prof_speedscope/evented.json",
            format!(
                r#"{{"$schema":"{speedscope}",{frames},"profiles":[{{"type":"evented","name":"t","unit":"microseconds","startValue":0,"endValue":50,"events":[{{"type":"O","frame":0,"at":0}},{{"type":"O","frame":1,"at":10}},{{"type":"C","frame":1,"at":30}},{{"type":"C","frame":0,"at":50}}]}}]}}"#
            ),
        ),
        (
            "fuzz/corpus/fuzz_prof_speedscope/bytes.json",
            format!(
                r#"{{"$schema":"{speedscope}",{frames},"profiles":[{{"type":"sampled","name":"heap","unit":"bytes","startValue":0,"endValue":0,"samples":[[0,1]],"weights":[128]}}]}}"#
            ),
        ),
        (
            "fuzz/corpus/fuzz_prof_cpuprofile/small.cpuprofile",
            format!(r#"{{{nodes},"startTime":1000,"endTime":1100,"samples":[4,4,3,2,1],"timeDeltas":[10,20,30,10,5]}}"#),
        ),
        (
            "fuzz/corpus/fuzz_prof_cpuprofile/hitcounts.cpuprofile",
            r#"{"nodes":[{"id":1,"callFrame":{"functionName":"a","url":"x.js"},"hitCount":4,"children":[2]},{"id":2,"callFrame":{"functionName":"b"},"hitCount":1}]}"#.to_string(),
        ),
        (
            "fuzz/corpus/fuzz_prof_chrome_trace/chunks.json",
            r#"{"traceEvents":[{"ph":"M","name":"thread_name","pid":1,"tid":7,"args":{"name":"main"}},
{"ph":"P","name":"Profile","id":"0x1","pid":1,"tid":7,"ts":1000,"args":{"data":{"startTime":1000}}},
{"ph":"P","name":"ProfileChunk","id":"0x1","pid":1,"tid":9,"ts":1100,"args":{"data":{"cpuProfile":{"nodes":[{"id":1,"callFrame":{"functionName":"(root)","scriptId":0,"url":""}},{"id":2,"parent":1,"callFrame":{"functionName":"main","scriptId":1,"url":"https://x/app.js","lineNumber":3}}],"samples":[2,2,1]},"timeDeltas":[10,10,-2]}}},
{"ph":"i","name":"MinorGC","pid":1,"tid":7,"ts":1020,"s":"t"}]}"#
                .to_string(),
        ),
        (
            "fuzz/corpus/fuzz_prof_chrome_trace/durations.json",
            r#"[{"ph":"X","name":"outer","pid":1,"tid":2,"ts":100,"dur":100},{"ph":"X","name":"inner","pid":1,"tid":2,"ts":120,"dur":30},
{"ph":"B","name":"span","pid":1,"tid":2,"ts":160},{"ph":"E","pid":1,"tid":2,"ts":170},{"ph":"X","name":"MajorGC","pid":1,"tid":3,"ts":150,"dur":10},"#
                .to_string(),
        ),
        ("fuzz/corpus/fuzz_prof_heapsnapshot/small.heapsnapshot", String::from_utf8(small_heap.write()).unwrap()),
        (
            "fuzz/corpus/fuzz_prof_heapsnapshot/reordered.heapsnapshot",
            r#"{"snapshot":{"meta":{"node_fields":["edge_count","self_size","name","type"],"node_types":["number","number","string",["hidden","object","synthetic"]],"edge_fields":["to_node","type","name_or_index"],"edge_types":["node",["element","property","weak"],"string_or_number"]},"node_count":3,"edge_count":3},
"nodes":[1,0,0,2, 2,40,1,1, 0,24,2,1],"edges":[4,0,1, 8,1,3, 0,2,3],"strings":["","Cache","Order","entry"]}"#
                .to_string(),
        ),
    ]
}

fn import(importer: &dyn CaptureImporter, path: &str) -> Capture {
    let bytes = std::fs::read(root().join(path)).unwrap_or_else(|e| panic!("{path}: {e} (regenerate the samples)"));
    importer
        .import(path, &bytes, &Limits::default())
        .unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn profile(importer: &dyn CaptureImporter, path: &str) -> Profile {
    match import(importer, path) {
        Capture::Profile(p) => p,
        Capture::Heap(_) => panic!("{path} is not a profile"),
    }
}

fn heap(path: &str) -> HeapSnapshot {
    match import(&HeapSnapshotImporter, path) {
        Capture::Heap(h) => h,
        Capture::Profile(_) => panic!("{path} is not a heap snapshot"),
    }
}

/// Share of a metric's total in samples whose stack contains `function`.
fn inclusive_share(p: &Profile, function: &str, metric: MetricId) -> f64 {
    let mi = p.metric_index(metric).unwrap();
    let stride = p.metrics.len();
    let (mut hit, mut total) = (0u64, 0u64);
    for i in 0..p.samples.len() {
        let v = p.samples.values[i * stride + mi];
        total += v;
        if p.stack_frames(p.samples.stack[i])
            .iter()
            .any(|&f| p.frames[f as usize].function == function)
        {
            hit += v;
        }
    }
    hit as f64 / total as f64
}

fn self_total(p: &Profile, function: &str, metric: MetricId) -> u64 {
    let mi = p.metric_index(metric).unwrap();
    (0..p.samples.len())
        .filter(|&i| p.frames[p.stacks[p.samples.stack[i] as usize].frame as usize].function == function)
        .map(|i| p.samples.values[i * p.metrics.len() + mi])
        .fold(0, |acc, v| if metric.is_max() { acc.max(v) } else { acc + v })
}

#[test]
#[ignore = "writes samples/profiling/*.json, *.cpuprofile, *.heapsnapshot and the fuzz seeds; run with DEBRIEF_REGENERATE_SAMPLES=1"]
fn regenerate_samples() {
    if std::env::var_os("DEBRIEF_REGENERATE_SAMPLES").is_none() {
        eprintln!("set DEBRIEF_REGENERATE_SAMPLES=1 to write the samples");
        return;
    }
    let files = samples().into_iter().chain(seeds().into_iter().map(|(p, s)| (p, s.into_bytes())));
    for (path, bytes) in files {
        let path = root().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
    }
}

#[test]
fn committed_samples_are_up_to_date() {
    for (path, bytes) in samples() {
        assert!(bytes.len() < 200 * 1024, "{path} is {} bytes", bytes.len());
        let committed = std::fs::read(root().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"));
        // Compared as JSON: key order depends on serde_json's `preserve_order`
        // feature, which the `plugin` feature (via debrief-core) turns on.
        let parse = |b: &[u8]| serde_json::from_slice::<serde_json::Value>(b).unwrap_or_else(|e| panic!("{path}: {e}"));
        assert!(
            parse(&committed) == parse(&bytes),
            "{path} differs from the generator; regenerate the samples"
        );
    }
    for (path, text) in seeds() {
        let committed = std::fs::read_to_string(root().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"));
        // JSON seeds compare as values (key order depends on serde_json features); others as text.
        let parse = |t: &str| serde_json::from_str::<serde_json::Value>(t).ok();
        match (parse(&committed), parse(&text)) {
            (Some(a), Some(b)) => assert!(a == b, "{path} differs from the generator"),
            _ => assert_eq!(committed, text, "{path}"),
        }
    }
}

#[test]
fn seeds_import() {
    for (path, text) in seeds() {
        let importer: &dyn CaptureImporter = if path.contains("speedscope") {
            &SpeedscopeImporter
        } else if path.contains("cpuprofile") {
            &CpuProfileImporter
        } else if path.contains("chrome_trace") {
            &TraceEventImporter
        } else {
            &HeapSnapshotImporter
        };
        importer
            .import(path, text.as_bytes(), &Limits::default())
            .unwrap_or_else(|e| panic!("{path}: {e}"));
        assert!(importer.sniff(path, text.as_bytes()) >= 80, "{path}");
    }
}

#[test]
fn speedscope_sample_shape() {
    let p = profile(&SpeedscopeImporter, "samples/profiling/orders.speedscope.json");
    assert_eq!(p.name, "OrderService");
    assert_eq!(p.kind, ProfileKind::Cpu);
    assert!(p.has_time);
    assert_eq!(p.samples.len(), 2000);
    let names: Vec<&str> = p.threads.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["Request thread", "Outbox worker"]);
    let share = |f| inclusive_share(&p, f, MetricId::TimeNs);
    assert!((0.07..0.13).contains(&share(SERIALIZE)), "serialisation {}", share(SERIALIZE));
    assert!((0.06..0.10).contains(&share(GC)), "gc {}", share(GC));
    assert!((0.04..0.08).contains(&share(LOCK)), "lock {}", share(LOCK));
    assert_eq!(share(AUDIT), 0.0);
    assert!(p.duration_ns > 900_000_000, "about one second per thread: {}", p.duration_ns);
    let main = p.frames.iter().find(|f| f.function == "Shop.Orders.Program.Main").unwrap();
    assert_eq!((main.module.as_deref(), main.line), (Some("Program.cs"), Some(12)));

    let r = profile(&SpeedscopeImporter, "samples/profiling/orders-regression.speedscope.json");
    let share = |f| inclusive_share(&r, f, MetricId::TimeNs);
    assert!((0.26..0.34).contains(&share(SERIALIZE)), "serialisation {}", share(SERIALIZE));
    assert!((0.03..0.07).contains(&share(AUDIT)), "audit {}", share(AUDIT));
}

#[test]
fn evented_speedscope_sample_shape() {
    let p = profile(&SpeedscopeImporter, "samples/profiling/orders-evented.speedscope.json");
    assert_eq!(p.kind, ProfileKind::Wall);
    assert!(p.metrics.iter().all(|m| m.measurement == Measurement::Exact));
    assert!(p.notes.is_empty(), "{:?}", p.notes);
    assert_eq!(p.threads.len(), 2);
    assert_eq!(self_total(&p, "Shop.Orders.OrderService.PlaceOrder", MetricId::Calls), 40);
    assert_eq!(self_total(&p, "System.Net.Http.HttpClient.SendAsync", MetricId::Calls), 20);
    let max_sql = self_total(&p, "System.Data.SqlClient.SqlCommand.ExecuteNonQuery", MetricId::MaxCallNs);
    assert!((400_000..=900_000).contains(&max_sql), "{max_sql}");
    // Self times add up to the threads' busy time: every nanosecond is attributed once.
    let total: u64 = (0..p.samples.len()).map(|i| p.samples.values[i * p.metrics.len()]).sum();
    assert!(total > 0 && total <= 2 * p.duration_ns);
}

#[test]
fn cpuprofile_sample_shape() {
    let p = profile(&CpuProfileImporter, "samples/profiling/orders.cpuprofile");
    assert_eq!(p.samples.len(), 2000);
    assert_eq!(p.started_ms, None);
    assert!(p.has_time);
    let share = |f| inclusive_share(&p, f, MetricId::TimeNs);
    assert!(
        (0.12..0.18).contains(&share("(garbage collector)")),
        "gc {}",
        share("(garbage collector)")
    );
    assert!((0.16..0.24).contains(&share("(idle)")));
    assert!(share("placeOrder") > 0.3);
    assert!(share("JSON.stringify") > 0.08);
    let gc = p.frames.iter().find(|f| f.function == "(garbage collector)").unwrap();
    assert!(!gc.is_framework);
    assert!(p.frames.iter().find(|f| f.function == "(idle)").unwrap().is_framework);
    assert!(
        p.frames.iter().find(|f| f.function == "query").unwrap().is_framework,
        "node_modules is library code"
    );
    let save = p.frames.iter().find(|f| f.function == "saveOrder").unwrap();
    assert_eq!((save.module.as_deref(), save.line), (Some("orders.js"), Some(88)));
    assert!(p.duration_ns > 900_000_000);
}

#[test]
fn trace_sample_shape() {
    let p = profile(&TraceEventImporter, "samples/profiling/orders.trace.json");
    assert_eq!(p.kind, ProfileKind::Cpu);
    assert_eq!(p.samples.len(), 600);
    assert_eq!(p.threads.len(), 1);
    assert_eq!(p.threads[0].name, "CrRendererMain");
    assert!(p.notes.iter().any(|n| n.contains("duration events were not used")), "{:?}", p.notes);
    let gcs = p.events.iter().filter(|e| e.kind == EventKind::Gc).count();
    assert_eq!(gcs, 4, "3 MinorGC + 1 MajorGC");
    assert!((0.1..0.2).contains(&inclusive_share(&p, "(garbage collector)", MetricId::TimeNs)));
}

#[test]
fn heap_sample_shapes() {
    let count = |h: &HeapSnapshot, t: &str| h.types.iter().find(|x| x.name == t).map_or(0, |x| x.count);
    let hs: Vec<HeapSnapshot> = (1..=3).map(|i| heap(&format!("samples/profiling/heap-{i}.heapsnapshot"))).collect();
    assert_eq!(hs.iter().map(|h| count(h, "Order")).collect::<Vec<_>>(), [50, 100, 150]);
    assert_eq!(hs.iter().map(|h| count(h, "AuditEntry")).collect::<Vec<_>>(), [0, 20, 40]);
    assert!(hs[0].types.iter().all(|t| t.name != "AuditLog"), "snapshot 1 lacks the audit types");
    for h in &hs {
        assert_eq!(count(h, "OrderCache"), 1);
        assert_eq!(count(h, "Customer"), 10);
        assert_eq!(count(h, "(closure)"), 3);
        assert_eq!(count(h, "(regexp)"), 1);
        assert!(count(h, "(string)") > 10);
        assert!(count(h, "(number)") >= 50);
        let g = h.graph.as_ref().unwrap();
        assert_eq!(g.roots, vec![0]);
        let labels: Vec<&str> = (0..g.len() as u32)
            .flat_map(|o| g.edges(o).map(|(_, l)| g.label(l).unwrap()))
            .collect();
        assert!(
            !labels.contains(&"session") && !labels.contains(&"target"),
            "weak edges are skipped"
        );
        assert!(!labels.contains(&"global"), "shortcut edges are skipped");
        assert!(labels.contains(&"orders") && labels.contains(&"[0]"));
    }
    assert!(hs[2].total_bytes() > hs[0].total_bytes());
}

#[test]
fn each_sample_is_claimed_by_its_own_importer() {
    let mine: Vec<(&str, &dyn CaptureImporter)> = vec![
        ("speedscope", &SpeedscopeImporter),
        ("cpuprofile", &CpuProfileImporter),
        ("chrome-trace", &TraceEventImporter),
        ("heapsnapshot", &HeapSnapshotImporter),
    ];
    let all = debrief_plugin_profiling::builtin_importers();
    for (path, bytes) in samples() {
        let head = &bytes[..bytes.len().min(64 * 1024)];
        let expected = if path.contains("speedscope") {
            "speedscope"
        } else if path.ends_with(".cpuprofile") {
            "cpuprofile"
        } else if path.contains("trace") {
            "chrome-trace"
        } else {
            "heapsnapshot"
        };
        let best = all.iter().max_by_key(|i| i.sniff(path, head)).unwrap();
        assert_eq!(best.id(), expected, "{path}");
        for (id, imp) in &mine {
            let s = imp.sniff(path, head);
            if *id == expected {
                assert!(s >= 85, "{path}: {id} {s}");
            } else {
                assert!(s <= 10, "{path} is claimed by {id} ({s})");
            }
        }
    }
    // No other sample in the repository is claimed by these importers.
    let ours: Vec<PathBuf> = samples().iter().map(|(p, _)| root().join(p)).collect();
    let mut stack = vec![root().join("samples")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if ours.contains(&path) {
                continue;
            }
            let bytes = std::fs::read(&path).unwrap();
            let name = path.to_string_lossy().to_string();
            for (id, imp) in &mine {
                assert_eq!(
                    imp.sniff(&name, &bytes[..bytes.len().min(64 * 1024)]),
                    0,
                    "{name} is claimed by {id}"
                );
            }
        }
    }
}
