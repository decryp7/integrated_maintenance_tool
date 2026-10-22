//! Workspace behaviour on small, purpose-built folders.

use anyhow::Result;
use imt_core::chart::ChartDef;
use imt_core::importers::{ImportCtx, Imported, Importer, Importers};
use imt_core::model::{Level, Record};
use imt_core::query::{FieldFilter, Query};
use imt_core::workspace::{ChartRequest, ExtractPreviewRequest, FacetRequest, PageRequest, TimelineRequest};
use imt_core::{LogStore, Profile, Workspace, DEFAULT_PROFILE_TOML};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A temporary folder removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> TempDir {
        static N: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!("imt-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }

    fn file(&self, name: &str, content: impl AsRef<[u8]>) -> PathBuf {
        let p = self.0.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, content).unwrap();
        p
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn clef(lines: &[(&str, &str, &str, &str)]) -> String {
    // (time, level, machine, message)
    lines
        .iter()
        .map(|(t, l, m, msg)| {
            format!(
                r#"{{"@t":"2026-10-14T{t}Z","@l":"{l}","@mt":"{msg}","MachineName":"{m}","Cpu":{}}}"#,
                msg.len()
            ) + "\n"
        })
        .collect()
}

fn page(ws: &Workspace, query: Query) -> Vec<(u32, String)> {
    let p = ws
        .page(&PageRequest {
            query,
            offset: 0,
            limit: 1000,
            descending: false,
            anchor: None,
        })
        .unwrap();
    p.rows.iter().map(|r| (r.id, r.message.to_string())).collect()
}

fn folder() -> TempDir {
    let d = TempDir::new();
    d.file(
        "a/app.clef",
        clef(&[
            ("10:00:00", "Information", "WEB01", "Application starting"),
            ("10:00:05", "Warning", "WEB01", "slow"),
            ("10:00:10", "Error", "WEB01", "boom"),
            ("10:00:20", "Information", "WEB01", "Application is shutting down"),
        ]),
    );
    d.file(
        "b/app.clef",
        clef(&[
            ("10:00:02", "Information", "WEB02", "Application starting"),
            ("10:00:30", "Fatal", "WEB02", "Host terminated unexpectedly"),
        ]),
    );
    d.file("empty.log", "");
    d.file("binary.dat", [0u8, 159, 146, 150, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
    d.file("notes.txt", "just some words\nwithout timestamps\n");
    d.file("array.json", "[]");
    d.file(".hidden/skip.log", "2026-10-14 10:00:00 INFO hidden\n");
    d.file("old-filtered.log", "2026-10-14 10:00:00 INFO filtered\n");
    d
}

#[test]
fn open_reports_sources_and_unrecognized_files() {
    let d = folder();
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(&[d.path().to_path_buf()]).unwrap();
    let s = ws.summary();
    assert_eq!(
        s.sources.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        vec!["a/app.clef", "b/app.clef"]
    );
    assert_eq!(s.total, 6);
    let reasons: Vec<(&str, &str)> = s.unrecognized.iter().map(|u| (u.name.as_str(), u.reason.as_str())).collect();
    assert_eq!(
        reasons,
        vec![
            ("array.json", "json importer found no events"),
            ("binary.dat", "binary file"),
            ("empty.log", "empty file"),
            ("notes.txt", "no enabled parser reads this file (see Settings ▸ Parsers)"),
        ]
    );
    assert_eq!(s.levels[&Level::Fatal], 1);
    assert_eq!(s.default_group_by, vec!["MachineName".to_string()]);
    assert_eq!(s.marker_kinds[0].kind, "fatal");
    assert_eq!(s.charts[0].id, "events-by-level");
    assert_eq!(ws.profile().name, "Default (Serilog)");
    let cpu = s.fields.iter().find(|f| f.name == "Cpu").unwrap();
    assert!(cpu.numeric && !cpu.derived);

    // Opening the same folder again adds nothing; a missing path is an error.
    ws.open(&[d.path().to_path_buf()]).unwrap();
    assert_eq!(ws.summary().total, 6);
    assert!(ws.open(&[d.path().join("missing")]).is_err());
}

#[test]
fn remove_reload_clear_and_set_profile() {
    let d = folder();
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(&[d.path().join("a/app.clef"), d.path().join("b")]).unwrap();
    let s = ws.summary();
    assert_eq!(s.sources.len(), 2);
    // A single file opened directly is named by its parent folder and file name.
    assert!(s.sources.iter().any(|x| x.name == "a/app.clef"));

    let first = s.sources[0].id;
    ws.remove_source(first);
    assert_eq!(ws.summary().sources.len(), 1);

    // Removing a file inside an opened folder keeps it out of reloads.
    let in_b = |ws: &Workspace| {
        ws.summary()
            .sources
            .iter()
            .find(|x| x.path.replace('\\', "/").ends_with("b/app.clef"))
            .map(|x| x.id)
    };
    ws.remove_source(in_b(&ws).unwrap());
    ws.reload().unwrap();
    assert_eq!(in_b(&ws), None);
    ws.open(&[d.path().join("b/app.clef")]).unwrap();
    assert!(in_b(&ws).is_some());

    d.file("b/more.clef", clef(&[("11:00:00", "Information", "WEB03", "new")]));
    ws.reload().unwrap();
    let s = ws.summary();
    assert_eq!(s.sources.len(), 2, "{:?}", s.sources.iter().map(|x| &x.name).collect::<Vec<_>>());

    let mine = Profile::from_toml("name = \"Mine\"\n[instance]\nkey = [\"@source\"]").unwrap();
    ws.set_profile(mine).unwrap();
    let s = ws.summary();
    assert_eq!(s.profile_name, "Mine");
    assert_eq!(s.default_group_by, vec!["@source".to_string()]);
    assert_eq!(s.sources.len(), 2);

    ws.clear();
    let s = ws.summary();
    assert_eq!((s.total, s.sources.len(), s.from), (0, 0, None));
    assert!(ws
        .set_profile(Profile::from_toml("[[extract]]\nname = 'x'\nregex = '('").unwrap())
        .is_err());
}

#[test]
fn single_source_with_constant_candidate_uses_it() {
    let d = TempDir::new();
    d.file(
        "one.clef",
        clef(&[("10:00:00", "Information", "WEB01", "a"), ("10:00:01", "Information", "WEB01", "b")]),
    );
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(&[d.path().to_path_buf()]).unwrap();
    assert_eq!(ws.summary().default_group_by, vec!["MachineName".to_string()]);

    let d = TempDir::new();
    d.file("x.log", "2026-10-14 10:00:00 INFO a\n");
    d.file("y.log", "2026-10-14 10:00:01 INFO b\n");
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(&[d.path().to_path_buf()]).unwrap();
    assert_eq!(ws.summary().default_group_by, vec!["@source".to_string()]);
}

#[test]
fn paging() {
    let d = folder();
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(&[d.path().to_path_buf()]).unwrap();

    let all = page(&ws, Query::default());
    assert_eq!(all.len(), 6);
    assert_eq!(all[0].1, "Application starting");

    let desc = ws
        .page(&PageRequest {
            query: Query::default(),
            offset: 0,
            limit: 2,
            descending: true,
            anchor: None,
        })
        .unwrap();
    assert_eq!(desc.total, 6);
    assert_eq!(desc.rows[0].message, "Host terminated unexpectedly");
    assert_eq!(desc.rows[0].markers[0], "fatal");
    assert_eq!(desc.rows[0].group, "WEB02");

    let anchored = ws
        .page(&PageRequest {
            query: Query::default(),
            offset: 0,
            limit: 2,
            descending: false,
            anchor: Some(5),
        })
        .unwrap();
    assert_eq!((anchored.offset, anchored.anchor_index), (4, Some(5)));

    let errors = Query {
        levels: vec![Level::Error],
        ..Default::default()
    };
    let filtered_out = ws
        .page(&PageRequest {
            query: errors,
            offset: 0,
            limit: 2,
            descending: false,
            anchor: Some(0),
        })
        .unwrap();
    assert_eq!(filtered_out.anchor_index, None);
    assert_eq!(filtered_out.rows.len(), 1);

    let beyond = ws
        .page(&PageRequest {
            query: Query::default(),
            offset: 100,
            limit: 10,
            descending: false,
            anchor: None,
        })
        .unwrap();
    assert!(beyond.rows.is_empty());

    // Request defaults.
    let req: PageRequest = serde_json::from_str("{}").unwrap();
    assert_eq!((req.limit, req.offset, req.descending), (500, 0, false));
    assert!(ws
        .page(&PageRequest {
            query: Query {
                text: "(".into(),
                regex: true,
                ..Default::default()
            },
            ..req
        })
        .is_err());
}

#[test]
fn timeline_ranges_groups_and_lifecycle() {
    let d = folder();
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(&[d.path().to_path_buf()]).unwrap();

    let req: TimelineRequest = serde_json::from_str("{}").unwrap();
    assert_eq!((req.buckets, req.always_show_lifecycle), (400, true));
    let tl = ws.timeline(&req).unwrap();
    assert_eq!(tl.lanes.len(), 2);
    assert_eq!(tl.truncated_lanes, 0);
    assert_eq!(tl.histogram.iter().map(|b| b.iter().sum::<u32>()).sum::<u32>(), 6);
    let web01 = &tl.lanes[0];
    assert_eq!(web01.key, "WEB01");
    let kinds: Vec<&str> = web01.markers.iter().map(|m| m.kind.as_str()).collect();
    assert_eq!(kinds, vec!["start", "warning", "error", "stop"]);

    // Explicit range and instance selection; lifecycle hidden on request.
    let q = Query {
        from: Some(1_791_972_000_000 + 1_000),
        to: Some(1_791_972_000_000 + 40_000),
        groups: vec!["WEB02".into()],
        levels: vec![Level::Fatal],
        ..Default::default()
    };
    let tl = ws
        .timeline(&TimelineRequest {
            query: q.clone(),
            buckets: 5,
            always_show_lifecycle: false,
        })
        .unwrap();
    assert_eq!(tl.histogram.len(), 10, "buckets are clamped to at least 10");
    assert_eq!(tl.lanes.len(), 1);
    let lane = &tl.lanes[0];
    assert_eq!(lane.markers.iter().map(|m| m.kind.as_str()).collect::<Vec<_>>(), vec!["fatal"]);
    assert_eq!((lane.total, lane.matched), (2, 1));
    let tl = ws
        .timeline(&TimelineRequest {
            query: q,
            buckets: 5,
            always_show_lifecycle: true,
        })
        .unwrap();
    assert_eq!(tl.lanes[0].markers.len(), 2);

    // Nothing with a timestamp in range.
    let empty = Query {
        from: Some(0),
        to: Some(1),
        ..Default::default()
    };
    let tl = ws
        .timeline(&TimelineRequest {
            query: empty,
            buckets: 10,
            always_show_lifecycle: true,
        })
        .unwrap();
    assert!(tl.lanes.is_empty());
}

#[test]
fn facets() {
    let d = folder();
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(&[d.path().to_path_buf()]).unwrap();
    let req: FacetRequest = serde_json::from_str(r#"{"field":"MachineName"}"#).unwrap();
    assert_eq!(req.limit, 50);
    let f = ws.facet(&req).unwrap();
    assert_eq!((f.distinct, f.total), (2, 6));
    assert_eq!(f.values[0].value, "WEB01");
    assert_eq!(f.values[0].count, 4);
    let g = ws
        .facet(&FacetRequest {
            query: Query::default(),
            field: "@group".into(),
            limit: 1,
        })
        .unwrap();
    assert_eq!(g.values.len(), 1);
    assert_eq!(g.distinct, 2);
    let none = ws
        .facet(&FacetRequest {
            query: Query::default(),
            field: "Nope".into(),
            limit: 5,
        })
        .unwrap();
    assert_eq!(none.total, 0);
}

#[test]
fn charts() {
    let d = TempDir::new();
    let mut lines = String::new();
    for i in 0..10 {
        for m in 0..10 {
            lines += &format!(r#"{{"@t":"2026-10-14T10:{i:02}:00Z","@mt":"x","M":"m{m}","V":{}}}"#, m * 10 + i);
            lines += "\n";
        }
        lines += &format!(r#"{{"@t":"2026-10-14T10:{i:02}:30Z","@l":"Warning","@mt":"w","M":"m0"}}"#);
        lines += "\n";
    }
    d.file("m.clef", lines);
    let profile = Profile::from_toml(
        "[[chart]]\nid='v'\ntitle='V'\ny='max(V)'\nbucket='1m'\nsplit_by=['M']\nwhen=[{field='V',op='exists'}]\n\
         [[chart]]\ntitle='Untitled id'\ny='count()'",
    )
    .unwrap();
    let mut ws = Workspace::new(profile).unwrap();
    ws.open(&[d.path().to_path_buf()]).unwrap();

    let req: ChartRequest = serde_json::from_str(r#"{"chartId":"v"}"#).unwrap();
    assert_eq!(req.buckets, 400);
    let c = ws.chart(&req).unwrap();
    assert_eq!(c.bucket_ms, 60_000);
    assert_eq!(c.series.len(), 8, "7 biggest + Other");
    assert_eq!(c.series.last().unwrap().key, "Other");
    let m9 = c.series.iter().find(|s| s.key == "m9");
    assert!(m9.is_none() || c.series.iter().any(|s| s.key == "Other"));
    assert_eq!(c.series[0].points.len(), 10);

    assert_eq!(ws.summary().charts[1].id, "chart-2");
    assert!(ws
        .chart(&ChartRequest {
            query: Query::default(),
            chart_id: Some("nope".into()),
            def: None,
            buckets: 100
        })
        .is_err());
    assert!(ws
        .chart(&ChartRequest {
            query: Query::default(),
            chart_id: None,
            def: None,
            buckets: 100
        })
        .is_err());

    let def = |y: &str, split: &[&str]| ChartDef {
        id: "adhoc".into(),
        title: "Ad hoc".into(),
        y: y.into(),
        bucket: "auto".into(),
        split_by: split.iter().map(|s| s.to_string()).collect(),
        kind: "line".into(),
        unit: String::new(),
    };
    let adhoc = |y: &str, split: &[&str]| {
        ws.chart(&ChartRequest {
            query: Query::default(),
            chart_id: None,
            def: Some(def(y, split)),
            buckets: 20,
        })
        .unwrap()
    };

    let by_level = adhoc("count()", &["@level"]);
    assert_eq!(
        by_level.series.iter().map(|s| s.key.as_str()).collect::<Vec<_>>(),
        vec!["Information", "Warning"]
    );
    let total = adhoc("count()", &[]);
    assert_eq!(total.series[0].key, "Ad hoc");
    assert_eq!(total.series[0].points.iter().filter_map(|p| p.1).sum::<f64>(), 110.0);
    // Records without the field are skipped; empty buckets are null for non-count aggregates.
    let avg = adhoc("avg(V)", &[]);
    assert!(avg.series[0].points.iter().any(|p| p.1.is_some()));
    assert!(
        avg.series[0].points.iter().any(|p| p.1.is_none()),
        "warning-only buckets have no average"
    );
    let missing = adhoc("sum(Nope)", &[]);
    assert!(missing.series.is_empty());
    assert!(ws
        .chart(&ChartRequest {
            query: Query::default(),
            chart_id: None,
            def: Some(def("bad", &[])),
            buckets: 20
        })
        .is_err());

    // Fixed tiny bucket over a long range is widened to cap the bucket count.
    let mut tiny = def("count()", &[]);
    tiny.bucket = "1ms".into();
    let c = ws
        .chart(&ChartRequest {
            query: Query::default(),
            chart_id: None,
            def: Some(tiny),
            buckets: 20,
        })
        .unwrap();
    assert!(c.series[0].points.len() <= 5001);

    // Ranged query.
    let q = Query {
        from: Some(1_791_972_000_000),
        to: Some(1_791_972_000_000 + 120_000),
        ..Default::default()
    };
    let c = ws
        .chart(&ChartRequest {
            query: q,
            chart_id: Some("v".into()),
            def: None,
            buckets: 20,
        })
        .unwrap();
    assert_eq!(c.from, 1_791_972_000_000);
}

#[test]
fn chart_lifecycle_events() {
    let d = folder();
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(&[d.path().to_path_buf()]).unwrap();
    let c = ws
        .chart(&ChartRequest {
            query: Query::default(),
            chart_id: Some("events-by-level".into()),
            def: None,
            buckets: 50,
        })
        .unwrap();
    let kinds: Vec<&str> = c.lifecycle.iter().map(|l| l.kind.as_str()).collect();
    assert_eq!(kinds, vec!["start", "start", "stop", "fatal"]);
    assert_eq!(c.lifecycle[3].group, "WEB02");
}

#[test]
fn extractor_preview() {
    let d = TempDir::new();
    d.file(
        "x.log",
        "2026-10-14 10:00:00 INFO CPU 12.5% mem 3 GB\n2026-10-14 10:00:01 INFO CPU high\n2026-10-14 10:00:02 WARN ignored\n2026-10-14 10:00:03 INFO CPU 1% mem 512 MB — ünïcödé\n",
    );
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(&[d.path().to_path_buf()]).unwrap();

    let toml = format!(
        "{DEFAULT_PROFILE_TOML}\n[[extract]]\nname='stats'\nregex='CPU (?P<cpu>[\\d.]+)% mem (?P<mem>\\d+) (?P<unit>MB|GB)'\nwhen=[{{field='@level',op='eq',value='Information'}}]\n[extract.fields]\ncpu={{type='number'}}\nmem={{type='bytes',unit_from='unit',to='MB'}}\n\
         [[extract]]\nname='from-missing'\nfrom='Nope'\nregex='x'\n"
    );
    let req: ExtractPreviewRequest = serde_json::from_str(&serde_json::json!({"toml": toml, "extractor": "stats"}).to_string()).unwrap();
    assert_eq!(req.limit, 20);
    let p = ws.preview_extractor(&req);
    assert_eq!(p.error, None);
    assert_eq!((p.candidates, p.matches, p.failed), (3, 2, 1));
    let s = &p.samples[0];
    assert_eq!(s.values["mem"], 3072.0);
    let cpu = s.spans.iter().find(|x| x.name == "cpu").unwrap();
    assert_eq!(&s.text[cpu.start..cpu.end], "12.5");
    assert_eq!(p.samples.len(), 2);

    let limited = ws.preview_extractor(&ExtractPreviewRequest { limit: 1, ..req.clone() });
    assert_eq!(limited.samples.len(), 1);
    let missing = ws.preview_extractor(&ExtractPreviewRequest {
        extractor: "from-missing".into(),
        ..req.clone()
    });
    assert_eq!((missing.candidates, missing.failed), (4, 4));
    let unknown = ws.preview_extractor(&ExtractPreviewRequest {
        extractor: "nope".into(),
        ..req.clone()
    });
    assert!(unknown.error.unwrap().contains("no [[extract]] named"));
    let bad = ws.preview_extractor(&ExtractPreviewRequest {
        toml: "[parse".into(),
        ..req.clone()
    });
    assert!(bad.error.unwrap().starts_with("line 1"));
    let invalid = ws.preview_extractor(&ExtractPreviewRequest {
        toml: "[[extract]]\nname='stats'\nregex='('".into(),
        ..req.clone()
    });
    assert!(invalid.error.is_some());
    let filtered = ws.preview_extractor(&ExtractPreviewRequest {
        query: Query {
            filters: vec![FieldFilter {
                field: "@message".into(),
                op: "contains".into(),
                value: Some("high".into()),
            }],
            ..Default::default()
        },
        ..req
    });
    assert_eq!((filtered.candidates, filtered.matches), (1, 0));
}

/// Reads Windows registry exports: one record per `[HKEY...]` key line.
struct RegistryImporter;

impl Importer for RegistryImporter {
    fn id(&self) -> &'static str {
        "registry"
    }

    fn sniff(&self, ctx: &ImportCtx, _head: &str) -> u8 {
        if ctx.path.extension().is_some_and(|e| e == "reg") {
            90
        } else {
            0
        }
    }

    fn import(&self, _ctx: &ImportCtx, content: &str) -> Result<Imported> {
        let mut out = Imported::default();
        for (i, line) in content.lines().enumerate() {
            if line.starts_with("[HKEY") {
                let mut r = Record::new(i as u32 + 1);
                r.message = line.trim().to_string();
                out.records.push(r);
            }
        }
        Ok(out)
    }
}

#[test]
fn custom_importers_can_be_registered() {
    let reg = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples/windows/registry-export.reg");
    let mut builtin = Workspace::new(Profile::default_profile()).unwrap();
    builtin.open(std::slice::from_ref(&reg)).unwrap();
    assert_eq!(builtin.summary().unrecognized.len(), 1);

    let mut importers = Importers::new();
    importers.register(Box::new(RegistryImporter));
    let mut ws = Workspace::with_importers(Profile::default_profile(), importers).unwrap();
    ws.open(&[reg]).unwrap();
    let s = ws.summary();
    assert!(s.unrecognized.is_empty());
    assert_eq!(s.sources.len(), 1);
    assert_eq!(s.sources[0].importer, "registry");
    assert_eq!(s.sources[0].name, "windows/registry-export.reg");
    assert_eq!(s.total, s.sources[0].records);
    assert!(page(&ws, Query::default()).iter().all(|(_, m)| m.starts_with("[HKEY")));
}

#[test]
fn workspace_works_through_the_log_store_trait() {
    let d = folder();
    let mut store: Box<dyn LogStore> = Box::new(Workspace::new(Profile::default_profile()).unwrap());
    store.open(&[d.path().to_path_buf()]).unwrap();
    assert_eq!(store.summary().total, 6);
    assert_eq!(store.profile().name, "Default (Serilog)");
    let p = store
        .page(&PageRequest {
            query: Query::default(),
            offset: 0,
            limit: 3,
            descending: false,
            anchor: None,
        })
        .unwrap();
    assert_eq!((p.total, p.rows.len()), (6, 3));
    let tl: TimelineRequest = serde_json::from_value(serde_json::json!({"buckets": 20})).unwrap();
    assert_eq!(store.timeline(&tl).unwrap().lanes.len(), 2);
    let facet: FacetRequest = serde_json::from_value(serde_json::json!({"field": "MachineName"})).unwrap();
    assert_eq!(store.facet(&facet).unwrap().distinct, 2);
    let chart: ChartRequest = serde_json::from_value(serde_json::json!({"chartId": "events-by-level"})).unwrap();
    assert!(!store.chart(&chart).unwrap().series.is_empty());
    let preview: ExtractPreviewRequest =
        serde_json::from_value(serde_json::json!({"toml": DEFAULT_PROFILE_TOML, "extractor": "x"})).unwrap();
    assert!(store.preview_extractor(&preview).error.is_some());

    let id = store.summary().sources[0].id;
    store.remove_source(id);
    assert_eq!(store.summary().sources.len(), 1);
    store.reload().unwrap();
    store.set_profile(Profile::from_toml("name = 'Other'").unwrap()).unwrap();
    assert_eq!(store.profile().name, "Other");
    store.clear();
    assert_eq!(store.summary().total, 0);
}

#[test]
fn moment_users_carry_the_field_they_were_read_from() {
    use imt_core::workspace::MomentRequest;
    let d = TempDir::new();
    let line = |t: &str, extra: &str| format!(r#"{{"@t":"2026-10-14T10:00:{t}Z","@mt":"did something"{extra}}}"#) + "\n";
    let text = [
        line("01", r#","UserName":"alice""#),
        line("02", r#","UserName":"alice""#),
        // UserName wins over ClientIp when both are present (first present field).
        line("03", r#","UserName":"bob","ClientIp":"10.0.0.9""#),
        line("04", r#","ClientIp":"10.0.0.7""#),
        line("05", ""),
    ]
    .concat();
    d.file("app.clef", text);
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(&[d.path().to_path_buf()]).unwrap();
    let from = chrono::DateTime::parse_from_rfc3339("2026-10-14T10:00:00Z")
        .unwrap()
        .timestamp_millis();
    let m = ws
        .moment(&MomentRequest {
            query: Query::default(),
            from,
            to: from + 10_000,
        })
        .unwrap();
    let users: Vec<(&str, &str, u64)> = m.users.iter().map(|u| (u.field.as_str(), u.value.as_str(), u.count)).collect();
    assert_eq!(
        users,
        vec![("UserName", "alice", 2), ("ClientIp", "10.0.0.7", 1), ("UserName", "bob", 1)]
    );
    let json = serde_json::to_value(&m.users[0]).unwrap();
    assert_eq!(json, serde_json::json!({"field": "UserName", "value": "alice", "count": 2}));
}

#[test]
fn removing_a_source_that_is_not_open_changes_nothing() {
    let dir = TempDir::new();
    dir.file("a.log", "2026-10-14 10:00:00 INFO one\n2026-10-14 10:00:01 INFO two\n");
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(&[dir.path().to_path_buf()]).unwrap();
    let (sources, events) = (ws.sources().len(), ws.records().len());
    assert_eq!(sources, 1);
    ws.remove_source(9_999);
    assert_eq!((ws.sources().len(), ws.records().len()), (sources, events));
}
