//! Deterministic generators for the pprof, perf script and folded samples in
//! `samples/profiling/`, and tests of how those samples import.
//!
//! Regenerate the samples (they are committed; never edit them by hand):
//!
//! ```sh
//! DEBRIEF_REGENERATE_SAMPLES=1 cargo test -p debrief-plugin-profiling --no-default-features \
//!     --test gen_pprof -- --ignored regenerate_samples
//! ```
//!
//! - `orders.pb.gz`: a Go-style CPU profile (samples/count + cpu/nanoseconds)
//!   of a fictional order service: ~40 distinct stacks, GC ~10%, mutex
//!   contention ~8%, an inlined location, two mappings, `thread` labels.
//! - `orders-alloc.pb.gz`: a Go heap profile of the same service
//!   (alloc_objects, alloc_space, inuse_objects, inuse_space).
//! - `orders.perf.txt`: `perf script` output of a native C++ build of the
//!   service: 120 cpu-clock samples on two threads with kernel, libc,
//!   libstdc++ and `[unknown]` frames (a fixed-seed generator below).
//! - `orders.folded` / `orders-regression.folded`: collapsed stacks of a .NET
//!   build, written verbatim from the constants below; in the regression the
//!   JSON serialisation share grows from 10% to ~28% and
//!   `Shop.Orders.AuditLog.Write` appears.

use debrief_plugin_profiling::folded::FoldedImporter;
use debrief_plugin_profiling::perf::PerfScriptImporter;
use debrief_plugin_profiling::pprof::proto::{Function, Label, Line, Location, Mapping, Profile as Pb, Sample, ValueType};
use debrief_plugin_profiling::pprof::PprofImporter;
use debrief_profile::{Capture, CaptureImporter, FrameKind, Limits, MetricId, Profile, ProfileKind};
use prost::Message;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples/profiling")
}

// ---------------------------------------------------------------- pprof

/// pprof encoder with interned strings, functions and locations.
struct Enc {
    pb: Pb,
    functions: HashMap<String, u64>,
    locations: HashMap<Vec<u64>, u64>,
}

/// Functions inlined into their caller (one location holds both).
const INLINED: &[&str] = &["orders.(*Service).validate", "encoding/json.isValidTag"];

impl Enc {
    fn new(types: &[(&str, &str)], period_type: (&str, &str), period: i64) -> Enc {
        let mut e = Enc {
            pb: Pb {
                string_table: vec![String::new()],
                ..Pb::default()
            },
            functions: HashMap::new(),
            locations: HashMap::new(),
        };
        for (t, u) in types {
            let vt = ValueType {
                r#type: e.s(t),
                unit: e.s(u),
            };
            e.pb.sample_type.push(vt);
        }
        e.pb.period_type = Some(ValueType {
            r#type: e.s(period_type.0),
            unit: e.s(period_type.1),
        });
        e.pb.period = period;
        // 2026-10-14T09:00:00Z
        e.pb.time_nanos = 1_791_968_400_000_000_000;
        e.pb.duration_nanos = 30_000_000_000;
        let (exe, vdso, build_id) = (e.s("/usr/local/bin/orders-svc"), e.s("[vdso]"), e.s("6f1c0a3e9b2d4f5a"));
        e.pb.mapping.push(Mapping {
            id: 1,
            memory_start: 0x400000,
            memory_limit: 0xc00000,
            filename: exe,
            build_id,
            has_functions: true,
            has_filenames: true,
            has_line_numbers: true,
            has_inline_frames: true,
            ..Mapping::default()
        });
        e.pb.mapping.push(Mapping {
            id: 2,
            memory_start: 0x7ffd_0000_0000,
            memory_limit: 0x7ffd_0000_2000,
            filename: vdso,
            has_functions: true,
            ..Mapping::default()
        });
        e
    }

    fn s(&mut self, v: &str) -> i64 {
        if let Some(i) = self.pb.string_table.iter().position(|x| x == v) {
            return i as i64;
        }
        self.pb.string_table.push(v.to_string());
        (self.pb.string_table.len() - 1) as i64
    }

    fn function(&mut self, name: &str) -> u64 {
        if let Some(&id) = self.functions.get(name) {
            return id;
        }
        let id = self.functions.len() as u64 + 1;
        let file = go_file(name);
        let f = Function {
            id,
            name: self.s(name),
            system_name: self.s(name),
            filename: self.s(&file),
            start_line: 10 + (hash(name) % 400) as i64,
        };
        self.pb.function.push(f);
        self.functions.insert(name.to_string(), id);
        id
    }

    /// A location for functions given innermost first (more than one: inlined).
    fn location(&mut self, names: &[&str]) -> u64 {
        let fids: Vec<u64> = names.iter().map(|n| self.function(n)).collect();
        if let Some(&id) = self.locations.get(&fids) {
            return id;
        }
        let id = self.locations.len() as u64 + 1;
        let vdso = names[0].starts_with("__vdso");
        let line = fids
            .iter()
            .zip(names)
            .map(|(&f, n)| Line {
                function_id: f,
                line: 12 + (hash(n) % 300) as i64,
                column: 0,
            })
            .collect();
        self.pb.location.push(Location {
            id,
            mapping_id: if vdso { 2 } else { 1 },
            address: if vdso { 0x7ffd_0000_0a00 + id * 16 } else { 0x401000 + id * 0x40 },
            line,
            is_folded: false,
        });
        self.locations.insert(fids, id);
        id
    }

    /// Location ids of a root-first `a;b;c` stack, leaf first, folding inlined functions into their caller.
    fn stack(&mut self, stack: &str) -> Vec<u64> {
        let names: Vec<&str> = stack.split(';').collect();
        let mut groups: Vec<Vec<&str>> = Vec::new(); // root first, each innermost first
        for n in names {
            match groups.last_mut() {
                Some(g) if INLINED.contains(&n) => g.insert(0, n),
                _ => groups.push(vec![n]),
            }
        }
        groups.iter().rev().map(|g| self.location(g)).collect()
    }

    fn sample(&mut self, stack: &str, values: Vec<i64>, thread: &str) {
        let location_id = self.stack(stack);
        let label = vec![Label {
            key: self.s("thread"),
            str: self.s(thread),
            ..Label::default()
        }];
        self.pb.sample.push(Sample {
            location_id,
            value: values,
            label,
        });
    }
}

fn gz(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::GzBuilder::new().mtime(0).write(Vec::new(), flate2::Compression::best());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

fn gunzip(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(data).read_to_end(&mut out).unwrap();
    out
}

/// FNV-1a, for stable pseudo line numbers and addresses.
fn hash(s: &str) -> u64 {
    s.bytes()
        .fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

/// A plausible Go source file for a function name.
fn go_file(name: &str) -> String {
    if name.starts_with("__vdso") {
        return String::new();
    }
    let slash = name.rfind('/').map_or(0, |i| i + 1);
    let pkg_end = name[slash..].find('.').map_or(name.len(), |i| slash + i);
    let pkg = &name[..pkg_end];
    let base = &pkg[slash..];
    match pkg {
        "main" => "/src/orders-svc/cmd/orders-svc/main.go".to_string(),
        "orders" => format!(
            "/src/orders-svc/internal/orders/{}.go",
            if name.contains("Repo") { "repo" } else { "service" }
        ),
        p if p.starts_with("github.com/") => format!("/root/go/pkg/mod/{p}@v1.10.9/conn.go"),
        p => format!("/usr/local/go/src/{p}/{base}.go"),
    }
}

const HTTP: &str =
    "runtime.goexit;net/http.(*conn).serve;net/http.serverHandler.ServeHTTP;net/http.(*ServeMux).ServeHTTP;main.(*api).handleOrders";

/// CPU stacks below [`HTTP`] (`+`) or complete, with weights in 10 ms samples.
const CPU_STACKS: &[(&str, i64)] = &[
    ("+;orders.(*Service).PlaceOrder", 30),
    ("+;orders.(*Service).PlaceOrder;orders.(*Service).validate", 46),
    ("+;orders.(*Service).PlaceOrder;orders.(*Service).validate;strings.TrimSpace", 18),
    ("+;orders.(*Service).PlaceOrder;orders.(*Repo).Save;database/sql.(*DB).ExecContext;database/sql.(*DB).execDC;github.com/lib/pq.(*conn).Exec;net.(*conn).Write;syscall.Syscall", 118),
    ("+;orders.(*Service).PlaceOrder;orders.(*Repo).Save;database/sql.(*DB).ExecContext;database/sql.(*DB).execDC;github.com/lib/pq.(*conn).Exec;net.(*conn).Read;syscall.Syscall", 92),
    ("+;orders.(*Service).PlaceOrder;orders.(*Repo).Save;database/sql.(*DB).ExecContext;database/sql.(*DB).conn;database/sql.(*DB).connectionCleaner", 12),
    ("+;orders.(*Service).PlaceOrder;orders.(*Repo).Save;database/sql.(*DB).QueryContext;database/sql.(*DB).queryDC;github.com/lib/pq.(*conn).query;net.(*conn).Read;syscall.Syscall", 106),
    ("+;orders.(*Service).PlaceOrder;orders.(*Repo).Save;database/sql.(*DB).QueryContext;database/sql.(*Rows).Next;github.com/lib/pq.(*rows).Next", 58),
    ("+;orders.(*Service).PlaceOrder;orders.(*Repo).Save;database/sql.(*DB).QueryContext;database/sql.(*Rows).Next;github.com/lib/pq.(*rows).Next;runtime.mallocgc", 24),
    ("+;orders.(*Service).PlaceOrder;orders.(*Repo).Save;database/sql.(*Tx).Commit;github.com/lib/pq.(*conn).Commit;net.(*conn).Write;syscall.Syscall", 40),
    ("+;orders.(*Service).PlaceOrder;orders.(*Inventory).Reserve", 44),
    ("+;orders.(*Service).PlaceOrder;orders.(*Inventory).Reserve;sync.(*Mutex).Lock", 22),
    ("+;orders.(*Service).PlaceOrder;orders.(*Inventory).Reserve;sync.(*Mutex).Lock;sync.(*Mutex).lockSlow", 40),
    ("+;orders.(*Service).PlaceOrder;orders.(*Inventory).Reserve;sync.(*Mutex).Lock;sync.(*Mutex).lockSlow;runtime.semacquire1", 34),
    ("+;orders.(*Service).PlaceOrder;orders.(*Bus).Publish;sync.(*Mutex).Lock;sync.(*Mutex).lockSlow", 38),
    ("+;orders.(*Service).PlaceOrder;orders.(*Bus).Publish;runtime.chansend1", 26),
    ("+;orders.(*Service).PlaceOrder;orders.(*Pricing).Quote", 64),
    ("+;orders.(*Service).PlaceOrder;orders.(*Pricing).Quote;orders.applyDiscounts", 72),
    ("+;orders.(*Service).PlaceOrder;orders.(*Pricing).Quote;orders.applyDiscounts;math.Round", 16),
    ("+;orders.(*Service).PlaceOrder;orders.(*Pricing).Quote;time.Now;runtime.nanotime1;__vdso_clock_gettime", 14),
    ("+;encoding/json.Marshal;encoding/json.(*encodeState).marshal;encoding/json.(*encodeState).reflectValue;encoding/json.structEncoder.encode", 52),
    ("+;encoding/json.Marshal;encoding/json.(*encodeState).marshal;encoding/json.(*encodeState).reflectValue;encoding/json.structEncoder.encode;encoding/json.stringEncoder", 46),
    ("+;encoding/json.Marshal;encoding/json.(*encodeState).marshal;encoding/json.(*encodeState).reflectValue;encoding/json.typeFields;encoding/json.isValidTag", 10),
    ("+;encoding/json.Marshal;runtime.mallocgc", 34),
    ("+;encoding/json.Marshal;runtime.mallocgc;runtime.memclrNoHeapPointers", 12),
    ("+;encoding/json.Marshal;bytes.(*Buffer).grow;runtime.growslice", 28),
    ("+;encoding/json.(*Decoder).Decode;encoding/json.(*decodeState).object", 44),
    ("+;encoding/json.(*Decoder).Decode;encoding/json.(*Decoder).readValue", 18),
    ("runtime.goexit;net/http.(*conn).serve;net/http.(*conn).readRequest;net/http.readRequest;net/textproto.(*Reader).ReadMIMEHeader", 48),
    ("runtime.goexit;net/http.(*conn).serve;net/http.(*conn).readRequest;bufio.(*Reader).fill;net.(*conn).Read;syscall.Syscall", 36),
    ("runtime.goexit;net/http.(*conn).serve;net/http.(*response).finishRequest;bufio.(*Writer).Flush;net.(*conn).Write;syscall.Syscall", 42),
    ("runtime.goexit;main.main;main.run;net/http.(*Server).ListenAndServe;net/http.(*Server).Serve;net.(*TCPListener).Accept;syscall.Syscall", 14),
    ("runtime.goexit;runtime.gcBgMarkWorker;runtime.systemstack;runtime.gcDrain;runtime.scanobject", 95),
    ("runtime.goexit;runtime.gcBgMarkWorker;runtime.systemstack;runtime.gcDrain;runtime.markroot;runtime.scanstack", 38),
    ("runtime.goexit;runtime.gcBgMarkWorker;runtime.systemstack;runtime.gcDrain;runtime.greyobject", 30),
    ("runtime.goexit;runtime.bgsweep;runtime.sweepone", 22),
    ("runtime.mcall;runtime.park_m;runtime.schedule;runtime.findRunnable;runtime.netpoll;runtime.epollwait", 58),
    ("runtime.mcall;runtime.park_m;runtime.schedule;runtime.findRunnable;runtime.stealWork", 20),
    ("runtime.mstart;runtime.sysmon;runtime.usleep", 12),
    ("runtime.goexit;database/sql.(*DB).connectionOpener;runtime.selectgo", 8),
];

fn full(stack: &str) -> String {
    match stack.strip_prefix('+') {
        Some(rest) => format!("{HTTP}{rest}"),
        None => stack.to_string(),
    }
}

fn thread_for(stack: &str) -> [&'static str; 2] {
    if stack.contains("gcBgMarkWorker") || stack.contains("bgsweep") {
        ["gc-worker-1", "gc-worker-2"]
    } else if stack.starts_with("runtime.m") {
        ["runtime", "runtime"]
    } else {
        ["http-worker-1", "http-worker-2"]
    }
}

/// The CPU profile, uncompressed.
fn cpu_profile() -> Vec<u8> {
    let mut e = Enc::new(&[("samples", "count"), ("cpu", "nanoseconds")], ("cpu", "nanoseconds"), 10_000_000);
    for (stack, weight) in CPU_STACKS {
        let stack = full(stack);
        let [t1, t2] = thread_for(&stack);
        // Split each stack over two threads, as profiles with thread labels do.
        let first = weight * 3 / 5;
        for (n, t) in [(first, t1), (weight - first, t2)] {
            if n > 0 {
                e.sample(&stack, vec![n, n * 10_000_000], t);
            }
        }
    }
    e.pb.encode_to_vec()
}

/// Allocation stacks with (objects, bytes per object, share still in use in %).
const ALLOC_STACKS: &[(&str, i64, i64, i64)] = &[
    ("+;encoding/json.Marshal;runtime.mallocgc", 52_000, 96, 2),
    ("+;encoding/json.Marshal;bytes.(*Buffer).grow;runtime.growslice", 9_800, 2_048, 1),
    (
        "+;encoding/json.(*Decoder).Decode;encoding/json.(*decodeState).object;runtime.mallocgc",
        31_000,
        64,
        3,
    ),
    (
        "+;orders.(*Service).PlaceOrder;orders.(*Repo).Save;database/sql.(*DB).QueryContext;database/sql.(*Rows).Next;runtime.mallocgc",
        24_500,
        48,
        0,
    ),
    (
        "+;orders.(*Service).PlaceOrder;orders.(*Repo).Save;database/sql.(*DB).ExecContext;runtime.mallocgc",
        12_000,
        160,
        0,
    ),
    (
        "+;orders.(*Service).PlaceOrder;orders.(*Inventory).Reserve;runtime.makemap",
        3_100,
        512,
        40,
    ),
    (
        "+;orders.(*Service).PlaceOrder;orders.(*Pricing).Quote;orders.applyDiscounts;runtime.growslice",
        18_200,
        128,
        0,
    ),
    (
        "+;orders.(*Service).PlaceOrder;orders.(*Bus).Publish;runtime.makechan",
        400,
        4_096,
        100,
    ),
    (
        "runtime.goexit;net/http.(*conn).serve;net/http.(*conn).readRequest;bufio.NewReaderSize;runtime.mallocgc",
        6_000,
        4_096,
        5,
    ),
    (
        "runtime.goexit;net/http.(*conn).serve;net/http.(*conn).readRequest;net/textproto.(*Reader).ReadMIMEHeader;runtime.makemap_small",
        22_000,
        48,
        1,
    ),
    (
        "runtime.goexit;main.main;main.run;orders.NewService;orders.(*Cache).warm;runtime.mallocgc",
        120_000,
        32,
        85,
    ),
];

/// The heap profile, uncompressed.
fn alloc_profile() -> Vec<u8> {
    let types = [
        ("alloc_objects", "count"),
        ("alloc_space", "bytes"),
        ("inuse_objects", "count"),
        ("inuse_space", "bytes"),
    ];
    let mut e = Enc::new(&types, ("space", "bytes"), 524_288);
    e.pb.duration_nanos = 0;
    e.pb.default_sample_type = e.s("inuse_space");
    for (stack, objects, size, live) in ALLOC_STACKS {
        let stack = full(stack);
        let inuse = objects * live / 100;
        e.sample(&stack, vec![*objects, objects * size, inuse, inuse * size], "all");
    }
    e.pb.encode_to_vec()
}

// ---------------------------------------------------------------- perf script

const EXE: &str = "/opt/orders/bin/orders-svc";
const LIBC: &str = "/usr/lib/x86_64-linux-gnu/libc.so.6";
const STDCXX: &str = "/usr/lib/x86_64-linux-gnu/libstdc++.so.6.0.30";
const PQ: &str = "/usr/lib/x86_64-linux-gnu/libpq.so.5.14";
const K: &str = "[kernel.kallsyms]";
const UNKNOWN: &str = "[unknown]";

/// A symbol and its binary.
type SymDso = (&'static str, &'static str);

const MAIN_ROOT: &[(&str, &str)] = &[
    ("_start", EXE),
    ("__libc_start_main", LIBC),
    ("main", EXE),
    ("orders::Server::run()", EXE),
    ("orders::Server::handle(orders::Request const&)", EXE),
];
const IO_ROOT: &[(&str, &str)] = &[
    ("clone3", LIBC),
    ("start_thread", LIBC),
    ("std::execute_native_thread_routine", STDCXX),
    ("orders::IoWorker::loop()", EXE),
];
const PLACE: (&str, &str) = ("orders::OrderService::placeOrder(orders::Order const&)", EXE);
const SYSCALL: &[(&str, &str)] = &[("entry_SYSCALL_64_after_hwframe", K), ("do_syscall_64", K)];

/// (thread 0 = main / 1 = io, weight, frames below the thread's root, root first).
fn perf_stacks() -> Vec<(usize, u32, Vec<SymDso>)> {
    let place = |rest: &[(&'static str, &'static str)]| -> Vec<(&'static str, &'static str)> {
        let mut v = vec![PLACE];
        v.extend_from_slice(rest);
        v
    };
    let with_sys = |pre: &[(&'static str, &'static str)], post: &[(&'static str, &'static str)]| {
        let mut v = pre.to_vec();
        v.extend_from_slice(SYSCALL);
        v.extend_from_slice(post);
        v
    };
    vec![
        (0, 30, place(&[])),
        (0, 50, place(&[("orders::Pricing::quote(orders::Order const&)", EXE)])),
        (0, 45, place(&[
            ("orders::Pricing::quote(orders::Order const&)", EXE),
            ("orders::Pricing::applyDiscounts(std::vector<orders::Line, std::allocator<orders::Line> >&)", EXE),
        ])),
        (0, 40, place(&[
            ("orders::Json::serialize(orders::Order const&)", EXE),
            ("std::__cxx11::basic_string<char, std::char_traits<char>, std::allocator<char> >::_M_append(char const*, unsigned long)", STDCXX),
            ("__memmove_avx_unaligned_erms", LIBC),
        ])),
        (0, 30, place(&[
            ("orders::Json::serialize(orders::Order const&)", EXE),
            ("operator new(unsigned long)", STDCXX),
            ("malloc", LIBC),
            ("_int_malloc", LIBC),
        ])),
        (0, 60, place(&with_sys(
            &[
                ("orders::OrderRepo::save(orders::Order const&)", EXE),
                ("PQexec", PQ),
                ("pqSocketCheck", PQ),
                ("__poll", LIBC),
            ],
            &[("__x64_sys_poll", K), ("do_sys_poll", K)],
        ))),
        (0, 25, place(&[("orders::OrderRepo::save(orders::Order const&)", EXE), ("PQexec", PQ), ("pqParseInput3", PQ)])),
        (0, 35, place(&with_sys(
            &[
                ("orders::Inventory::reserve(orders::Order const&)", EXE),
                ("pthread_mutex_lock", LIBC),
                ("__lll_lock_wait", LIBC),
            ],
            &[("__x64_sys_futex", K), ("futex_wait", K), ("schedule", K)],
        ))),
        (0, 15, vec![(UNKNOWN, UNKNOWN), ("orders::Metrics::record(std::basic_string_view<char, std::char_traits<char> >, double)", EXE)]),
        (0, 10, vec![(UNKNOWN, UNKNOWN)]),
        (1, 50, with_sys(
            &[("orders::IoWorker::flush()", EXE), ("__libc_write", LIBC)],
            &[("ksys_write", K), ("vfs_write", K), ("sock_write_iter", K), ("tcp_sendmsg", K)],
        )),
        (1, 45, with_sys(&[("epoll_wait", LIBC)], &[("__x64_sys_epoll_wait", K), ("ep_poll", K), ("schedule_hrtimeout_range", K)])),
        (1, 40, vec![("orders::Codec::decode(std::span<unsigned char const, 18446744073709551615ul>)", EXE)]),
        (1, 25, vec![("orders::Codec::decode(std::span<unsigned char const, 18446744073709551615ul>)", EXE), ("malloc", LIBC)]),
    ]
}

/// A small fixed-seed generator (xorshift64*).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
}

/// Samples in `orders.perf.txt` (each is ~800 bytes of text; kept small so the sample stays ~100 KB).
const PERF_SAMPLES: usize = 120;

/// `perf script` output: [`PERF_SAMPLES`] samples, 1 ms cpu-clock period.
fn perf_script() -> String {
    let stacks = perf_stacks();
    let total: u32 = stacks.iter().map(|s| s.1).sum();
    let mut rng = Rng(0x0dd5_eed5);
    let mut out = String::from(
        "# ========\n# captured on    : Wed Oct 14 09:00:00 2026\n# hostname : orders-01\n# cmdline : /usr/bin/perf record -F 1000 -g -p 4242\n# event : name = cpu-clock, , id = { 11, 12 }, type = 1, size = 136, { sample_period, sample_freq } = 1000\n# ========\n#\n",
    );
    let mut time_us: u64 = 81_234_100_000;
    for _ in 0..PERF_SAMPLES {
        time_us += 1_000 + rng.next() % 500;
        let mut pick = (rng.next() % u64::from(total)) as u32;
        let (thread, _, frames) = stacks
            .iter()
            .find(|(_, w, _)| {
                if pick < *w {
                    true
                } else {
                    pick -= w;
                    false
                }
            })
            .unwrap();
        let (comm, tid, root) = if *thread == 0 {
            ("orders-svc", 4242, MAIN_ROOT)
        } else {
            ("orders-io", 4251, IO_ROOT)
        };
        let cpu = rng.next() % 4;
        out.push_str(&format!(
            "{comm:>16} 4242/{tid} [{cpu:03}] {}.{:06}:    1000000 cpu-clock:\n",
            time_us / 1_000_000,
            time_us % 1_000_000
        ));
        let all: Vec<&(&str, &str)> = root.iter().chain(frames.iter()).collect();
        for (sym, dso) in all.into_iter().rev() {
            let h = hash(sym);
            let addr = if *dso == K {
                0xffff_ffff_8100_0000 | (h & 0x00ff_fff0)
            } else if *dso == EXE {
                0x55d4_c3a0_0000 | (h & 0x000f_fff0)
            } else {
                0x7f3a_2b00_0000 | (h & 0x00ff_fff0)
            };
            if *sym == UNKNOWN {
                out.push_str(&format!("\t{addr:>16x} [unknown] ([unknown])\n"));
            } else {
                out.push_str(&format!("\t{addr:>16x} {sym}+0x{:x} ({dso})\n", (h >> 32) % 0x400));
            }
        }
        out.push('\n');
    }
    out
}

// ---------------------------------------------------------------- folded

const FOLDED: &str = "\
Shop.Program.Main;Shop.Orders.OrderService.PlaceOrder 80
Shop.Program.Main;Shop.Orders.OrderService.PlaceOrder;Shop.Orders.OrderService.Validate 80
Shop.Program.Main;Shop.Orders.OrderService.PlaceOrder;Shop.Orders.OrderService.Validate;System.Text.RegularExpressions.Regex.IsMatch 40
Shop.Program.Main;Shop.Orders.OrderService.PlaceOrder;Shop.Orders.OrderRepository.Save;Microsoft.Data.SqlClient.SqlCommand.ExecuteNonQuery 220
Shop.Program.Main;Shop.Orders.OrderService.PlaceOrder;Shop.Orders.OrderRepository.Save;Microsoft.Data.SqlClient.SqlConnection.Open 60
Shop.Program.Main;Shop.Orders.OrderService.PlaceOrder;Shop.Orders.PricingEngine.Quote 120
Shop.Program.Main;Shop.Orders.OrderService.PlaceOrder;Shop.Orders.PricingEngine.Quote;Shop.Orders.DiscountRules.Apply 90
Shop.Program.Main;Shop.Orders.OrderService.PlaceOrder;Shop.Orders.OrderService.Serialize;System.Text.Json.JsonSerializer.Serialize 60
Shop.Program.Main;Shop.Orders.OrderService.PlaceOrder;Shop.Orders.OrderService.Serialize;System.Text.Json.JsonSerializer.Serialize;System.Text.Json.Utf8JsonWriter.WriteString 40
Shop.Program.Main;Shop.Orders.OrderService.PlaceOrder;Shop.Orders.Inventory.Reserve 50
Shop.Program.Main;Shop.Orders.OrderService.PlaceOrder;Shop.Orders.Inventory.Reserve;System.Threading.Monitor.Enter 70
Shop.Program.Main;Microsoft.Extensions.Hosting.HostingAbstractionsHostExtensions.Run;System.Threading.Tasks.Task.Wait 30
System.Threading.ThreadPoolWorkQueue.Dispatch;Shop.Orders.EmailSender.SendConfirmation;System.Net.Mail.SmtpClient.Send 60
";

const FOLDED_REGRESSION: &str = "\
Shop.Program.Main;Shop.Orders.OrderService.PlaceOrder 80
Shop.Program.Main;Shop.Orders.OrderService.PlaceOrder;Shop.Orders.OrderService.Validate 80
Shop.Program.Main;Shop.Orders.OrderService.PlaceOrder;Shop.Orders.OrderService.Validate;System.Text.RegularExpressions.Regex.IsMatch 40
Shop.Program.Main;Shop.Orders.OrderService.PlaceOrder;Shop.Orders.OrderRepository.Save;Microsoft.Data.SqlClient.SqlCommand.ExecuteNonQuery 220
Shop.Program.Main;Shop.Orders.OrderService.PlaceOrder;Shop.Orders.OrderRepository.Save;Microsoft.Data.SqlClient.SqlConnection.Open 60
Shop.Program.Main;Shop.Orders.OrderService.PlaceOrder;Shop.Orders.PricingEngine.Quote 120
Shop.Program.Main;Shop.Orders.OrderService.PlaceOrder;Shop.Orders.PricingEngine.Quote;Shop.Orders.DiscountRules.Apply 90
Shop.Program.Main;Shop.Orders.OrderService.PlaceOrder;Shop.Orders.OrderService.Serialize;System.Text.Json.JsonSerializer.Serialize 240
Shop.Program.Main;Shop.Orders.OrderService.PlaceOrder;Shop.Orders.OrderService.Serialize;System.Text.Json.JsonSerializer.Serialize;System.Text.Json.Utf8JsonWriter.WriteString 150
Shop.Program.Main;Shop.Orders.OrderService.PlaceOrder;Shop.Orders.Inventory.Reserve 50
Shop.Program.Main;Shop.Orders.OrderService.PlaceOrder;Shop.Orders.Inventory.Reserve;System.Threading.Monitor.Enter 70
Shop.Program.Main;Shop.Orders.OrderService.PlaceOrder;Shop.Orders.AuditLog.Write 20
Shop.Program.Main;Shop.Orders.OrderService.PlaceOrder;Shop.Orders.AuditLog.Write;System.IO.FileStream.Write 60
Shop.Program.Main;Microsoft.Extensions.Hosting.HostingAbstractionsHostExtensions.Run;System.Threading.Tasks.Task.Wait 30
System.Threading.ThreadPoolWorkQueue.Dispatch;Shop.Orders.EmailSender.SendConfirmation;System.Net.Mail.SmtpClient.Send 60
";

// ---------------------------------------------------------------- generation

#[test]
#[ignore = "writes samples/profiling; run with DEBRIEF_REGENERATE_SAMPLES=1"]
fn regenerate_samples() {
    if std::env::var("DEBRIEF_REGENERATE_SAMPLES").as_deref() != Ok("1") {
        eprintln!("set DEBRIEF_REGENERATE_SAMPLES=1 to write the samples");
        return;
    }
    let d = dir();
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("orders.pb.gz"), gz(&cpu_profile())).unwrap();
    std::fs::write(d.join("orders-alloc.pb.gz"), gz(&alloc_profile())).unwrap();
    std::fs::write(d.join("orders.perf.txt"), perf_script()).unwrap();
    std::fs::write(d.join("orders.folded"), FOLDED).unwrap();
    std::fs::write(d.join("orders-regression.folded"), FOLDED_REGRESSION).unwrap();
}

fn read(name: &str) -> Vec<u8> {
    let p = dir().join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e} (regenerate the samples, see the top of this file)", p.display()))
}

#[test]
fn committed_samples_match_the_generators() {
    // Compare decompressed bytes: gzip output may differ between flate2 versions.
    assert_eq!(gunzip(&read("orders.pb.gz")), cpu_profile(), "orders.pb.gz is stale");
    assert_eq!(gunzip(&read("orders-alloc.pb.gz")), alloc_profile(), "orders-alloc.pb.gz is stale");
    assert_eq!(
        String::from_utf8(read("orders.perf.txt")).unwrap(),
        perf_script(),
        "orders.perf.txt is stale"
    );
    assert_eq!(String::from_utf8(read("orders.folded")).unwrap(), FOLDED);
    assert_eq!(String::from_utf8(read("orders-regression.folded")).unwrap(), FOLDED_REGRESSION);
    for n in ["orders.pb.gz", "orders-alloc.pb.gz"] {
        assert!(read(n).len() < 50 * 1024, "{n} stays small");
    }
}

// ---------------------------------------------------------------- shapes

fn import(importer: &dyn CaptureImporter, name: &str) -> Profile {
    let bytes = read(name);
    let head = &bytes[..bytes.len().min(64 * 1024)];
    let mine = importer.sniff(name, head);
    assert!(mine >= 60, "{} sniffs {name} at {mine}", importer.id());
    match importer.import(name, &bytes, &Limits::default()).unwrap() {
        Capture::Profile(p) => p,
        Capture::Heap(_) => panic!("{name} is a profile"),
    }
}

/// Share (0..1) of samples whose stack contains a frame whose name contains `needle`.
fn share(p: &Profile, metric: MetricId, needle: &str) -> f64 {
    let m = p.metric_index(metric).unwrap();
    let stride = p.metrics.len();
    let (mut hit, mut all) = (0u64, 0u64);
    for i in 0..p.samples.len() {
        let v = p.samples.values[i * stride + m];
        all += v;
        if p.stack_frames(p.samples.stack[i])
            .iter()
            .any(|&f| p.frames[f as usize].function.contains(needle))
        {
            hit += v;
        }
    }
    hit as f64 / all as f64
}

#[test]
fn orders_cpu_profile_imports() {
    let p = import(&PprofImporter, "orders.pb.gz");
    assert_eq!(p.kind, ProfileKind::Cpu);
    assert_eq!(p.name, "orders.pb");
    assert_eq!(
        p.metrics.iter().map(|m| m.id).collect::<Vec<_>>(),
        [MetricId::Samples, MetricId::TimeNs]
    );
    let stacks: std::collections::HashSet<u32> = p.samples.stack.iter().copied().collect();
    assert_eq!(stacks.len(), CPU_STACKS.len());
    assert!((38..=45).contains(&stacks.len()));
    let gc = share(&p, MetricId::TimeNs, "runtime.gcBgMarkWorker");
    assert!((0.08..0.12).contains(&gc), "GC share {gc}");
    let lock = share(&p, MetricId::TimeNs, "sync.(*Mutex).Lock");
    assert!((0.06..0.10).contains(&lock), "mutex share {lock}");
    let threads: Vec<&str> = p.threads.iter().map(|t| t.name.as_str()).collect();
    assert!(
        threads.contains(&"http-worker-1") && threads.contains(&"gc-worker-2"),
        "{threads:?}"
    );
    let validate = p.frames.iter().find(|f| f.function == "orders.(*Service).validate").unwrap();
    assert_eq!(validate.module.as_deref(), Some("orders-svc"));
    assert_eq!(validate.file.as_deref(), Some("/src/orders-svc/internal/orders/service.go"));
    let vdso = p.frames.iter().find(|f| f.function == "__vdso_clock_gettime").unwrap();
    assert_eq!((vdso.module.as_deref(), vdso.kind), (Some("[vdso]"), FrameKind::Native));
    assert_eq!(p.started_ms, Some(1_791_968_400_000));
    assert_eq!(p.duration_ns, 30_000_000_000);
    assert!(!p.has_time);
    assert!(p.notes.is_empty(), "{:?}", p.notes);
}

#[test]
fn orders_alloc_profile_imports() {
    let p = import(&PprofImporter, "orders-alloc.pb.gz");
    assert_eq!(p.kind, ProfileKind::Alloc);
    assert_eq!(
        p.metrics.iter().map(|m| m.id).collect::<Vec<_>>(),
        [MetricId::AllocCount, MetricId::AllocBytes]
    );
    assert_eq!(p.samples.len(), ALLOC_STACKS.len());
    assert!(p.notes.iter().filter(|n| n.contains("inuse_")).count() == 2, "{:?}", p.notes);
    let json = share(&p, MetricId::AllocCount, "encoding/json");
    assert!(json > 0.25, "json allocations {json}");
}

#[test]
fn orders_perf_script_imports() {
    let p = import(&PerfScriptImporter, "orders.perf.txt");
    assert_eq!(p.samples.len(), PERF_SAMPLES);
    assert!(p.has_time);
    assert_eq!(
        p.metrics.iter().map(|m| m.id).collect::<Vec<_>>(),
        [MetricId::Samples, MetricId::TimeNs]
    );
    assert!(p.samples.values.chunks(2).all(|v| v == [1, 1_000_000]));
    let threads: Vec<(Option<u64>, &str)> = p.threads.iter().map(|t| (t.os_id, t.name.as_str())).collect();
    assert_eq!(threads.len(), 2);
    assert!(threads.contains(&(Some(4242), "orders-svc")) && threads.contains(&(Some(4251), "orders-io")));
    // 1 to 1.5 ms between samples.
    let n = PERF_SAMPLES as u64;
    assert!(
        p.duration_ns > (n - 1) * 1_000_000 && p.duration_ns < n * 1_500_000,
        "{}",
        p.duration_ns
    );
    for (kind, name) in [
        (FrameKind::Kernel, "do_syscall_64"),
        (FrameKind::Native, "malloc"),
        (FrameKind::Unknown, "[unknown]"),
        (FrameKind::Native, "orders::OrderService::placeOrder(orders::Order const&)"),
    ] {
        let f = p.frames.iter().find(|f| f.function == name).unwrap_or_else(|| panic!("{name}"));
        assert_eq!(f.kind, kind, "{name}");
    }
    assert!(p.frames.iter().any(|f| f.module.as_deref() == Some("libc.so.6") && f.is_framework));
    assert!(p.notes.is_empty(), "{:?}", p.notes);
}

#[test]
fn orders_folded_and_regression_import() {
    let base = import(&FoldedImporter, "orders.folded");
    let reg = import(&FoldedImporter, "orders-regression.folded");
    for p in [&base, &reg] {
        assert_eq!(p.kind, ProfileKind::Cpu);
        assert_eq!(p.threads.len(), 1);
        assert!(p.notes.is_empty());
    }
    let before = share(&base, MetricId::Samples, "OrderService.Serialize");
    let after = share(&reg, MetricId::Samples, "OrderService.Serialize");
    assert!((0.09..0.11).contains(&before), "{before}");
    assert!((0.25..0.32).contains(&after), "{after}");
    assert!(!base.frames.iter().any(|f| f.function == "Shop.Orders.AuditLog.Write"));
    assert!(reg.frames.iter().any(|f| f.function == "Shop.Orders.AuditLog.Write"));
}

/// Every sample in `samples/profiling` (including other importers' formats)
/// is claimed most strongly by its own importer among these three, and the
/// others stay low.
#[test]
fn sniffers_do_not_claim_each_others_samples() {
    let importers: [(&dyn CaptureImporter, &[&str]); 3] = [
        (&PprofImporter, &["orders.pb.gz", "orders-alloc.pb.gz"]),
        (&PerfScriptImporter, &["orders.perf.txt"]),
        (&FoldedImporter, &["orders.folded", "orders-regression.folded"]),
    ];
    let mut seen = 0;
    for entry in std::fs::read_dir(dir()).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let bytes = std::fs::read(&path).unwrap();
        let head = &bytes[..bytes.len().min(64 * 1024)];
        for (imp, own) in &importers {
            let score = imp.sniff(&name, head);
            if own.contains(&name.as_str()) {
                assert!(score >= 85, "{} sniffs its own {name} at {score}", imp.id());
                seen += 1;
            } else {
                // Unreadable gzip may get a weak claim; nothing else may.
                let max = if bytes.starts_with(&[0x1f, 0x8b]) { 20 } else { 0 };
                assert!(score <= max, "{} claims {name} with {score}", imp.id());
            }
        }
    }
    assert_eq!(seen, 5, "all five samples were checked");
}
