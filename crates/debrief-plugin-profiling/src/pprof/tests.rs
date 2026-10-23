use super::proto::{Function, Label, Line, Location, Mapping, Profile as Pb, Sample, ValueType};
use super::*;
use debrief_profile::Profile;
use std::io::Write;

/// Builds pprof messages with an interned string table.
struct P {
    pb: Pb,
}

impl P {
    fn new(types: &[(&str, &str)]) -> P {
        let mut p = P {
            pb: Pb {
                string_table: vec![String::new()],
                ..Pb::default()
            },
        };
        for (t, u) in types {
            let vt = ValueType {
                r#type: p.s(t),
                unit: p.s(u),
            };
            p.pb.sample_type.push(vt);
        }
        p
    }

    fn s(&mut self, v: &str) -> i64 {
        if let Some(i) = self.pb.string_table.iter().position(|x| x == v) {
            return i as i64;
        }
        self.pb.string_table.push(v.to_string());
        (self.pb.string_table.len() - 1) as i64
    }

    fn mapping(&mut self, id: u64, file: &str) {
        let filename = self.s(file);
        self.pb.mapping.push(Mapping {
            id,
            filename,
            ..Mapping::default()
        });
    }

    fn function(&mut self, id: u64, name: &str, file: &str, start: i64) {
        let (name, filename) = (self.s(name), self.s(file));
        self.pb.function.push(Function {
            id,
            name,
            filename,
            start_line: start,
            ..Function::default()
        });
    }

    /// A location with lines (function ids, innermost first).
    fn location(&mut self, id: u64, mapping_id: u64, functions: &[u64]) {
        self.pb.location.push(Location {
            id,
            mapping_id,
            address: 0x1000 + id,
            line: functions
                .iter()
                .map(|&f| Line {
                    function_id: f,
                    line: 10,
                    column: 0,
                })
                .collect(),
            is_folded: false,
        });
    }

    fn sample(&mut self, locations: &[u64], values: &[i64], labels: &[(&str, Result<&str, i64>)]) {
        let label = labels
            .iter()
            .map(|(k, v)| {
                let key = self.s(k);
                match v {
                    Ok(s) => Label {
                        key,
                        str: self.s(s),
                        ..Label::default()
                    },
                    Err(n) => Label {
                        key,
                        num: *n,
                        ..Label::default()
                    },
                }
            })
            .collect();
        self.pb.sample.push(Sample {
            location_id: locations.to_vec(),
            value: values.to_vec(),
            label,
        });
    }

    fn bytes(&self) -> Vec<u8> {
        self.pb.encode_to_vec()
    }
}

fn gz(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

fn import(bytes: &[u8]) -> Result<Profile> {
    import_with(bytes, &Limits::default())
}

fn import_with(bytes: &[u8], limits: &Limits) -> Result<Profile> {
    match PprofImporter.import("cpu.pb.gz", bytes, limits)? {
        Capture::Profile(p) => Ok(p),
        Capture::Heap(_) => unreachable!(),
    }
}

fn names(p: &Profile, stack: u32) -> Vec<String> {
    p.stack_frames(stack)
        .iter()
        .map(|&f| p.frames[f as usize].function.clone())
        .collect()
}

/// main -> work (inlined helper in work) ; main -> sleep in the kernel.
fn cpu() -> P {
    let mut p = P::new(&[("samples", "count"), ("cpu", "nanoseconds")]);
    p.mapping(1, "/usr/local/bin/app");
    p.mapping(2, "[kernel.kallsyms]");
    p.function(1, "main.main", "/src/main.go", 5);
    p.function(2, "main.work", "/src/main.go", 20);
    p.function(3, "main.helper", "/src/main.go", 40);
    p.function(4, "do_syscall_64", "", 0);
    p.location(1, 1, &[1]);
    p.location(2, 1, &[3, 2]); // helper inlined into work
    p.location(3, 2, &[4]);
    p.sample(&[2, 1], &[3, 30_000_000], &[]);
    p.sample(&[3, 1], &[1, 10_000_000], &[]);
    p.pb.time_nanos = 1_760_000_000_123_000_000;
    p.pb.duration_nanos = 5_000_000_000;
    p
}

#[test]
fn imports_a_cpu_profile_with_inlined_frames() {
    let p = import(&gz(&cpu().bytes())).unwrap();
    assert_eq!(p.kind, ProfileKind::Cpu);
    assert_eq!(p.source, "pprof");
    assert_eq!(p.name, "cpu.pb");
    assert_eq!(
        p.metrics.iter().map(|m| m.id).collect::<Vec<_>>(),
        [MetricId::Samples, MetricId::TimeNs]
    );
    assert!(p.metrics.iter().all(|m| m.measurement == Measurement::Sampled));
    assert_eq!(p.samples.len(), 2);
    assert_eq!(names(&p, p.samples.stack[0]), ["main.main", "main.work", "main.helper"]);
    assert_eq!(names(&p, p.samples.stack[1]), ["main.main", "do_syscall_64"]);
    assert_eq!(&p.samples.values[..2], &[3, 30_000_000]);
    assert!(!p.has_time);
    assert_eq!(p.started_ms, Some(1_760_000_000_123));
    assert_eq!(p.duration_ns, 5_000_000_000);
    assert_eq!(p.threads.len(), 1);
    assert_eq!(p.threads[0].name, "all");
    let main = p.frames.iter().find(|f| f.function == "main.main").unwrap();
    assert_eq!(main.module.as_deref(), Some("app"));
    assert_eq!(main.file.as_deref(), Some("/src/main.go"));
    assert_eq!(main.line, Some(5));
    assert_eq!(main.kind, FrameKind::Native);
    let k = p.frames.iter().find(|f| f.function == "do_syscall_64").unwrap();
    assert_eq!(k.kind, FrameKind::Kernel);
    assert_eq!(k.line, None);
    assert!(p.notes.is_empty(), "{:?}", p.notes);
    // The same bytes uncompressed import the same way.
    assert_eq!(import(&cpu().bytes()).unwrap(), p);
}

#[test]
fn converts_time_units_and_maps_wait_and_contention_profiles() {
    for (unit, factor) in [
        ("microseconds", 1_000),
        ("milliseconds", 1_000_000),
        ("seconds", 1_000_000_000),
        ("ns", 1),
    ] {
        let mut p = P::new(&[("wall", unit)]);
        p.function(1, "f", "", 0);
        p.location(1, 0, &[1]);
        p.sample(&[1], &[7], &[]);
        let prof = import(&p.bytes()).unwrap();
        assert_eq!(prof.metrics[0].id, MetricId::TimeNs);
        assert_eq!(prof.samples.values, [7 * factor], "{unit}");
    }
    let mut p = P::new(&[("contentions", "count"), ("delay", "nanoseconds")]);
    p.function(1, "sync.(*Mutex).Lock", "", 0);
    p.location(1, 0, &[1]);
    p.sample(&[1], &[2, 500], &[]);
    let prof = import(&p.bytes()).unwrap();
    assert_eq!(prof.kind, ProfileKind::Wall);
    assert_eq!(
        prof.metrics.iter().map(|m| m.id).collect::<Vec<_>>(),
        [MetricId::Samples, MetricId::WaitNs]
    );
}

#[test]
fn heap_profiles_prefer_allocations_over_in_use() {
    let mut p = P::new(&[
        ("alloc_objects", "count"),
        ("alloc_space", "bytes"),
        ("inuse_objects", "count"),
        ("inuse_space", "bytes"),
    ]);
    p.function(1, "runtime.mallocgc", "", 0);
    p.location(1, 0, &[1]);
    p.sample(&[1], &[4, 4096, 1, 1024], &[]);
    let prof = import(&p.bytes()).unwrap();
    assert_eq!(prof.kind, ProfileKind::Alloc);
    assert_eq!(
        prof.metrics.iter().map(|m| m.id).collect::<Vec<_>>(),
        [MetricId::AllocCount, MetricId::AllocBytes]
    );
    assert_eq!(prof.samples.values, [4, 4096]);
    assert_eq!(
        prof.notes.iter().filter(|n| n.contains("was skipped")).count(),
        2,
        "{:?}",
        prof.notes
    );
    assert_eq!(prof.name, "cpu.pb");

    // In-use first, allocations later: allocations still win.
    let mut p = P::new(&[("inuse_space", "bytes"), ("alloc_space", "bytes")]);
    p.function(1, "f", "", 0);
    p.location(1, 0, &[1]);
    p.sample(&[1], &[1, 2], &[]);
    let prof = import(&p.bytes()).unwrap();
    assert_eq!(prof.samples.values, [2]);
    assert!(
        prof.notes.iter().any(|n| n.contains("inuse_space/bytes was skipped")),
        "{:?}",
        prof.notes
    );

    // Only in-use: shown, and said so.
    let mut p = P::new(&[("inuse_objects", "count"), ("inuse_space", "bytes")]);
    p.function(1, "f", "", 0);
    p.location(1, 0, &[1]);
    p.sample(&[1], &[1, 2], &[]);
    let prof = import(&p.bytes()).unwrap();
    assert_eq!(prof.name, "cpu.pb (in use)");
    assert!(prof.notes.iter().any(|n| n.contains("in use")));
}

#[test]
fn mixed_and_unsupported_sample_types() {
    let mut p = P::new(&[
        ("samples", "count"),
        ("alloc_space", "bytes"),
        ("weird", "furlongs"),
        ("calls", "count"),
    ]);
    p.function(1, "f", "", 0);
    p.location(1, 0, &[1]);
    p.sample(&[1], &[1, 2, 3, 4], &[]);
    let prof = import(&p.bytes()).unwrap();
    assert_eq!(prof.kind, ProfileKind::Mixed);
    assert!(prof.notes.iter().any(|n| n.contains("weird/furlongs is not supported")));
    assert!(prof.notes.iter().any(|n| n.contains("calls/count is not supported")));

    let mut p = P::new(&[("weird", "furlongs")]);
    p.sample(&[], &[1], &[]);
    let e = import(&p.bytes()).unwrap_err();
    assert!(matches!(e, ProfileError::Unsupported { .. }), "{e}");
    assert!(e.to_string().contains("weird/furlongs"));

    let p = P::new(&[]);
    assert!(import(&p.bytes()).unwrap_err().to_string().contains("no sample types"));

    let mut p = P::new(&[("goroutine", "count")]);
    p.sample(&[], &[1], &[]);
    let prof = import(&p.bytes()).unwrap();
    assert_eq!(names(&prof, prof.samples.stack[0]), ["(no stack)"]);
    let mut p = P::new(&[("samples", "count")]);
    p.sample(&[], &[1, 2], &[]);
    assert!(
        import(&p.bytes()).unwrap_err().to_string().contains("no samples"),
        "the only sample is skipped"
    );
}

#[test]
fn labels_become_threads_and_timestamps() {
    let mut p = P::new(&[("samples", "count")]);
    p.function(1, "f", "", 0);
    p.location(1, 0, &[1]);
    p.sample(&[1], &[1], &[("thread", Ok("worker-1")), ("timestamp", Err(5_000))]);
    p.sample(
        &[1],
        &[1],
        &[("threadid", Err(42)), ("thread_name", Ok("io")), ("timestamp", Err(2_000))],
    );
    p.sample(&[1], &[1], &[("goroutine", Err(17)), ("timestamp", Err(3_000))]);
    p.sample(&[1], &[1], &[("thread", Ok("worker-1")), ("timestamp", Err(9_000))]);
    let prof = import(&p.bytes()).unwrap();
    let names: Vec<&str> = prof.threads.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["worker-1", "io", "goroutine 17"]);
    assert_eq!(prof.threads[1].os_id, Some(42));
    assert!(prof.has_time);
    assert_eq!(prof.samples.time, [0, 1_000, 3_000, 7_000], "sorted, relative to the earliest");

    // Timestamps in ms; one sample without a timestamp disables time.
    let mut p = P::new(&[("samples", "count")]);
    p.function(1, "f", "", 0);
    p.location(1, 0, &[1]);
    let ms = p.s("ms");
    let key = p.s("timestamp");
    p.sample(&[1], &[1], &[]);
    p.pb.sample[0].label.push(Label {
        key,
        num: 2,
        num_unit: ms,
        ..Label::default()
    });
    let prof = import(&p.bytes()).unwrap();
    assert!(prof.has_time);
    p.sample(&[1], &[1], &[]);
    assert!(!import(&p.bytes()).unwrap().has_time);
}

#[test]
fn bad_references_are_noted_not_fatal() {
    let mut p = P::new(&[("samples", "count")]);
    p.mapping(1, "/lib/libfoo.so");
    p.function(1, "f", "", 0);
    p.function(1, "dup", "", 0);
    p.pb.function.push(Function {
        id: 2,
        name: 999,
        system_name: -1,
        filename: 12345,
        start_line: -3,
    });
    p.location(1, 1, &[1, 77]); // unknown function 77
    p.location(2, 1, &[]); // no symbols
    p.location(3, 9, &[]); // no symbols, unknown mapping
    p.location(4, 0, &[2]); // function with bad string indexes
    p.sample(&[1, 2, 3, 4, 555], &[1], &[]);
    p.sample(&[555], &[-4], &[]);
    p.sample(&[1], &[1, 2], &[]);
    p.pb.comment = vec![p.s("recorded by test"), 0];
    let prof = import(&p.bytes()).unwrap();
    assert_eq!(prof.samples.len(), 2);
    assert_eq!(
        names(&prof, prof.samples.stack[0]),
        ["(unknown location)", "(anonymous)", "0x1003", "0x1002", "(unknown function)", "f"]
    );
    let addr = prof.frames.iter().find(|f| f.function == "0x1002").unwrap();
    assert_eq!((addr.module.as_deref(), addr.kind), (Some("libfoo.so"), FrameKind::Native));
    let orphan = prof.frames.iter().find(|f| f.function == "0x1003").unwrap();
    assert_eq!((orphan.module.as_deref(), orphan.kind), (None, FrameKind::Unknown));
    assert_eq!(prof.samples.values, [1, 0]);
    let notes = prof.notes.join("\n");
    for expect in [
        "1 samples with the wrong number",
        "1 negative values",
        "2 stack entries refer to unknown locations",
        "1 source lines refer to unknown functions",
        "2 locations have no symbols",
        "1 functions, mappings or locations have duplicate ids",
        "references to missing strings",
        "comment: recorded by test",
    ] {
        assert!(notes.contains(expect), "missing {expect:?} in {notes}");
    }
}

#[test]
fn deep_stacks_are_cut_to_the_limit() {
    let mut p = P::new(&[("samples", "count")]);
    for i in 1..=50u64 {
        p.function(i, &format!("f{i}"), "", 0);
        p.location(i, 0, &[i, i]); // each location: the function inlined into itself
    }
    let ids: Vec<u64> = (1..=50).rev().collect();
    p.sample(&ids, &[1], &[]);
    let limits = Limits {
        max_stack_depth: 10,
        ..Limits::default()
    };
    let prof = import_with(&p.bytes(), &limits).unwrap();
    assert_eq!(prof.stack_frames(prof.samples.stack[0]).len(), 10);
    assert_eq!(names(&prof, prof.samples.stack[0])[0], "f1", "outermost frames kept");
    assert!(prof.notes.iter().any(|n| n.contains("deeper than 10")));
}

#[test]
fn enforces_limits_before_decoding() {
    let mut p = P::new(&[("samples", "count")]);
    for _ in 0..5 {
        p.sample(&[], &[1], &[]);
    }
    let small = Limits {
        max_samples: 4,
        ..Limits::default()
    };
    let e = import_with(&p.bytes(), &small).unwrap_err();
    assert!(matches!(e, ProfileError::LimitExceeded { what: "samples", .. }), "{e}");
    let tiny = Limits {
        max_frames: 0,
        ..Limits::default()
    };
    assert!(matches!(
        import_with(&p.bytes(), &tiny),
        Err(ProfileError::LimitExceeded { what: "strings", .. })
    ));
    // A gzip bomb is refused by the bounded inflater.
    let bomb = gz(&vec![0u8; 1 << 20]);
    let capped = Limits {
        max_input_bytes: 1 << 16,
        ..Limits::default()
    };
    assert!(matches!(import_with(&bomb, &capped), Err(ProfileError::LimitExceeded { .. })));
    // Raw input over the limit.
    let raw = p.bytes();
    let short = Limits {
        max_input_bytes: 3,
        ..Limits::default()
    };
    assert!(matches!(import_with(&raw, &short), Err(ProfileError::LimitExceeded { .. })));
}

#[test]
fn rejects_files_that_are_not_pprof() {
    assert!(import(b"").unwrap_err().to_string().contains("empty"));
    for bad in [
        &b"{\"nodes\":[]}"[..],
        b"main;foo 1\n",
        &[0x0a, 0x05, 0x08],
        &[0x1a, 0x02, 0x08, 0xff],
    ] {
        let e = import(bad).unwrap_err();
        assert!(matches!(e, ProfileError::Malformed { .. }), "{e}");
    }
    // Structurally valid top level, invalid inside (a string that is not UTF-8).
    let e = import(&[0x32, 0x01, 0xff]).unwrap_err();
    assert!(e.to_string().contains("not a pprof profile"), "{e}");
    assert!(import(&[0x1f, 0x8b, 0x08, 0x00]).is_err(), "damaged gzip");
}

#[test]
fn sniffs_pprof_and_nothing_else() {
    assert_eq!((PprofImporter.id(), PprofImporter.name()), ("pprof", "pprof (profile.proto)"));
    let raw = cpu().bytes();
    let z = gz(&raw);
    let s = |name: &str, head: &[u8]| PprofImporter.sniff(name, head);
    assert_eq!(s("cpu.pb.gz", &z), 95);
    assert_eq!(s("pprof.samples.cpu.001.pb.gz", &z), 95);
    assert_eq!(s("profile", &z), 75);
    assert_eq!(s("cpu.prof", &raw), 90);
    assert_eq!(s("heap.pprof", &raw), 90);
    assert_eq!(s("data.bin", &raw), 10);
    assert_eq!(s("x.json.gz", &gz(b"{\"shared\":{}}")), 0);
    assert_eq!(s("x.gz", &gz(b"hello")), 0);
    assert_eq!(s("x.gz", &[0x1f, 0x8b, 0xff, 0xff]), 20, "unreadable gzip: weak claim");
    assert_eq!(s("x.pb.gz", &[0x1f, 0x8b, 0xff, 0xff]), 40);
    assert_eq!(s("x.pb", b"main;foo 3\n"), 0);
    assert_eq!(s("x.pb", b""), 0);
    assert_eq!(s("x.prof", b"{\"nodes\":[]}"), 0);
    assert!(s("cut.pb.gz", &z[..z.len() / 2]) >= 75, "a cut head still sniffs");
}

#[test]
fn classifies_types_and_frame_kinds() {
    assert_eq!(classify("alloc_objects", "count").unwrap().metric, MetricId::AllocCount);
    assert_eq!(classify("allocations", "").unwrap().metric, MetricId::AllocCount);
    assert_eq!(classify("space", "BYTES").unwrap().metric, MetricId::AllocBytes);
    assert!(classify("inuse_space", "bytes").unwrap().in_use);
    assert_eq!(
        classify("lock_time", "s").unwrap(),
        Mapped {
            metric: MetricId::WaitNs,
            scale: 1_000_000_000,
            in_use: false
        }
    );
    assert_eq!(classify("events", "samples").unwrap().metric, MetricId::Samples);
    assert_eq!(classify("cpu", "count"), None);
    assert_eq!(kind_of(&[MetricId::TimeNs]), ProfileKind::Cpu);
    assert_eq!(kind_of(&[MetricId::AllocBytes]), ProfileKind::Alloc);
    assert_eq!(kind_of(&[MetricId::WaitNs]), ProfileKind::Wall);
    assert_eq!(kind_of(&[MetricId::TimeNs, MetricId::AllocBytes]), ProfileKind::Mixed);
    assert_eq!(frame_kind("/usr/bin/vmlinux-6.1", ""), FrameKind::Kernel);
    assert_eq!(frame_kind("node", "/app/server.js"), FrameKind::Managed);
    assert_eq!(frame_kind("python3", "orders.py"), FrameKind::Managed);
    assert_eq!(frame_kind("app", "main.rs"), FrameKind::Native);
    assert!(has_pprof_name(r"C:\tmp\heap.PROF"));
    assert!(!has_pprof_name("trace.json.gz"));
}
