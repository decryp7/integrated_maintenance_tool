//! Mixed sources (ADR 0020) end to end on `samples/linux/logserver01`: one file written by several products
//! with different text formats, JSON lines and several processes; routes splitting it into logical
//! sources; the per-source instance key; unmatched lines reported as problems; the streaming import
//! reading the file the same way.

use debrief_core::api::SourceInfo;
use debrief_core::batch::{EventSchema, PendingBatch, SourceId};
use debrief_core::importers::Importers;
use debrief_core::ingest::{import_path, BatchSink, ImportEnv, NewSource, Outcome, ProblemKind, SourceReport};
use debrief_core::model::{Level, Record};
use debrief_core::profile::CompiledProfile;
use debrief_core::query::group_key;
use debrief_core::skew::{OffsetSpec, SetOffsetRequest};
use debrief_core::{Profile, Workspace};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

fn samples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples")
}

fn apps_log() -> PathBuf {
    samples().join("linux/logserver01/apps-20261014.log")
}

fn logserver_profile() -> Profile {
    Profile::from_toml(&std::fs::read_to_string(samples().join("profiles/logserver.debriefprofile.toml")).unwrap()).unwrap()
}

fn open(profile: Profile, paths: &[PathBuf]) -> Workspace {
    let mut ws = Workspace::new(profile).unwrap();
    ws.open(paths).unwrap();
    ws
}

/// What the file holds, from its text: (event lines, JSON lines, haproxy lines that no format reads).
fn expected() -> (usize, usize, Vec<u32>) {
    let text = std::fs::read_to_string(apps_log()).unwrap();
    let (mut events, mut json, mut unmatched) = (0, 0, Vec::new());
    for (i, l) in text.lines().enumerate() {
        if l.starts_with("2026-") {
            events += 1;
        } else if l.starts_with('{') {
            events += 1;
            json += 1;
        } else if l.starts_with("Oct ") {
            unmatched.push(i as u32 + 1);
        }
    }
    (events, json, unmatched)
}

fn name(ws: &Workspace, id: u32) -> String {
    ws.sources().iter().find(|s| s.id == id).map(|s| s.name.clone()).unwrap()
}

#[test]
fn every_line_is_read_with_its_own_format() {
    let ws = open(Profile::default_profile(), &[apps_log()]);
    let (events, json, unmatched) = expected();
    let [source] = ws.sources() else { panic!("{:?}", ws.sources()) };
    assert_eq!(source.importer, "text");
    assert_eq!(ws.records().len(), events);
    assert_eq!(source.skipped, 1, "the comment line before the first event");

    let field = |r: &Record, f: &str| r.fields.get(f).map(|v| v.as_str().map_or(v.to_string(), str::to_string));
    let inventory: Vec<&Record> = ws
        .records()
        .iter()
        .filter(|r| field(r, "Logger").as_deref() == Some("Inventory.Sync"))
        .collect();
    let orders: Vec<&Record> = ws
        .records()
        .iter()
        .filter(|r| field(r, "Application").as_deref() == Some("Checkout"))
        .collect();
    let payments: Vec<&Record> = ws
        .records()
        .iter()
        .filter(|r| field(r, "Application").as_deref() == Some("Payments"))
        .collect();
    assert_eq!(
        inventory.len() + orders.len() + payments.len(),
        events,
        "every event belongs to one product"
    );
    assert_eq!(payments.len(), json, "JSON lines are JSON events");
    assert!(payments.iter().all(|r| r.ts.is_some() && r.message.starts_with("Captured p-")));
    // log4net lines keep their level and logger although most lines are Serilog's.
    assert!(inventory.iter().all(|r| r.message.starts_with("host=SRV2 pid=")));
    assert!(inventory.iter().any(|r| r.level == Level::Warning) && inventory.iter().any(|r| r.level == Level::Error));
    assert!(inventory
        .iter()
        .filter(|r| r.level == Level::Error)
        .all(|r| r.exception.as_deref().is_some_and(|x| x.starts_with("java.io.IOException"))));
    // No product's lines continue another product's event.
    for r in &orders {
        let text = format!("{}\n{}", r.message, r.exception.as_deref().unwrap_or(""));
        assert!(!text.contains("host=SRV2") && !text.contains("\"@t\""), "line {}: {text}", r.line);
    }
    assert!(orders
        .iter()
        .filter(|r| r.level == Level::Error)
        .all(|r| r.exception.as_deref().is_some_and(|x| x.starts_with("System.TimeoutException"))));

    // The haproxy lines match no format: kept with the event above, reported.
    let problem = source.problems.iter().find(|p| p.kind == ProblemKind::UnmatchedLine).unwrap();
    assert_eq!(problem.count as usize, unmatched.len());
    assert_eq!(problem.sample_lines, unmatched);

    // Lanes: one per Checkout/Payments process; Inventory (no key fields) is one lane, its source.
    let summary = ws.summary();
    assert_eq!(summary.default_group_by, ["Application", "MachineName", "ProcessId"]);
    let lanes: BTreeSet<String> = ws
        .records()
        .iter()
        .map(|r| group_key(&summary.default_group_by, r, &source.name))
        .collect();
    let want: BTreeSet<String> = [
        "Checkout · APP01 · 3120",
        "Checkout · APP01 · 3388",
        "Checkout · APP02 · 3410",
        "Payments · PAY01 · 6100",
        source.name.as_str(),
    ]
    .into_iter()
    .map(String::from)
    .collect();
    assert_eq!(lanes, want);
}

#[test]
fn routes_split_the_file_into_logical_sources() {
    let ws = open(logserver_profile(), &[apps_log()]);
    let (events, ..) = expected();
    let physical = "logserver01/apps-20261014.log";
    let by_name: BTreeMap<&str, &SourceInfo> = ws.sources().iter().map(|s| (s.name.as_str(), s)).collect();
    assert_eq!(
        by_name.keys().copied().collect::<Vec<_>>(),
        ["Checkout@APP01", "Checkout@APP02", "Inventory@SRV2", "Payments@PAY01"],
        "every event is routed, so the file has no source of its own"
    );
    let path = apps_log().canonicalize().unwrap().to_string_lossy().into_owned();
    for s in ws.sources() {
        assert_eq!(s.routed_from.as_deref(), Some(physical));
        assert_eq!(s.path, path);
        // Rotated copies of the file (apps-20261015.log...) share the group of each logical source.
        assert_eq!(s.group, format!("logserver01/apps.log › {}", s.name));
        assert_eq!(s.records, ws.records().iter().filter(|r| r.source == s.id).count());
    }
    assert_eq!(ws.records().len(), events);
    let counted: usize = ws.sources().iter().map(|s| s.records).sum();
    assert_eq!(counted, events);
    // File-wide counts are on one source only.
    assert_eq!(ws.sources().iter().map(|s| s.skipped).sum::<usize>(), 1);
    assert_eq!(ws.sources().iter().filter(|s| !s.problems.is_empty()).count(), 1);

    // The instance key is chosen per logical source: Inventory's extracted Pid counts although Inventory
    // writes a minority of the events; its constant Host does not split lanes.
    let key = ws.summary().default_group_by;
    assert_eq!(key, ["Application", "MachineName", "ProcessId", "Pid"]);
    let mut lanes: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for r in ws.records() {
        let source = name(&ws, r.source);
        lanes.entry(source.clone()).or_default().insert(group_key(&key, r, &source));
    }
    assert_eq!(
        lanes["Inventory@SRV2"],
        ["Inventory@SRV2 · 7001", "Inventory@SRV2 · 7002"].map(String::from).into()
    );
    assert_eq!(lanes["Checkout@APP01"].len(), 2, "before and after the recycle");
    assert_eq!(lanes["Payments@PAY01"].len(), 1);

    // A clock offset belongs to the file: every logical source of it moves.
    let mut ws = ws;
    let before: Vec<(u32, Option<i64>)> = ws.records().iter().map(|r| (r.line, r.ts)).collect();
    let payments = by_name_id(&ws, "Payments@PAY01");
    let request = SetOffsetRequest {
        spec: OffsetSpec::Manual {
            source: payments,
            offset_ms: 1_500,
        },
    };
    ws.set_source_offset(&request).unwrap();
    assert!(ws.sources().iter().all(|s| s.offset_ms == 1_500));
    let after: BTreeMap<u32, Option<i64>> = ws.records().iter().map(|r| (r.line, r.ts)).collect();
    assert!(before.iter().all(|(line, ts)| after[line] == ts.map(|t| t + 1_500)));

    // Removing one logical source removes the file.
    let id = by_name_id(&ws, "Payments@PAY01");
    ws.remove_source(id);
    assert!(ws.sources().is_empty() && ws.records().is_empty());
}

fn by_name_id(ws: &Workspace, name: &str) -> u32 {
    ws.sources().iter().find(|s| s.name == name).unwrap().id
}

#[test]
fn unrouted_events_stay_in_the_file_source_and_reload_keeps_the_split() {
    let toml = "[[route]]\nwhen = [{ field = 'Application', op = 'eq', value = 'Payments' }]\nsource = 'payments'\n";
    let mut ws = open(Profile::from_toml(toml).unwrap(), &[apps_log()]);
    let names = |ws: &Workspace| {
        ws.sources()
            .iter()
            .map(|s| (s.name.clone(), s.routed_from.is_some()))
            .collect::<Vec<_>>()
    };
    let want = vec![("logserver01/apps-20261014.log".to_string(), false), ("payments".to_string(), true)];
    assert_eq!(names(&ws), want);
    let own = &ws.sources()[0];
    assert_eq!(
        (own.skipped, own.problems.len()),
        (1, 1),
        "file-wide counts on the file's own source"
    );
    ws.reload().unwrap();
    assert_eq!(names(&ws), want);
}

#[test]
fn json_first_then_text_is_read_by_the_text_importer() {
    let dir = tempdir();
    let p = dir.join("mixed.log");
    std::fs::write(
        &p,
        "{\"@t\":\"2026-10-14T10:00:00Z\",\"@m\":\"json one\"}\n\
         {\"@t\":\"2026-10-14T10:00:01Z\",\"@m\":\"json two\",\"@l\":\"Error\"}\n\
         2026-10-14 10:00:02.000 +00:00 [WRN] text three\n\
         {\"order\": 1}\n\
         2026-10-14 10:00:03,000 [t] INFO Billing.Run - text four\n",
    )
    .unwrap();
    let ws = open(Profile::default_profile(), &[p]);
    assert_eq!(ws.sources()[0].importer, "text");
    let got: Vec<(&str, Level)> = ws.records().iter().map(|r| (r.message.as_str(), r.level)).collect();
    assert_eq!(
        got,
        [
            ("json one", Level::Information),
            ("json two", Level::Error),
            ("text three\n{\"order\": 1}", Level::Warning),
            ("text four", Level::Information),
        ]
    );
    // A plain JSON-lines file stays with the JSON importer.
    let p = dir.join("plain.json");
    std::fs::write(&p, "{\"@t\":\"2026-10-14T10:00:00Z\",\"@m\":\"a\"}\n").unwrap();
    assert_eq!(open(Profile::default_profile(), &[p]).sources()[0].importer, "json");
    std::fs::remove_dir_all(dir).ok();
}

fn tempdir() -> PathBuf {
    let d = std::env::temp_dir().join(format!("debrief-mixed-{}-{:?}", std::process::id(), std::thread::current().id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Counts events per source and keeps the reports.
#[derive(Default)]
struct Counting {
    next: u32,
    rows: usize,
    reports: Vec<SourceReport>,
}

impl BatchSink for Counting {
    fn begin_source(&mut self, _source: &NewSource) -> anyhow::Result<SourceId> {
        self.next += 1;
        Ok(SourceId(self.next - 1))
    }
    fn write(&mut self, batch: PendingBatch) -> anyhow::Result<()> {
        self.rows += batch.len();
        Ok(())
    }
    fn finish_source(&mut self, report: &SourceReport) -> anyhow::Result<()> {
        self.reports.push(report.clone());
        Ok(())
    }
    fn abort_source(&mut self, _id: SourceId) -> anyhow::Result<()> {
        Ok(())
    }
}

#[test]
fn the_streaming_import_reads_the_file_the_same_way() {
    let importers = Importers::default();
    let profile = debrief_core::parser::ParserSet::of_compiled(CompiledProfile::compile(Profile::default_profile()).unwrap());
    let cancel = AtomicBool::new(false);
    let env = ImportEnv {
        encodings: &Default::default(),
        importers: &importers,
        parsers: &profile,
        schema: Arc::new(EventSchema::plain()),
        cancel: &cancel,
        window: None,
    };
    let mut sink = Counting::default();
    let path = apps_log();
    let out = import_path(&env, &path, "apps.log", &HashSet::new(), &mut sink);
    let Outcome::Imported(report) = &out[0] else { panic!("{out:?}") };
    let (events, _, unmatched) = expected();
    // The streaming import keeps the comment line before the first event as one untimed event.
    assert_eq!(report.events, events + 1);
    assert_eq!(sink.rows, events + 1);
    let problem = report.problems.iter().find(|p| p.kind == ProblemKind::UnmatchedLine).unwrap();
    assert_eq!(problem.sample_lines, unmatched);
    assert!(Path::new(&report.path).ends_with("apps-20261014.log"));
}

/// An importer declaring identity fields its events carry under names no profile candidate lists.
struct Tagged;

impl debrief_core::importers::Importer for Tagged {
    fn id(&self) -> &'static str {
        "tagged"
    }
    fn sniff(&self, _ctx: &debrief_core::importers::ImportCtx, head: &str) -> u8 {
        if head.starts_with("tagged") {
            100
        } else {
            0
        }
    }
    fn import(&self, _ctx: &debrief_core::importers::ImportCtx, content: &str) -> anyhow::Result<debrief_core::importers::Imported> {
        let records = content
            .lines()
            .skip(1)
            .enumerate()
            .map(|(i, l)| {
                let mut r = Record::new(i as u32 + 2);
                r.ts = Some(1_791_968_400_000 + i as i64);
                r.fields.insert("Worker".into(), l.into());
                r
            })
            .collect();
        Ok(debrief_core::importers::Imported {
            records,
            ..Default::default()
        })
    }
    fn identity_fields(&self) -> Vec<String> {
        vec!["Worker".into()]
    }
}

#[test]
fn identity_fields_of_importers_join_the_candidates() {
    let dir = tempdir().join("tagged");
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("w.tagged");
    std::fs::write(
        &p,
        "tagged
a
b
a
",
    )
    .unwrap();
    let mut importers = Importers::new();
    importers.register(Box::new(Tagged));
    let mut ws = Workspace::with_importers(Profile::default_profile(), importers).unwrap();
    ws.open(&[p]).unwrap();
    assert_eq!(ws.summary().default_group_by, ["Worker"]);
    std::fs::remove_dir_all(dir).ok();
}
