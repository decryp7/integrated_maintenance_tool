//! Engine benchmarks: importing scaled-up samples, and paging, facets,
//! timeline and chart bucketing on a loaded workspace.
//!
//! Run with `cargo bench -p imt-core` (or `-- --quick` for a fast pass); the
//! HTML report lands in `target/criterion/report/index.html`. Sizes are kept
//! small enough for a quick run by default and can be raised with
//! `IMT_BENCH_IMPORT_LINES` (lines per importer input, default 250 000) and
//! `IMT_BENCH_WORKSPACE_LINES` (lines per workspace file, default 100 000).

// The app's allocator (`app/src-tauri/src/main.rs`), so the numbers are the shipped ones.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use imt_core::importers::{ImportCtx, Importers};
use imt_core::model::Level;
use imt_core::patterns::{MinerConfig, PatternMiner};
use imt_core::profile::CompiledProfile;
use imt_core::query::Query;
use imt_core::workspace::{ChartRequest, FacetRequest, PageRequest, TimelineRequest};
use imt_core::{Profile, Workspace};
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// One sample per built-in importer: (benchmark name, path under `/samples`, importer id, header lines).
const SAMPLES: &[(&str, &str, &str, usize)] = &[
    ("clef", "serilog-clef/web01/orders-20261014.clef", "json", 0),
    ("serilog-text", "serilog-text/worker01/worker-20261014.txt", "text", 0),
    ("log4net", "log4net/legacy-billing/billing.log.2026-10-14", "text", 0),
    ("csv", "csv/perfmon-export.csv", "csv", 1),
    // A log server's file: three products, three formats (Serilog text, log4net, CLEF lines) line by line.
    ("logserver", "linux/logserver01/apps-20261014.log", "text", 1),
];

fn samples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples")
}

fn env_lines(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(default)
}

/// The sample repeated until it has at least `lines` lines; the first `header` lines are kept once.
fn scaled(rel: &str, header: usize, lines: usize) -> (String, usize) {
    let text = std::fs::read_to_string(samples().join(rel)).expect("sample is readable UTF-8");
    let all: Vec<&str> = text.lines().collect();
    let (head, body) = all.split_at(header.min(all.len()));
    assert!(!body.is_empty(), "{rel} has no body lines");
    let mut out = String::with_capacity(text.len() * (lines / body.len() + 1));
    for h in head {
        out.push_str(h);
        out.push('\n');
    }
    let mut n = 0;
    while n < lines {
        for line in body {
            out.push_str(line);
            out.push('\n');
        }
        n += body.len();
    }
    (out, n)
}

fn import(c: &mut Criterion) {
    let lines = env_lines("IMT_BENCH_IMPORT_LINES", 250_000);
    let profile = CompiledProfile::compile(Profile::default_profile()).expect("default profile compiles");
    let importers = Importers::new();
    let mut group = c.benchmark_group("import");
    group
        .sample_size(10)
        .warm_up_time(Duration::from_millis(500))
        .measurement_time(Duration::from_secs(5));
    for &(name, rel, id, header) in SAMPLES {
        let (content, n) = scaled(rel, header, lines);
        let importer = importers.find(id).expect("built-in importer");
        let ctx = ImportCtx {
            profile: &profile,
            path: Path::new(rel),
            file_date: None,
        };
        group.throughput(Throughput::Bytes(content.len() as u64));
        group.bench_with_input(BenchmarkId::new(name, n), &content, |b, content| {
            b.iter(|| {
                let imported = importer.import(&ctx, black_box(content)).expect("sample imports");
                black_box(imported.records.len())
            })
        });
    }
    group.finish();
}

/// Scaled copies of the samples in a private temporary folder, removed on drop.
struct ScaledFiles {
    dir: PathBuf,
    paths: Vec<PathBuf>,
}

impl ScaledFiles {
    fn write(lines: usize) -> ScaledFiles {
        let dir = std::env::temp_dir().join(format!("imt-bench-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temporary folder");
        let paths = SAMPLES
            .iter()
            .map(|&(_, rel, _, header)| {
                let file_name = Path::new(rel).file_name().expect("sample file name");
                let path = dir.join(file_name);
                std::fs::write(&path, scaled(rel, header, lines).0).expect("write scaled sample");
                path
            })
            .collect();
        ScaledFiles { dir, paths }
    }
}

impl Drop for ScaledFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn queries(c: &mut Criterion) {
    let lines = env_lines("IMT_BENCH_WORKSPACE_LINES", 100_000);
    let files = ScaledFiles::write(lines);
    let mut ws = Workspace::new(Profile::default_profile()).expect("default profile");
    ws.open(&files.paths).expect("scaled samples open");
    let total = ws.records().len();
    let errors = Query {
        levels: vec![Level::Error, Level::Fatal],
        ..Default::default()
    };
    let text = Query {
        text: "timeout".into(),
        ..Default::default()
    };

    let mut group = c.benchmark_group("workspace");
    group
        .sample_size(20)
        .warm_up_time(Duration::from_millis(500))
        .measurement_time(Duration::from_secs(3));
    group.throughput(Throughput::Elements(total as u64));

    for (name, query, offset, descending) in [
        ("page/first", Query::default(), 0, false),
        ("page/middle", Query::default(), total / 2, false),
        ("page/last-descending", Query::default(), 0, true),
        ("page/errors", errors.clone(), 0, false),
        ("page/text", text.clone(), 0, false),
    ] {
        let req = PageRequest {
            query,
            offset,
            limit: 500,
            descending,
            anchor: None,
        };
        group.bench_function(name, |b| b.iter(|| black_box(ws.page(black_box(&req)).expect("page").rows.len())));
    }

    for (name, field) in [
        ("facet/level", "@level"),
        ("facet/machine", "MachineName"),
        ("facet/group", "@group"),
    ] {
        let req = FacetRequest {
            query: Query::default(),
            field: field.into(),
            limit: 50,
        };
        group.bench_function(name, |b| b.iter(|| black_box(ws.facet(black_box(&req)).expect("facet").distinct)));
    }

    for (name, query) in [("timeline/all", Query::default()), ("timeline/errors", errors.clone())] {
        let req = TimelineRequest {
            query,
            buckets: 400,
            always_show_lifecycle: true,
        };
        group.bench_function(name, |b| {
            b.iter(|| black_box(ws.timeline(black_box(&req)).expect("timeline").lanes.len()))
        });
    }

    for (name, query) in [("chart/by-level", Query::default()), ("chart/text", text)] {
        let req = ChartRequest {
            query,
            chart_id: Some("events-by-level".into()),
            def: None,
            buckets: 400,
        };
        group.bench_function(name, |b| {
            b.iter(|| black_box(ws.chart(black_box(&req)).expect("chart").series.len()))
        });
    }
    // Analyses whose per-event loops are maps keyed by log content.
    let all = imt_core::api::QueryRequest { query: Query::default() };
    group.bench_function("analysis/exceptions", |b| {
        b.iter(|| black_box(ws.exception_groups(black_box(&all)).expect("exceptions").len()))
    });
    group.bench_function("analysis/patterns", |b| {
        b.iter(|| {
            let mut miner = PatternMiner::new(MinerConfig::default());
            for r in ws.records() {
                miner.add(r);
            }
            black_box(miner.overflowed())
        })
    });
    group.finish();
}

criterion_group!(benches, import, queries);
criterion_main!(benches);
