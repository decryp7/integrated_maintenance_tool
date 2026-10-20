//! End-to-end checks against the generated files in /samples.

use imt_core::api::WorkspaceState;
use imt_core::bundle;
use imt_core::config::{snapshot, ChangeKind, CompareOptions, ConfigParsers, Limits};
use imt_core::model::Level;
use imt_core::project::ProjectFile;
use imt_core::query::{FieldFilter, Query};
use imt_core::skew::SuggestRequest;
use imt_core::workspace::{ChartRequest, FacetRequest, PageRequest, TimelineRequest};
use imt_core::{Profile, Workspace};
use std::path::{Path, PathBuf};

fn samples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples")
}

/// Folders of `/samples` that are not logs: `profiling` holds profiler
/// captures (opened by the profiling plugin, tested in `imt-plugin-profiling`),
/// `configs` the configuration-diff pairs, `share` a project and a bundle
/// (opened as such, tested below), `live` the live-tail demo script and `dumps` crash dumps and UMDH heap logs
/// (opened by the dumps plugin, tested in `imt-plugin-dumps`).
const NOT_LOGS: &[&str] = &["profiling", "configs", "share", "live", "dumps"];

/// Folders whose formats only plugins read (tested in the plugin crates):
/// Windows event logs, ProcMon/PerfMon CSV, the crash dump, msinfo/registry
/// exports, web server logs and HAR, syslog/journald and OTLP.
const PLUGIN_ONLY: &[&str] = &["windows", "web", "linux", "otel"];

/// Every entry of `/samples` that the built-in importers are expected to open.
fn log_samples() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(samples())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| !p.file_name().is_some_and(|n| NOT_LOGS.iter().chain(PLUGIN_ONLY).any(|x| n == *x)))
        .collect();
    paths.sort();
    paths
}

fn workspace(profile: Profile) -> Workspace {
    let mut ws = Workspace::new(profile).unwrap();
    ws.open(&log_samples()).unwrap();
    ws
}

fn open(dirs: &[&str]) -> Workspace {
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(&dirs.iter().map(|d| samples().join(d)).collect::<Vec<_>>()).unwrap();
    ws
}

fn sample_profile() -> Profile {
    let text = std::fs::read_to_string(samples().join("profiles/orderservice.imtprofile.toml")).unwrap();
    Profile::from_toml(&text).unwrap()
}

/// 2026-10-14 09:00 and 12:00 UTC, the scenario window of `samples/generate.py`.
const START: i64 = 1_791_968_400_000;
const END: i64 = 1_791_979_200_000;
/// WEB02's clock runs this far behind the gateway's (`clock-skew/`).
const SKEW_MS: i64 = 3_742;

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> TempDir {
        let p = std::env::temp_dir().join(format!("imt-samples-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        TempDir(std::fs::canonicalize(p).unwrap())
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn every_sample_format_is_recognised() {
    let ws = workspace(Profile::default_profile());
    let s = ws.summary();
    let importer = |name: &str| s.sources.iter().find(|x| x.name.ends_with(name)).map(|x| x.importer.as_str());
    assert_eq!(importer("web01/orders-20261014.clef"), Some("json"));
    assert_eq!(importer("gateway/gateway-20261014.json"), Some("json"));
    assert_eq!(importer("worker01/worker-20261014.txt"), Some("text"));
    assert_eq!(importer("billing.log.2026-10-14"), Some("text"));
    assert_eq!(importer("perfmon-export.csv"), Some("csv"));
    assert_eq!(importer("BILL01/process-monitor.clef"), Some("json"));
    assert_eq!(importer("gateway/requests-20261014.clef"), Some("json"));
    assert_eq!(importer("web02/requests-20261014.clef"), Some("json"));
    assert_eq!(importer("batch01/report-service-20261014.txt"), Some("text"));
    assert_eq!(importer("shared/workers-20261014.log"), Some("text"));
    for entry in [
        "notify01-logs-20261014.zip/app.log",
        "notify01-logs-20261014.zip/app.log.1",
        "notify01-logs-20261014.zip/app.log.2",
        "app.log.3.gz",
    ] {
        assert_eq!(importer(entry), Some("text"), "{entry}");
    }
    assert!(
        s.sources.iter().all(|x| !x.name.contains("profiling")),
        "profiler captures are not log samples"
    );
    // The diagnostics report is IMT's own session, recorded after the incident (13:00), with metadata
    // files (manifest, system info) that have no timestamps; diagnostics tests cover it.
    for s in s.sources.iter().filter(|s| !s.path.contains("imt-diagnostics-sample.zip")) {
        assert_eq!(s.skipped, 0, "{} skipped lines", s.name);
        assert!(s.from.is_some(), "{} has no timestamps", s.name);
        // SQL01 writes local time (UTC+2); only the formats plugin's ERRORLOG importer applies its "UTC adjustment".
        if !s.name.ends_with("SQL01/ERRORLOG") {
            let (from, to) = (s.from.unwrap(), s.to.unwrap());
            // Responses to the last requests are logged a moment after 12:00.
            assert!(START <= from && to <= END + 60_000, "{}: {from}..{to}", s.name);
        }
    }
    // Only the log4j XML events (the formats plugin reads them), the generator and its README are left,
    // besides the diagnostics report's notes and screenshots (the app's diagnostics view shows those).
    let mut unrecognized: Vec<&str> = s
        .unrecognized
        .iter()
        .map(|u| u.name.as_str())
        .filter(|n| !n.starts_with("imt-diagnostics-sample.zip/"))
        .collect();
    unrecognized.sort();
    assert_eq!(
        unrecognized,
        [
            "billing-api/billing-api.xml",
            "samples/README.md",
            "samples/generate.py",
            "scheduler/scheduler-20261014.nlog.xml"
        ]
    );
}

#[test]
fn plugin_formats_degrade_without_their_plugins() {
    // Without the plugins, the Windows exports and the binary formats are listed as unrecognised, never an error.
    let ws = open(PLUGIN_ONLY);
    let s = ws.summary();
    let unrecognized: Vec<&str> = s.unrecognized.iter().map(|u| u.name.as_str()).collect();
    for name in [
        "registry-export.reg",
        "msinfo32.nfo",
        "System.evtx",
        "Application.evtx",
        "OrderService.exe.4120.dmp",
    ] {
        assert!(unrecognized.iter().any(|n| n.ends_with(name)), "{name} in {unrecognized:?}");
    }
}
#[test]
fn log4net_fields_and_levels() {
    let ws = workspace(Profile::default_profile());
    let q = Query {
        filters: vec![FieldFilter {
            field: "Logger".into(),
            op: "eq".into(),
            value: Some("Billing.Service".into()),
        }],
        ..Default::default()
    };
    let page = ws
        .page(&PageRequest {
            query: q,
            offset: 0,
            limit: 10,
            descending: false,
            anchor: None,
        })
        .unwrap();
    assert_eq!(page.total, 2);
    assert_eq!(page.rows[0].fields["Thread"], 1);
    let errors = ws
        .records()
        .iter()
        .filter(|r| r.fields.contains_key("Logger") && r.level == Level::Error)
        .count();
    assert!(errors > 0);
    assert!(ws
        .records()
        .iter()
        .any(|r| r.fields.contains_key("Logger") && r.exception.as_deref().is_some_and(|x| x.contains("CommunicationException"))));
}

#[test]
fn timeline_shows_the_crash_and_restart() {
    let ws = workspace(sample_profile());
    let tl = ws
        .timeline(&TimelineRequest {
            query: Query::default(),
            buckets: 400,
            always_show_lifecycle: true,
        })
        .unwrap();
    let lane = |key: &str| {
        tl.lanes
            .iter()
            .find(|l| l.key == key)
            .unwrap_or_else(|| panic!("no lane {key}: {:?}", tl.lanes.iter().map(|l| &l.key).collect::<Vec<_>>()))
    };
    let crashed = lane("WEB02 · 4120");
    assert!(crashed.markers.iter().any(|m| m.kind == "fatal"));
    assert!(crashed.markers.iter().any(|m| m.kind == "start"));
    assert!(!crashed.markers.iter().any(|m| m.kind == "stop"));
    let restarted = lane("WEB02 · 5532");
    assert!(restarted.first > crashed.last);
    assert!(lane("WORKER01 · 2210").markers.iter().any(|m| m.kind == "stop"));

    // Lifecycle markers survive a level filter.
    let errors_only = Query {
        levels: vec![Level::Error],
        ..Default::default()
    };
    let tl = ws
        .timeline(&TimelineRequest {
            query: errors_only,
            buckets: 400,
            always_show_lifecycle: true,
        })
        .unwrap();
    let crashed = tl.lanes.iter().find(|l| l.key == "WEB02 · 4120").unwrap();
    assert!(crashed.markers.iter().any(|m| m.kind == "start"));
    assert!(crashed.matched < crashed.total);
}

#[test]
fn extracted_metrics_chart() {
    let ws = workspace(sample_profile());
    let s = ws.summary();
    let cpu = s.fields.iter().find(|f| f.name == "CpuPercent").unwrap();
    assert!(cpu.numeric && cpu.derived);
    let chart = ws
        .chart(&ChartRequest {
            query: Query::default(),
            chart_id: Some("cpu".into()),
            def: None,
            buckets: 200,
        })
        .unwrap();
    assert_eq!(chart.bucket_ms, 60_000);
    let keys: Vec<&str> = chart.series.iter().map(|s| s.key.as_str()).collect();
    assert!(keys.contains(&"WORKER01 · 2208"), "{keys:?}");
    let web02 = chart.series.iter().find(|s| s.key == "WEB02 · 4120").unwrap();
    let peak = web02.points.iter().filter_map(|p| p.1).fold(0.0, f64::max);
    assert!(peak > 90.0, "peak {peak}");
    assert!(chart.lifecycle.iter().any(|l| l.kind == "fatal"));
}

#[test]
fn paging_with_anchor_and_facets() {
    let ws = workspace(Profile::default_profile());
    let fatal = ws.records().iter().find(|r| r.level == Level::Fatal).unwrap().id;
    let page = ws
        .page(&PageRequest {
            query: Query::default(),
            offset: 0,
            limit: 100,
            descending: true,
            anchor: Some(fatal),
        })
        .unwrap();
    let idx = page.anchor_index.unwrap();
    assert_eq!(page.rows[idx - page.offset].id, fatal);

    let facet = ws
        .facet(&FacetRequest {
            query: Query::default(),
            field: "MachineName".into(),
            limit: 10,
        })
        .unwrap();
    let names: Vec<&str> = facet.values.iter().map(|v| v.value.as_str()).collect();
    for m in ["WEB01", "WEB02", "WORKER01", "GATEWAY"] {
        assert!(names.contains(&m), "{names:?}");
    }
}

#[test]
fn process_monitor_output_has_dotnet_runtime_counters() {
    let ws = open(&["monitor"]);
    let stats: Vec<_> = ws.records().iter().filter(|r| r.message.starts_with("Process stats")).collect();
    assert!(stats.len() > 1000);
    for field in [
        "CpuPercent",
        "WorkingSetMb",
        "DotnetGen2Collections",
        "DotnetHeapBytes",
        "DotnetTimeInGcPercent",
        "DotnetExceptionsPerSec",
    ] {
        assert!(stats.iter().all(|r| r.fields.contains_key(field)), "{field}");
    }
    let s = ws.summary();
    let gc = s.fields.iter().find(|f| f.name == "DotnetTimeInGcPercent").unwrap();
    assert!(gc.numeric);
    // The app pool recycle: an exit (warning, RestartedAs) and the restart.
    let exit = ws
        .records()
        .iter()
        .find(|r| r.message.starts_with("Application exited: w3wp.exe (6120)"))
        .unwrap();
    assert_eq!((exit.level, exit.fields["RestartedAs"].as_i64()), (Level::Warning, Some(7044)));
    assert!(ws
        .records()
        .iter()
        .any(|r| r.message == "Application started: w3wp.exe (7044)" && r.fields["Restarted"] == true));
}

#[test]
fn rotated_logs_are_read_from_the_zip_and_the_gzip() {
    let ws = open(&["archives"]);
    let s = ws.summary();
    let mut names: Vec<&str> = s.sources.iter().map(|x| x.name.as_str()).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "notify01/app.log.3.gz",
            "notify01/notify01-logs-20261014.zip/app.log",
            "notify01/notify01-logs-20261014.zip/app.log.1",
            "notify01/notify01-logs-20261014.zip/app.log.2",
        ]
    );
    // The zip's members cannot be opened on their own; the gzip file can, but its lines are compressed.
    let kinds: Vec<(&str, imt_core::api::FileKind)> = s.sources.iter().map(|x| (x.name.as_str(), x.file_kind)).collect();
    for (name, kind) in &kinds {
        let expected = if name.ends_with(".gz") {
            imt_core::api::FileKind::Compressed
        } else {
            imt_core::api::FileKind::ArchiveMember
        };
        assert_eq!(*kind, expected, "{name}");
    }
    // One lane (the log's single instance) lists the files its events come from.
    let tl = ws
        .timeline(&TimelineRequest {
            query: Query::default(),
            buckets: 20,
            always_show_lifecycle: false,
        })
        .unwrap();
    let mut ids: Vec<u32> = s.sources.iter().map(|x| x.id).collect();
    ids.sort_unstable();
    let mut listed: Vec<u32> = tl.lanes.iter().flat_map(|l| l.sources.iter().copied()).collect();
    listed.sort_unstable();
    listed.dedup();
    assert_eq!(listed, ids);
    assert!(tl.lanes.iter().all(|l| l.sources.windows(2).all(|w| w[0] < w[1])));
    // The rotation splits one continuous log: each file starts after the previous one ended.
    let mut ranges: Vec<(i64, i64)> = s.sources.iter().map(|x| (x.from.unwrap(), x.to.unwrap())).collect();
    ranges.sort();
    assert!(ranges.windows(2).all(|w| w[0].1 < w[1].0), "{ranges:?}");
    assert_eq!(ranges[0].0, START);
    // Recipients are e-mail addresses: something to redact.
    assert!(ws.records().iter().any(|r| r.message.contains("@example.com")));
}

#[test]
fn clock_skew_suggestion_finds_web02s_offset() {
    let ws = open(&["clock-skew"]);
    let r = ws.suggest_offsets(&SuggestRequest::default());
    let s = &r.suggestions[0];
    let web02 = ws.summary().sources.iter().find(|x| x.name.contains("web02")).unwrap().id;
    // Aligning WEB02 on the gateway moves it forward by the skew (less the network hop); the other way round, back.
    let offset = if s.source == web02 { s.suggested_ms } else { -s.suggested_ms };
    assert!((SKEW_MS - 15..=SKEW_MS).contains(&offset), "{s:?}");
    assert!(s.pairs > 1000 && s.confidence > 0.9, "{s:?}");
}

#[test]
fn configuration_pairs_differ_where_web02_was_changed() {
    let parsers = ConfigParsers::builtin();
    let dir = samples().join("configs");
    let d = snapshot::diff_folders(
        &parsers,
        &dir.join("WEB01"),
        &dir.join("WEB02"),
        &CompareOptions::default(),
        &Limits::default(),
    )
    .unwrap();
    for file in ["appsettings.json", "web.config", "orderservice.ini", "otel-collector.yaml"] {
        let f = d
            .files
            .iter()
            .find(|f| f.rel_path == file)
            .unwrap_or_else(|| panic!("{file} not compared: {:?}", d.files));
        assert_eq!(f.status, snapshot::FileStatus::Changed, "{file}");
        assert!(f.counts.as_ref().unwrap().differences() >= 2, "{file}: {f:?}");
    }
    let tree = |m: &str| imt_core::config::read_config(&parsers, &dir.join(m).join("appsettings.json"), &Limits::default()).unwrap();
    let diff = imt_core::config::diff(&tree("WEB01"), &tree("WEB02"), &CompareOptions::default()).unwrap();
    let leaves = |kind: ChangeKind| -> Vec<String> {
        diff.entries
            .iter()
            .filter(|e| e.kind == kind && !e.has_changes)
            .map(|e| e.display.clone())
            .collect()
    };
    let changed = leaves(ChangeKind::Changed);
    for path in ["Default", "Orders", "Enabled", "NewCheckout", "MaxConcurrentConnections"] {
        assert!(changed.iter().any(|p| p.ends_with(path)), "{path} not in {changed:?}");
    }
    assert!(leaves(ChangeKind::Removed).iter().any(|p| p.contains("HealthChecks")));
    assert!(leaves(ChangeKind::Added).iter().any(|p| p.contains("DumpOnCrash")));
}

#[test]
fn runs_samples_have_start_and_stop_markers() {
    let ws = open(&["runs"]);
    let tl = ws
        .timeline(&TimelineRequest {
            query: Query::default(),
            buckets: 400,
            always_show_lifecycle: true,
        })
        .unwrap();
    let keys: Vec<&str> = tl.lanes.iter().map(|l| l.key.as_str()).collect();
    let count = |key: &str, kind: &str| {
        let lane = tl
            .lanes
            .iter()
            .find(|l| l.key.contains(key))
            .unwrap_or_else(|| panic!("no lane {key} in {keys:?}"));
        lane.markers.iter().filter(|m| m.kind == kind).count()
    };
    // The report service: 7 runs started, 5 stopped (one cut off by the power loss, one still running).
    assert_eq!((count("BATCH01", "start"), count("BATCH01", "stop")), (7, 5), "{keys:?}");
    // The shared file: one lane per worker process, interleaved in one source.
    assert_eq!(tl.lanes.iter().filter(|l| l.key.contains("ImageWorker")).count(), 2, "{keys:?}");
    assert_eq!(count("7101", "start"), count("7101", "stop"), "worker A always stops");
    assert_eq!(count("7102", "start"), count("7102", "stop") + 1, "worker B abandons one batch");
    assert_eq!(ws.summary().sources.iter().filter(|s| s.name.contains("shared")).count(), 1);
}

fn restore(p: &ProjectFile) -> Workspace {
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    let state = WorkspaceState {
        roots: p.sources.clone(),
        removed: p.removed.clone(),
        offsets: p.clock.clone(),
    };
    let missing = ws.restore(Profile::from_toml(&p.profile).unwrap(), &state).unwrap();
    assert!(missing.is_empty(), "{missing:?}");
    ws
}

#[test]
fn project_opens_the_samples_with_web02s_clock_offset() {
    let p = ProjectFile::load(&samples().join("share/orderservice-incident.imtproj")).unwrap();
    assert_eq!(p.version, 2);
    assert!(p.sources.iter().all(|s| Path::new(s).is_dir()), "{:?}", p.sources);
    let ws = restore(&p);
    assert_eq!(ws.profile().name, "OrderService (sample)");
    let s = ws.summary();
    let skewed: Vec<(&str, i64)> = s
        .sources
        .iter()
        .filter(|x| x.offset_ms != 0)
        .map(|x| (x.name.as_str(), x.offset_ms))
        .collect();
    assert_eq!(skewed.len(), 1, "{skewed:?}");
    assert!(
        skewed[0].0.ends_with("web02/requests-20261014.clef") && skewed[0].1 == SKEW_MS,
        "{skewed:?}"
    );
}

#[test]
fn bundle_extracts_checks_out_and_opens() {
    let dest = TempDir::new("bundle");
    let b = bundle::extract(&samples().join("share/web02-clock-skew.imtz"), &dest.0).unwrap();
    let manifest = b.manifest.expect("a share manifest");
    assert_eq!(
        (manifest.format.as_str(), manifest.schema, manifest.entries.len()),
        ("imt-share", 1, 5)
    );
    let ws = restore(&ProjectFile::load(&b.project.expect("a project")).unwrap());
    let s = ws.summary();
    assert_eq!(s.sources.len(), 2);
    let web02 = s.sources.iter().find(|x| x.name.ends_with("1/requests-20261014.clef")).unwrap();
    assert_eq!(web02.offset_ms, SKEW_MS);
    // With the offset applied, WEB02 receives each request after the gateway forwarded it.
    let gateway = s.sources.iter().find(|x| x.id != web02.id).unwrap();
    let first = |src: u32| ws.records().iter().find(|r| r.source == src).unwrap();
    assert_eq!(
        first(gateway.id).fields.get("CorrelationId"),
        first(web02.id).fields.get("CorrelationId")
    );
    assert!(first(web02.id).ts > first(gateway.id).ts);
}
