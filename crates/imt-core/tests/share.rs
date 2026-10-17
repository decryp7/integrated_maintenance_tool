//! Explain and share: the redacted view, redacted exports, dashboards,
//! reports and `.imtz` bundles, end to end through the store.

use imt_core::api::{ContextRequest, ExportRequest, FacetRequest, PageRequest, QueryRequest};
use imt_core::bundle::{self, BundleContent, BundleOptions};
use imt_core::dashboard::{TileCfg, TileData, TileRequest};
use imt_core::project::ProjectFile;
use imt_core::query::Query;
use imt_core::report::Report;
use imt_core::workspace::{RedactionPreviewRequest, RedactionSummaryRequest};
use imt_core::{LogStore, Profile, Workspace};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

struct Dir(PathBuf);
impl Dir {
    fn new(tag: &str) -> Dir {
        let p = std::env::temp_dir().join(format!("imt-share-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Dir(std::fs::canonicalize(p).unwrap())
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const PROFILE: &str = r#"
name = "Shared"
[redact]
detectors = ["email", "ipv4", "password"]
fields = ["@message", "@exception", "User"]
mode = "mask"
"#;

fn clef(lines: &[Value]) -> String {
    lines.iter().map(|l| format!("{l}\n")).collect()
}

/// A workspace over one CLEF file with personal data in it.
fn workspace(dir: &Path, profile: &str) -> Workspace {
    let text = clef(&[
        json!({"@t": "2026-10-14T10:00:00Z", "@mt": "Login by {User} from 10.0.0.1", "User": "alice@contoso.com", "@m": "Login by alice@contoso.com from 10.0.0.1"}),
        json!({"@t": "2026-10-14T10:00:01Z", "@l": "Error", "@m": "Order failed for bob@contoso.com password=hunter2", "User": "bob@contoso.com", "@x": "System.Exception: mail alice@contoso.com"}),
        json!({"@t": "2026-10-14T10:00:02Z", "@m": "Retry for alice@contoso.com", "User": "alice@contoso.com"}),
        json!({"@t": "2026-10-14T10:00:03Z", "@l": "Warning", "@m": "<script>alert('x')</script> nothing personal", "Other": "carol@contoso.com"}),
    ]);
    std::fs::create_dir_all(dir.join("logs")).unwrap();
    std::fs::write(dir.join("logs/app.clef"), text).unwrap();
    let mut ws = Workspace::new(Profile::from_toml(profile).unwrap()).unwrap();
    ws.open(&[dir.join("logs")]).unwrap();
    assert_eq!(ws.summary().total, 4);
    ws
}

fn page(ws: &dyn LogStore) -> Value {
    serde_json::to_value(
        ws.page(&PageRequest {
            query: Query::default(),
            offset: 0,
            limit: 10,
            descending: false,
            anchor: None,
        })
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn the_redacted_view_masks_pages_facets_and_context() {
    let d = Dir::new("view");
    let mut ws = workspace(&d.0, PROFILE);
    let store: &mut dyn LogStore = &mut ws;
    assert_eq!(page(&*store)["rows"][0]["message"], "Login by alice@contoso.com from 10.0.0.1");

    store.set_view_redaction(true).unwrap();
    assert!(store.summary().redacted_view);
    let p = page(&*store);
    assert_eq!(p["rows"][0]["message"], "Login by <email> from <ipv4>");
    assert_eq!(p["rows"][1]["message"], "Order failed for <email> password=<password>");
    assert_eq!(p["rows"][1]["exception"], "System.Exception: mail <email>");
    assert_eq!(p["rows"][0]["fields"]["User"], "<email>");
    // Out of scope: not redacted in the grid.
    assert_eq!(p["rows"][3]["fields"]["Other"], "carol@contoso.com");
    assert_eq!(p["rows"][0]["template"], "Login by {User} from <ipv4>");

    // Facet values that mask alike are counted together.
    let facet = store
        .facet(&FacetRequest {
            query: Query::default(),
            field: "User".into(),
            limit: 10,
        })
        .unwrap();
    assert_eq!(
        (facet.values.len(), facet.values[0].value.as_str(), facet.values[0].count),
        (1, "<email>", 3)
    );

    let ctx = store
        .source_context(&ContextRequest {
            record_id: 1,
            before: 1,
            after: 1,
        })
        .unwrap();
    assert!(
        ctx.lines
            .iter()
            .all(|l| !l.text.contains("contoso.com") && !l.text.contains("hunter2")),
        "{:?}",
        ctx.lines
    );
    assert!(ctx.lines.iter().any(|l| l.text.contains("<email>")));

    // A profile change keeps the view redacted with the new rules...
    store
        .set_profile(Profile::from_toml("name='x'\n[redact]\ndetectors=['ipv4']").unwrap())
        .unwrap();
    assert_eq!(page(&*store)["rows"][0]["message"], "Login by alice@contoso.com from <ipv4>");
    // A restored project's profile applies too.
    let state = store.state();
    store
        .restore(Profile::from_toml("name='y'\n[redact]\ndetectors=['email']").unwrap(), &state)
        .unwrap();
    assert_eq!(page(&*store)["rows"][0]["message"], "Login by <email> from 10.0.0.1");
    // ... and switches it off when the new profile has none.
    store
        .set_profile(Profile::from_toml("name='x'\n[redact]\ndetectors=[]").unwrap())
        .unwrap();
    assert!(!store.summary().redacted_view);
    let e = store.set_view_redaction(true).unwrap_err();
    assert!(e.to_string().contains("no redaction rules"), "{e}");
    store.set_view_redaction(false).unwrap();
    assert_eq!(page(&*store)["rows"][0]["message"], "Login by alice@contoso.com from 10.0.0.1");
}

#[test]
fn exports_are_redacted_with_a_fresh_salt_each_time() {
    let d = Dir::new("export");
    let profile = PROFILE.replace("mode = \"mask\"", "mode = \"hash\"");
    let ws = workspace(&d.0, &profile);
    let export = |name: &str, format: &str, redact: bool| {
        let path = d.0.join(name);
        let res = ws
            .export(&ExportRequest {
                query: Query::default(),
                format: format.into(),
                path: path.to_string_lossy().into_owned(),
                descending: false,
                redact,
                overwrite: false,
            })
            .unwrap();
        (std::fs::read_to_string(path).unwrap(), res)
    };
    let (a, res) = export("a.clef", "clef", true);
    assert!(
        !a.contains("alice@") && !a.contains("bob@") && !a.contains("hunter2") && !a.contains("10.0.0.1"),
        "{a}"
    );
    // Out of scope: kept.
    assert!(a.contains("carol@contoso.com"));
    assert_eq!(res.redactions["email"], 7);
    // Equal values hash alike within the export (alice is in rows 1 and 3)...
    let lines: Vec<Value> = a.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let alice = lines[0]["User"].as_str().unwrap().to_string();
    assert!(alice.starts_with("<email:"));
    assert_eq!(lines[2]["User"], alice);
    assert_ne!(lines[1]["User"], alice);
    // ...and differently in another export.
    let (b, _) = export("b.clef", "clef", true);
    let other: Value = serde_json::from_str(b.lines().next().unwrap()).unwrap();
    assert_ne!(other["User"], alice);

    let (csv, _) = export("c.csv", "csv", true);
    assert!(!csv.contains("alice@") && !csv.contains("bob@"));
    let (raw, res) = export("d.csv", "csv", false);
    assert!(raw.contains("alice@contoso.com") && res.redactions.is_empty());
}

#[test]
fn previews_and_summaries() {
    let d = Dir::new("preview");
    let ws = workspace(&d.0, PROFILE);
    let p = ws.preview_redaction(&RedactionPreviewRequest {
        sample: "a@b.io 1.2.3.4".into(),
        toml: None,
    });
    assert_eq!(p.after, "<email> <ipv4>");
    let p = ws.preview_redaction(&RedactionPreviewRequest {
        sample: "a@b.io".into(),
        toml: Some("[redact]\ndetectors = ['email']\nmode = 'drop'".into()),
    });
    assert_eq!(p.after, "");
    let p = ws.preview_redaction(&RedactionPreviewRequest {
        sample: "x".into(),
        toml: Some("[redact".into()),
    });
    assert!(p.error.is_some() && p.after == "x");

    let s = ws
        .redaction_summary(&RedactionSummaryRequest {
            query: Query::default(),
            samples: 2,
        })
        .unwrap();
    assert_eq!((s.scanned, s.changed, s.truncated), (4, 3, false));
    assert_eq!(s.samples.len(), 2);
    assert_eq!(s.samples[0].after, "Login by <email> from <ipv4>");
    assert_eq!(
        s.hits.iter().map(|h| h.rule.as_str()).collect::<Vec<_>>(),
        ["password", "email", "ipv4"]
    );
}

fn sample_workspace() -> Workspace {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples");
    let text = std::fs::read_to_string(root.join("profiles/orderservice.imtprofile.toml")).unwrap();
    let mut ws = Workspace::new(Profile::from_toml(&text).unwrap()).unwrap();
    ws.open(&[root.join("serilog-clef"), root.join("serilog-text")]).unwrap();
    ws
}

#[test]
fn every_tile_of_the_sample_dashboard_evaluates() {
    let ws = sample_workspace();
    let dashboards = ws.summary().dashboards;
    let health = dashboards.iter().find(|d| d.id == "orderservice-health").expect("sample dashboard");
    assert_eq!(health.title, "OrderService health");
    for tile in &health.tile {
        let data = ws
            .dashboard_tile(&TileRequest {
                tile: tile.clone(),
                query: Query::default(),
            })
            .unwrap_or_else(|e| panic!("tile {}: {e:#}", tile.id));
        match (tile.id.as_str(), data) {
            (
                "errors",
                TileData::Stat {
                    value, label, sparkline, ..
                },
            ) => {
                assert!(value.unwrap() > 0.0);
                assert_eq!(label, "errors");
                assert_eq!(sparkline.len(), imt_core::dashboard::SPARKLINE_POINTS);
                let sum: f64 = sparkline.iter().filter_map(|p| p.1).sum();
                assert_eq!(sum, value.unwrap(), "the sparkline adds up to the total");
            }
            ("p95-latency", TileData::Stat { value, unit, .. }) => assert!(value.unwrap() > 0.0 && unit.as_deref() == Some("ms")),
            ("problems" | "cpu" | "memory", TileData::Chart { data }) => assert!(!data.series.is_empty(), "{}", tile.id),
            ("noisy-sources", TileData::Top { facet }) => assert!(!facet.values.is_empty()),
            ("slow-endpoints", TileData::Table { columns, rows, .. }) => {
                assert_eq!(columns, ["RequestPath", "requests", "p95"]);
                assert!(!rows.is_empty() && rows.len() <= 10);
            }
            ("alerts", TileData::Alerts { .. }) | ("warnings" | "backlog", TileData::Stat { .. }) => {}
            (id, other) => panic!("unexpected data for {id}: {other:?}"),
        }
    }
}

fn tile(toml: &str) -> TileCfg {
    toml::from_str(toml).unwrap()
}

#[test]
fn tiles_of_every_kind_and_their_errors() {
    let ws = sample_workspace();
    let eval = |t: &str, q: Query| ws.dashboard_tile(&TileRequest { tile: tile(t), query: q });
    let inline = eval(
        "id='c'\ntype='chart'\nvalue='count()'\nby=['@level']\nstyle='bar'\nquery='level >= warning'",
        Query::default(),
    )
    .unwrap();
    let TileData::Chart { data } = inline else { panic!() };
    assert_eq!((data.def.kind.as_str(), data.def.title.as_str()), ("bar", "c"));
    let titled = eval("id='c'\ntitle='Problems'\ntype='chart'\nvalue='count()'", Query::default()).unwrap();
    let TileData::Chart { data: titled } = titled else { panic!() };
    assert_eq!(titled.def.title, "Problems");
    assert!(data
        .series
        .iter()
        .all(|s| s.key == "Warning" || s.key == "Error" || s.key == "Fatal"));

    let events = eval("id='t'\ntype='table'\nquery='level = error'\nlimit=3", Query::default()).unwrap();
    let TileData::Table { columns, rows, truncated } = events else {
        panic!()
    };
    assert_eq!((columns.len(), rows.len(), truncated), (3, 3, true));

    // The tile's own window, relative to the newest event.
    let recent = eval("id='s'\ntype='stat'\nquery='| stats count()'\nwindow='1m'", Query::default()).unwrap();
    let all = eval("id='s'\ntype='stat'\nquery='| stats count()'", Query::default()).unwrap();
    let (TileData::Stat { value: Some(recent), .. }, TileData::Stat { value: Some(all), .. }) = (recent, all) else {
        panic!()
    };
    assert!(recent < all);
    // The base query of the window applies too.
    let base = Query {
        ql: "level = error".into(),
        ..Default::default()
    };
    let TileData::Stat { value: Some(errors), .. } = eval("id='s'\ntype='stat'\nquery='| stats count()'", base).unwrap() else {
        panic!()
    };
    assert!(errors < all);
    let empty = Query {
        ql: "nosuchword_xyz".into(),
        ..Default::default()
    };
    let TileData::Stat { value, .. } = eval("id='s'\ntype='stat'\nquery='| stats count()'", empty).unwrap() else {
        panic!()
    };
    assert!(value.is_none_or(|v| v == 0.0));

    assert!(matches!(
        eval("id='a'\ntype='annotations'", Query::default()).unwrap(),
        TileData::Annotations { .. }
    ));
    assert!(eval("id='x'\ntype='stat'\nquery='level = error'", Query::default())
        .unwrap_err()
        .to_string()
        .contains("stats"));
    assert!(eval("id='x'\ntype='chart'\nchart='nope'", Query::default())
        .unwrap_err()
        .to_string()
        .contains("unknown chart"));
    assert!(eval("id='x'\ntype='table'\nquery='| sort'", Query::default()).is_err());

    // Through the store abstraction; a profile with a broken dashboard is refused.
    let store: &dyn LogStore = &ws;
    assert!(store
        .dashboard_tile(&TileRequest {
            tile: tile("id='a'\ntype='annotations'"),
            query: Query::default()
        })
        .is_ok());
    let bad = Profile::from_toml("[[dashboard]]\nid='d'\ntitle='D'\n[[dashboard.tile]]\nid='t'\ntype='stat'").unwrap();
    assert!(format!("{:#}", Workspace::new(bad).err().unwrap()).contains("needs a query"));
}

fn report(ids: &[u32]) -> Report {
    Report::parse(json!({
        "id": "r1",
        "title": "Outage for alice@contoso.com",
        "summary": "Seen from 10.0.0.1. <script>alert(1)</script>",
        "sections": [{
            "id": "s1", "title": "Events",
            "blocks": [
                {"id": "b1", "type": "events", "caption": "Bookmarks", "ids": ids},
                {"id": "b2", "type": "chart", "title": "", "spec": {"id": "c", "title": "Errors by user", "y": "count()"},
                 "svg": "<svg xmlns=\"http://www.w3.org/2000/svg\"><text>alice@contoso.com</text><script>x</script></svg>"},
                {"id": "b3", "type": "timeline", "svg": "<svg><text>10.0.0.1</text></svg>"},
                {"id": "b4", "type": "table", "columns": ["User"], "rows": [["bob@contoso.com"], [3], [null]]},
                {"id": "b5", "type": "finding", "severity": "warning", "title": "t", "detail": "carol@contoso.com"},
                {"id": "b6", "type": "annotations", "items": [{"from": 1, "label": "reboot of 10.0.0.2"}]},
                {"id": "b7", "type": "text", "markdown": "mail **alice@contoso.com**"},
                {"id": "b8", "type": "future", "x": 1}
            ]
        }]
    }))
    .unwrap()
}

#[test]
fn reports_resolve_events_and_redact_every_text() {
    let d = Dir::new("report");
    let ws = workspace(&d.0, PROFILE);
    let r = ws.resolve_report(&report(&[1, 3, 999]), true).unwrap();
    let json = serde_json::to_string(&r).unwrap();
    assert!(
        !json.contains("contoso.com") && !json.contains("10.0.0.") && !json.contains("hunter2"),
        "{json}"
    );
    assert!(r.redacted && r.redaction.iter().any(|h| h.rule == "email" && h.count > 0));
    let v = serde_json::to_value(&r).unwrap();
    let blocks = &v["sections"][0]["blocks"];
    assert_eq!(blocks[0]["events"].as_array().unwrap().len(), 2);
    assert_eq!(blocks[0]["missing"], 1);
    assert_eq!(blocks[0]["events"][0]["message"], "Order failed for <email> password=<password>");
    assert_eq!(blocks[1]["title"], "Errors by user");
    assert!(!blocks[1]["svg"].as_str().unwrap().contains("script"));
    assert_eq!(blocks[2]["title"], "Timeline");
    assert_eq!(blocks[3]["rows"], json!([["<email>"], ["3"], [""]]));
    assert_eq!(blocks[7], json!({"type": "unsupported", "kind": "future"}));

    // HTML: log text containing markup stays text.
    let html = imt_core::report::html::render(&r, imt_core::report::Theme::Light);
    assert!(!html.contains("<script"));
    assert!(
        html.contains("&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt; nothing personal")
            || html.contains("&lt;script&gt;alert(1)&lt;/script&gt;")
    );

    let plain = ws.resolve_report(&report(&[0]), false).unwrap();
    assert!(!plain.redacted && plain.redaction.is_empty());
    assert!(serde_json::to_string(&plain).unwrap().contains("alice@contoso.com"));
}

#[test]
fn bundles_round_trip_raw_files_and_redacted_events() {
    let d = Dir::new("bundle");
    let ws = workspace(&d.0, PROFILE);
    let r = serde_json::to_value(report(&[0])).unwrap();

    // Raw files, not redacted: the same events after opening.
    let opts = BundleOptions {
        name: "Case 42".into(),
        content: BundleContent::Raw,
        query: Query::default(),
        redact: false,
        ui: json!({"bookmarks": [1]}),
        reports: vec![r.clone()],
    };
    let est = ws.estimate_bundle(&opts, PROFILE).unwrap();
    assert_eq!((est.files, est.events), (5, 4));
    assert!(est.bytes > 0 && est.skipped.is_empty());
    let path = d.0.join("case.imtz");
    let m = ws.write_bundle(&opts, PROFILE, &path).unwrap();
    assert!(!d.0.join("case.imtz.part").exists());
    assert_eq!(m.format, bundle::BUNDLE_FORMAT);
    let paths: Vec<&str> = m.entries.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "logs/0/app.clef",
            "profile.toml",
            "dashboards.json",
            "reports/r1.json",
            "project.imtproj"
        ]
    );

    let x = bundle::extract(&path, &d.0.join("opened")).unwrap();
    assert_eq!(x.manifest.as_ref().unwrap().name, "Case 42");
    let project = ProjectFile::load(x.project.as_ref().unwrap()).unwrap();
    assert_eq!(project.name, "Case 42");
    assert_eq!(project.ui["bookmarks"][0], 1);
    assert_eq!(project.reports.len(), 1);
    let mut opened = Workspace::new(Profile::from_toml(&project.profile).unwrap()).unwrap();
    let missing = opened
        .restore(
            Profile::from_toml(&project.profile).unwrap(),
            &imt_core::api::WorkspaceState {
                roots: project.sources.clone(),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(missing.is_empty());
    assert_eq!(
        page(&opened)["rows"],
        page(&ws)["rows"]
            .clone()
            .as_array()
            .map(|rows| {
                // Same events (source paths differ: the bundle's folder).
                let mut rows = rows.clone();
                for (a, b) in rows.iter_mut().zip(page(&opened)["rows"].as_array().unwrap()) {
                    a["sourceName"] = b["sourceName"].clone();
                    a["group"] = b["group"].clone();
                }
                Value::Array(rows)
            })
            .unwrap()
    );

    // Filtered events, redacted.
    let opts = BundleOptions {
        content: BundleContent::Events,
        query: Query {
            ql: "level >= warning".into(),
            ..Default::default()
        },
        redact: true,
        reports: vec![],
        ..opts
    };
    let est = ws.estimate_bundle(&opts, PROFILE).unwrap();
    assert_eq!((est.events, est.files), (2, 4));
    let path = d.0.join("filtered.imtz");
    let m = ws.write_bundle(&opts, PROFILE, &path).unwrap();
    assert_eq!(m.events, Some(2));
    assert!(m.redacted && m.redaction["email"] > 0);
    let x = bundle::extract(&path, &d.0.join("opened")).unwrap();
    let events = std::fs::read_to_string(x.dir.join("events/events.clef")).unwrap();
    assert!(!events.contains("bob@") && !events.contains("hunter2"), "{events}");
    let project = ProjectFile::load(x.project.as_ref().unwrap()).unwrap();
    let mut opened = Workspace::new(Profile::from_toml(&project.profile).unwrap()).unwrap();
    opened.open(&project.sources.iter().map(PathBuf::from).collect::<Vec<_>>()).unwrap();
    assert_eq!(opened.summary().total, 2);

    // Raw + redaction: text files are redacted, binary files left out.
    std::fs::write(d.0.join("logs/blob.bin"), [0u8, 1, 2, 3, 0, 0, 0, 0, 255, 254]).unwrap();
    let raw = BundleOptions {
        content: BundleContent::Raw,
        ..opts
    };
    let path = d.0.join("raw-redacted.imtz");
    let m = ws.write_bundle(&raw, PROFILE, &path).unwrap();
    let x = bundle::extract(&path, &d.0.join("opened")).unwrap();
    let log = std::fs::read_to_string(x.dir.join("logs/0/app.clef")).unwrap();
    assert!(!log.contains("contoso.com") && log.contains("<email>"));
    assert!(m.redacted);

    // A report that does not parse fails fast; nothing is left behind.
    let bad = BundleOptions {
        reports: vec![json!({"title": "no id"})],
        ..raw
    };
    let path = d.0.join("bad.imtz");
    assert!(format!("{:#}", ws.write_bundle(&bad, PROFILE, &path).unwrap_err()).contains("cannot be bundled"));
    assert!(!path.exists());
    let nowhere = d.0.join("no/such/dir/x.imtz");
    let ok = BundleOptions { reports: vec![], ..bad };
    assert!(ws.write_bundle(&ok, PROFILE, &nowhere).is_err());
}

#[test]
fn bundles_skip_missing_and_unredactable_sources() {
    let d = Dir::new("bundle-skip");
    let ws = workspace(&d.0, PROFILE);
    let gz = {
        use std::io::Write;
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(b"{\"@t\":\"2026-10-14T10:00:00Z\",\"@m\":\"x\"}\n").unwrap();
        e.finish().unwrap()
    };
    let mut ws2 = Workspace::new(Profile::from_toml(PROFILE).unwrap()).unwrap();
    std::fs::create_dir_all(d.0.join("gz")).unwrap();
    std::fs::write(d.0.join("gz/a.clef.gz"), gz).unwrap();
    std::fs::write(d.0.join("gz/b.clef"), "{\"@t\":\"2026-10-14T10:00:00Z\",\"@m\":\"y\"}\n").unwrap();
    ws2.open(&[d.0.join("gz")]).unwrap();
    std::fs::remove_file(d.0.join("gz/b.clef")).unwrap();
    let opts = BundleOptions {
        name: "x".into(),
        content: BundleContent::Raw,
        query: Query::default(),
        redact: true,
        ui: Value::Null,
        reports: vec![],
    };
    let est = ws2.estimate_bundle(&opts, "").unwrap();
    assert_eq!(est.skipped.len(), 1);
    assert!(est.skipped[0].reason.contains("no longer exists"));
    let m = ws2.write_bundle(&opts, "", &d.0.join("x.imtz")).unwrap();
    let reasons: Vec<&str> = m.skipped.iter().map(|s| s.reason.as_str()).collect();
    assert!(reasons.iter().any(|r| r.contains("archives cannot be redacted")), "{reasons:?}");
    assert!(m.entries.iter().all(|e| !e.path.starts_with("logs/")));
    drop(ws);
}

#[test]
fn queries_see_the_dashboard_and_redaction_in_the_summary() {
    let d = Dir::new("summary");
    let ws = workspace(&d.0, PROFILE);
    let s = serde_json::to_value(ws.summary()).unwrap();
    assert_eq!(s["redactedView"], false);
    assert_eq!(s["dashboards"], json!([]));
    let r = ws.run_query(&QueryRequest { query: Query::default() });
    assert!(r.is_ok());
}

#[test]
fn everything_works_through_the_store_trait() {
    let d = Dir::new("trait");
    let ws = workspace(&d.0, PROFILE);
    let store: &dyn LogStore = &ws;
    let s = store
        .redaction_summary(&RedactionSummaryRequest {
            query: Query::default(),
            samples: 1,
        })
        .unwrap();
    assert_eq!(s.changed, 3);
    let r = store.resolve_report(&report(&[0]), false).unwrap();
    assert_eq!(r.title, "Outage for alice@contoso.com");
    let opts = BundleOptions {
        name: "t".into(),
        content: BundleContent::Events,
        query: Query::default(),
        redact: false,
        ui: Value::Null,
        reports: vec![],
    };
    assert_eq!(store.estimate_bundle(&opts, PROFILE).unwrap().events, 4);
    // Unredacted events.
    let m = store.write_bundle(&opts, PROFILE, &d.0.join("t.imtz")).unwrap();
    assert!(!m.redacted && m.events == Some(4));
    let x = bundle::extract(&d.0.join("t.imtz"), &d.0.join("o")).unwrap();
    assert!(std::fs::read_to_string(x.dir.join("events/events.clef"))
        .unwrap()
        .contains("alice@contoso.com"));
    // A failed write leaves no half-written bundle behind (the target is a folder).
    std::fs::create_dir_all(d.0.join("taken.imtz")).unwrap();
    assert!(store.write_bundle(&opts, PROFILE, &d.0.join("taken.imtz")).is_err());
    assert!(!d.0.join("taken.imtz.part").exists());
    let tile: TileCfg = toml::from_str("id='s'\ntype='stat'\nquery='| stats count()'").unwrap();
    let ranged = Query {
        from: Some(1_791_972_000_000),
        to: Some(1_791_972_001_500),
        ..Default::default()
    };
    let TileData::Stat { value, sparkline, .. } = store.dashboard_tile(&TileRequest { tile, query: ranged }).unwrap() else {
        panic!()
    };
    assert_eq!(value, Some(2.0));
    assert_eq!(sparkline.iter().filter_map(|p| p.1).sum::<f64>(), 2.0);
}

#[test]
fn raw_bundles_copy_an_archive_once() {
    use std::io::Write;
    let d = Dir::new("archive");
    let mut z = zip::ZipWriter::new(std::fs::File::create(d.0.join("logs.zip")).unwrap());
    for name in ["a.clef", "b.clef"] {
        z.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(b"{\"@t\":\"2026-10-14T10:00:00Z\",\"@m\":\"x\"}\n").unwrap();
    }
    z.finish().unwrap();
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(&[d.0.join("logs.zip")]).unwrap();
    assert_eq!(ws.summary().sources.len(), 2);
    let opts = BundleOptions {
        name: "z".into(),
        content: BundleContent::Raw,
        query: Query::default(),
        redact: false,
        ui: Value::Null,
        reports: vec![],
    };
    let m = ws.write_bundle(&opts, "", &d.0.join("z.imtz")).unwrap();
    let logs: Vec<&str> = m
        .entries
        .iter()
        .filter(|e| e.path.starts_with("logs/"))
        .map(|e| e.path.as_str())
        .collect();
    assert_eq!(logs, ["logs/0/logs.zip"]);
}
