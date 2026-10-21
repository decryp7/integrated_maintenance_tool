//! Differential tests: the DuckDB store answers like the in-memory `Workspace` on `/samples` (page,
//! timeline and facet: counts and order). Rows are matched by source path and line; untimed events are
//! left out where the two differ by design (text before a file's first event is an untimed event in
//! the new import, skipped lines in the old one).

mod common;

use common::{pair, ranged, sample_profile, store_with, Pair, TempDir};
use imt_core::api::{FacetRequest, PageRequest, TimelineRequest};
use imt_core::model::Level;
use imt_core::ports::{Diagnostics, Filter, Ingest, PageQuery, Profiles, Query, StoreError};
use imt_core::query::FieldFilter;
use imt_core::Profile;
use imt_store_duck::DuckStore;
use std::collections::{BTreeMap, HashMap};
use std::ops::ControlFlow;

/// Queries covering the SQL path (time, levels, text, sources, markers) and the scan fallback (regex,
/// field filters, instance selection, query language).
fn queries(p: &Pair) -> Vec<(&'static str, Filter)> {
    let mid = (p.range.0 + p.range.1) / 2;
    let first_source = *p.ws_paths.keys().min().unwrap();
    let marker = p.ws.summary().marker_kinds.first().map(|m| m.kind.clone());
    let mut qs = vec![
        ("all", Filter::default()),
        (
            "levels",
            Filter {
                levels: vec![Level::Warning, Level::Error, Level::Fatal],
                ..Filter::default()
            },
        ),
        (
            "text",
            Filter {
                text: "timeout".into(),
                ..Filter::default()
            },
        ),
        (
            "smart search: words, the last a prefix",
            Filter {
                text: "timeout req".into(),
                text_mode: imt_core::query::TextMode::Words,
                ..Filter::default()
            },
        ),
        (
            "smart search: words or substrings",
            Filter {
                text: "imeout".into(),
                text_mode: imt_core::query::TextMode::WordsOrSubstring,
                ..Filter::default()
            },
        ),
        (
            "smart search: a phrase",
            Filter {
                text: "\"timed out\" ".into(),
                text_mode: imt_core::query::TextMode::Words,
                levels: vec![Level::Warning, Level::Error],
                ..Filter::default()
            },
        ),
        (
            "text, case-sensitive",
            Filter {
                text: "Order".into(),
                case_sensitive: true,
                ..Filter::default()
            },
        ),
        (
            "time window",
            Filter {
                from: Some(mid - 600_000),
                to: Some(mid + 600_000),
                ..Filter::default()
            },
        ),
        (
            "regex (scan)",
            Filter {
                text: r"time[ds]? ?out".into(),
                regex: true,
                ..Filter::default()
            },
        ),
        (
            "field filter (scan)",
            Filter {
                filters: vec![FieldFilter {
                    field: "MachineName".into(),
                    op: "exists".into(),
                    value: None,
                }],
                ..Filter::default()
            },
        ),
        (
            "query language (scan)",
            Filter {
                ql: "level >= warning".into(),
                ..Filter::default()
            },
        ),
    ];
    // Source ids may differ between the stores: the source filter is built per store below.
    let _ = first_source;
    if let Some(m) = marker {
        qs.push((
            "markers",
            Filter {
                markers: vec![m],
                ..Filter::default()
            },
        ));
    }
    qs
}

/// Rows as (path, line, ts, level, message), the identity both stores share.
type Row = (String, u32, Option<i64>, Level, String);

fn ws_rows(p: &Pair, q: &Filter, offset: usize, limit: usize, descending: bool) -> (usize, Vec<Row>) {
    let page =
        p.ws.page(&PageRequest {
            query: q.clone(),
            offset,
            limit,
            descending,
            anchor: None,
        })
        .unwrap();
    let rows = page
        .rows
        .iter()
        .map(|r| (p.ws_paths[&r.source].clone(), r.line, r.ts, r.level, r.message.to_string()))
        .collect();
    (page.total, rows)
}

fn duck_rows(p: &Pair, q: &Filter, offset: usize, limit: usize, descending: bool) -> (usize, Vec<Row>) {
    let page = p
        .duck
        .page(&PageQuery {
            query: q.clone(),
            offset,
            limit,
            descending,
            anchor: None,
        })
        .unwrap();
    let rows = page
        .rows
        .iter()
        .map(|r| (p.duck_paths[&r.source.0].clone(), r.line, r.ts, r.level, r.message.clone()))
        .collect();
    (page.total, rows)
}

fn check_pages(p: &Pair) {
    for (name, q) in queries(p) {
        let q = ranged(p, q);
        for (offset, desc) in [(0, false), (0, true), (1000, false), (7777, true)] {
            let (wt, wr) = ws_rows(p, &q, offset, 300, desc);
            let (dt, dr) = duck_rows(p, &q, offset, 300, desc);
            assert_eq!(dt, wt, "{name}: total");
            assert!(wt > 0 && (offset < wt || dr.is_empty()), "{name} matches events");
            assert_eq!(dr, wr, "{name}: page at {offset} (descending: {desc})");
        }
    }
    // Without a range, untimed events come last; the timed ones before them match.
    let (wt, wr) = ws_rows(p, &Filter::default(), 0, 500, false);
    let (dt, dr) = duck_rows(p, &Filter::default(), 0, 500, false);
    assert!(dt >= wt, "the new import adds untimed events, never loses any ({dt} < {wt})");
    assert_eq!(dr, wr);
}

fn check_sources_filter(p: &Pair) {
    let path = p.ws_paths.values().min().unwrap().clone();
    let ws_id = *p.ws_paths.iter().find(|(_, v)| **v == path).unwrap().0;
    let duck_id = *p.duck_paths.iter().find(|(_, v)| **v == path).unwrap().0;
    let ws_q = ranged(
        p,
        Filter {
            sources: vec![ws_id],
            ..Filter::default()
        },
    );
    let duck_q = Filter {
        sources: vec![duck_id],
        ..ws_q.clone()
    };
    let (wt, wr) = ws_rows(p, &ws_q, 0, 200, false);
    let (dt, dr) = duck_rows(p, &duck_q, 0, 200, false);
    assert_eq!((dt, dr), (wt, wr), "source filter");
}

fn check_anchor(p: &Pair) {
    let q = ranged(
        p,
        Filter {
            levels: vec![Level::Error],
            ..Filter::default()
        },
    );
    for desc in [false, true] {
        let ws_all =
            p.ws.page(&PageRequest {
                query: q.clone(),
                offset: 0,
                limit: 5000,
                descending: desc,
                anchor: None,
            })
            .unwrap();
        let target = &ws_all.rows[ws_all.rows.len() * 2 / 3];
        let key = (p.ws_paths[&target.source].clone(), target.line);
        let ws =
            p.ws.page(&PageRequest {
                query: q.clone(),
                offset: 0,
                limit: 100,
                descending: desc,
                anchor: Some(target.id),
            })
            .unwrap();
        // The same event's id in the DuckDB store.
        let mut duck_id = None;
        p.duck
            .scan(&q, &mut |b| {
                for r in b.rows() {
                    if (p.duck_paths[&r.source().0].clone(), r.line()) == key {
                        duck_id = Some(r.id());
                        return ControlFlow::Break(());
                    }
                }
                ControlFlow::Continue(())
            })
            .unwrap();
        let duck = p
            .duck
            .page(&PageQuery {
                query: q.clone(),
                offset: 0,
                limit: 100,
                descending: desc,
                anchor: duck_id,
            })
            .unwrap();
        assert_eq!(
            (duck.anchor_index, duck.offset),
            (ws.anchor_index, ws.offset),
            "anchor (descending: {desc})"
        );
        assert!(duck.anchor_index.is_some());
        let row = &duck.rows[duck.anchor_index.unwrap() - duck.offset];
        assert_eq!((p.duck_paths[&row.source.0].clone(), row.line), key);
    }
}

fn check_timelines(p: &Pair) {
    for (name, q) in queries(p) {
        for buckets in [10, 400] {
            let req = TimelineRequest {
                query: q.clone(),
                buckets,
                always_show_lifecycle: true,
            };
            let w = p.ws.timeline(&req).unwrap();
            let d = p.duck.timeline(&req).unwrap();
            assert_eq!((d.from, d.to), (w.from, w.to), "{name}: bounds");
            assert_eq!(d.bucket_ms, w.bucket_ms, "{name}");
            assert_eq!(d.histogram, w.histogram, "{name}: histogram");
            assert_eq!(d.truncated_lanes, w.truncated_lanes, "{name}");
            assert_eq!(d.lanes.len(), w.lanes.len(), "{name}: lanes");
            for (dl, wl) in d.lanes.iter().zip(&w.lanes) {
                assert_eq!(
                    (&dl.key, dl.first, dl.last, dl.total, dl.matched),
                    (&wl.key, wl.first, wl.last, wl.total, wl.matched),
                    "{name}: lane"
                );
                assert_eq!(dl.density, wl.density, "{name}: density of {}", wl.key);
                let paths = |ids: &[u32], m: &HashMap<u32, String>| ids.iter().map(|i| m[i].clone()).collect::<Vec<_>>();
                let mut ds = paths(&dl.sources, &p.duck_paths);
                let mut ws = paths(&wl.sources, &p.ws_paths);
                ds.sort();
                ws.sort();
                assert_eq!(ds, ws, "{name}: sources of {}", wl.key);
                let clusters = |l: &imt_core::api::Lane| {
                    l.markers
                        .iter()
                        .map(|m| (m.kind.clone(), m.bucket, m.ts, m.count, m.message.clone()))
                        .collect::<Vec<_>>()
                };
                assert_eq!(clusters(dl), clusters(wl), "{name}: markers of {}", wl.key);
            }
        }
    }
}

/// Large stores answer timelines from the minute histogram: the same lanes, totals and markers; counts
/// per bucket may move to a neighbouring bucket (a minute is counted whole) and lane bounds are minutes.
fn check_histogram_timelines(p: &Pair) {
    p.duck.set_preaggregate_above(0);
    // One lane per source (a source with events of several instances needs the events).
    let by_source = Filter {
        group_by: vec!["@source".into()],
        ..Filter::default()
    };
    let levels = Filter {
        levels: vec![Level::Error, Level::Warning],
        ..by_source.clone()
    };
    for (name, q) in [("everything", by_source), ("levels", levels)] {
        let req = TimelineRequest {
            query: q,
            buckets: 10,
            always_show_lifecycle: true,
        };
        let w = p.ws.timeline(&req).unwrap();
        let d = p.duck.timeline(&req).unwrap();
        assert!(w.bucket_ms >= 60_000.0, "{name}: the samples span more than ten minutes");
        assert_eq!((d.from, d.to, d.bucket_ms), (w.from, w.to, w.bucket_ms), "{name}: bounds");
        let sum = |h: &[[u32; 6]]| {
            h.iter().fold([0u32; 6], |mut a, b| {
                for (x, y) in a.iter_mut().zip(b) {
                    *x += y;
                }
                a
            })
        };
        assert_eq!(sum(&d.histogram), sum(&w.histogram), "{name}: histogram totals");
        // From the histogram: whole minutes per bucket, so some counts move to a neighbouring bucket.
        assert_ne!(d.histogram, w.histogram, "{name}: the histogram path answers");
        assert_eq!(d.lanes.len(), w.lanes.len(), "{name}: lanes");
        for (dl, wl) in d.lanes.iter().zip(&w.lanes) {
            assert_eq!((&dl.key, dl.total, dl.matched), (&wl.key, wl.total, wl.matched), "{name}: lane");
            // Without a time range, lane bounds are the sources' (exact).
            assert_eq!((dl.first, dl.last), (wl.first, wl.last), "{name}: lane bounds");
            assert_eq!(
                dl.density.iter().sum::<u32>(),
                wl.density.iter().sum::<u32>(),
                "{name}: density of {}",
                wl.key
            );
            let counts = |l: &imt_core::api::Lane| l.markers.iter().map(|m| (m.kind.clone(), m.bucket, m.count)).collect::<Vec<_>>();
            assert_eq!(counts(dl), counts(wl), "{name}: markers of {}", wl.key);
        }
    }
    p.duck.set_preaggregate_above(imt_store_duck::PREAGGREGATE_ABOVE);
}

fn check_facets(p: &Pair) {
    for (name, q) in queries(p) {
        let q = ranged(p, q);
        for field in ["@level", "@source", "@group", "MachineName", "SourceContext", "@line", "nope"] {
            let req = FacetRequest {
                query: q.clone(),
                field: field.into(),
                limit: 50,
            };
            let w = p.ws.facet(&req).unwrap();
            let d = p.duck.facet(&req).unwrap();
            assert_eq!((d.total, d.distinct), (w.total, w.distinct), "{name}: facet {field}");
            let values = |f: &imt_core::api::Facet| f.values.iter().map(|v| (v.value.clone(), v.count)).collect::<Vec<_>>();
            assert_eq!(values(&d), values(&w), "{name}: facet {field}");
        }
    }
}

fn check_summary(p: &Pair) {
    let w = p.ws.summary();
    let d = p.duck.summary().unwrap();
    assert_eq!(d.default_group_by, w.default_group_by);
    assert_eq!((d.from, d.to), (w.from, w.to));
    let timed = |m: &BTreeMap<String, usize>| m.clone();
    let mut ws_sources: BTreeMap<String, usize> = BTreeMap::new();
    for s in &w.sources {
        ws_sources.insert(s.path.clone(), s.records);
    }
    let mut duck_sources: BTreeMap<String, usize> = BTreeMap::new();
    for s in &d.sources {
        duck_sources.insert(s.path.clone(), s.records);
    }
    assert_eq!(
        timed(&duck_sources).keys().collect::<Vec<_>>(),
        timed(&ws_sources).keys().collect::<Vec<_>>(),
        "the same sources"
    );
    let orphans = d.total - w.total;
    for (path, n) in &duck_sources {
        let old = ws_sources[path];
        assert!(*n == old || *n == old + 1, "{path}: {n} events, the in-memory store has {old}");
    }
    assert_eq!(duck_sources.values().sum::<usize>(), d.total);
    assert!(orphans <= d.sources.len());
    eprintln!(
        "{} events ({} untimed, {orphans} from text before a first event), {} sources, {} unrecognized, group by {:?}, {} marker kinds",
        d.total,
        d.untimed,
        d.sources.len(),
        d.unrecognized.len(),
        d.default_group_by,
        d.marker_kinds.len()
    );
    let names: Vec<(String, usize, usize)> = w.fields.iter().map(|f| (f.name.clone(), f.count, f.distinct)).collect();
    let duck_names: Vec<(String, usize, usize)> = d.fields.iter().map(|f| (f.name.clone(), f.count, f.distinct)).collect();
    assert_eq!(duck_names, names, "fields");
    assert_eq!(
        d.unrecognized.iter().map(|u| (&u.path, &u.reason)).collect::<Vec<_>>(),
        w.unrecognized.iter().map(|u| (&u.path, &u.reason)).collect::<Vec<_>>()
    );
}

/// Every condition operator (as a field filter) and every query-language form: the SQL compiler
/// matches the engine's evaluator (the in-memory store) on the samples.
fn check_operators(p: &Pair) {
    let filters: &[(&str, &str, Option<&str>)] = &[
        ("MachineName", "eq", Some("WEB01")),
        ("MachineName", "ne", Some("WEB01")),
        ("@level", "eq", Some("warning")),
        ("@level", "ne", Some("Warning")),
        ("@message", "contains", Some("order")),
        ("@message", "notContains", Some("order")),
        ("Path", "matches", Some("^/api/.*s$")),
        ("StatusCode", "exists", None),
        ("StatusCode", "missing", None),
        ("ElapsedMs", "gt", Some("250")),
        ("ElapsedMs", "lt", Some("20")),
        ("ElapsedMs", "eq", Some("12")),
        ("@level", "gt", Some("2")),
        ("@line", "lt", Some("100")),
        ("MachineName", "in", Some("WEB01|WEB02")),
        ("@source", "contains", Some("web")),
        ("CpuPercent", "gt", Some("50")),
        ("Threads", "lt", Some("40")),
        ("WorkingSetMb", "eq", Some("512")),
    ];
    let mut cases: Vec<(String, Filter)> = filters
        .iter()
        .map(|(field, op, value)| {
            (
                format!("{field} {op} {value:?}"),
                Filter {
                    filters: vec![FieldFilter {
                        field: field.to_string(),
                        op: op.to_string(),
                        value: value.map(str::to_string),
                    }],
                    ..Filter::default()
                },
            )
        })
        .collect();
    for ql in [
        "ElapsedMs > 250",
        "ElapsedMs <= 3",
        "ElapsedMs != 5",
        "Path = '/api/orders'",
        "MachineName != WEB01",
        "level in (warning, error)",
        "level < information",
        "StatusCode in (500, 404)",
        "message contains timeout",
        "message !contains order",
        "not exists(StatusCode)",
        "missing(Upstream) or level >= error",
        "Path ~ 'orders$'",
        "Path !~ 'orders'",
        "timeout",
        "\"payment failed\"",
        "@line >= 10 and @line < 20",
        "source contains gateway",
        "Upstream > m",
        "CpuPercent >= 50 or Threads < 30",
        "| where ElapsedMs > 100 | where MachineName = WEB02",
    ] {
        cases.push((
            format!("ql {ql}"),
            Filter {
                ql: ql.into(),
                ..Filter::default()
            },
        ));
    }
    let group =
        p.ws.facet(&FacetRequest {
            query: Filter::default(),
            field: "@group".into(),
            limit: 3,
        })
        .unwrap();
    cases.push((
        "instance selection".into(),
        Filter {
            groups: group.values.iter().take(2).map(|v| v.value.clone()).collect(),
            ..Filter::default()
        },
    ));
    let mut matched = 0;
    for (name, q) in cases {
        let q = ranged(p, q);
        let (wt, wr) = ws_rows(p, &q, 0, 300, false);
        let (dt, dr) = duck_rows(p, &q, 0, 300, false);
        assert_eq!(dt, wt, "{name}: total");
        assert_eq!(dr, wr, "{name}: rows");
        matched += usize::from(wt > 0);
    }
    assert!(matched > 20, "most cases match events ({matched})");
}

#[test]
fn answers_like_the_workspace_with_the_default_profile() {
    let p = pair(Profile::default_profile());
    check_summary(&p);
    check_operators(&p);
    check_pages(&p);
    check_sources_filter(&p);
    check_anchor(&p);
    check_facets(&p);
    check_timelines(&p);
    check_histogram_timelines(&p);
}

#[test]
fn answers_like_the_workspace_with_the_sample_profile() {
    // Typed extractor fields (declared columns), marker rules, an instance key.
    let p = pair(sample_profile());
    check_summary(&p);
    check_operators(&p);
    check_pages(&p);
    check_facets(&p);
    check_timelines(&p);
    for field in ["CpuPercent", "Threads", "WorkingSetMb", "Backlog"] {
        let req = FacetRequest {
            query: Filter::default(),
            field: field.into(),
            limit: 20,
        };
        let (w, d) = (p.ws.facet(&req).unwrap(), p.duck.facet(&req).unwrap());
        assert_eq!((d.total, d.distinct), (w.total, w.distinct), "declared field {field}");
        assert!(d.total > 0 || field == "Backlog", "{field} has values");
    }
}

#[test]
fn reports_problems_refuses_what_is_not_built_yet_and_reopens() {
    let dir = TempDir::new("misc");
    let paths = vec![common::samples().join("serilog-text"), common::samples().join("sqlserver")];
    let (store, report) = store_with(&dir, Profile::default_profile(), &paths).unwrap();
    assert!(report.events > 0 && report.unrecognized.is_empty(), "{:?}", report.unrecognized);
    // A second open of the same folders reads nothing new.
    let again = store.open(&paths).unwrap().wait().unwrap();
    assert_eq!(again.events, 0);
    let problems = store.problems().unwrap();
    assert!(problems.iter().all(|p| p.count > 0 && p.sample_lines.len() <= 20));
    assert_eq!(store.active_profile().unwrap().name, Profile::default_profile().name);
    assert!(store.validate_profile(&Profile::default_profile()).is_ok());
    let total = store.summary().unwrap().total;
    drop(store);
    let reopened = DuckStore::open(&dir.0, Profile::default_profile(), imt_core::PluginRegistry::new()).unwrap();
    assert_eq!(reopened.summary().unwrap().total, total);
    let first = reopened.summary().unwrap().sources[0].id;
    reopened.remove_source(imt_core::batch::SourceId(first)).unwrap();
    assert!(reopened.summary().unwrap().total < total);
    assert!(matches!(
        reopened.remove_source(imt_core::batch::SourceId(first)),
        Err(StoreError::NotFound(_))
    ));
    reopened.clear().unwrap();
    assert_eq!(reopened.summary().unwrap().total, 0);
    assert!(matches!(
        DuckStore::open(&dir.0.join("none"), Profile::default_profile(), imt_core::PluginRegistry::new()),
        Err(StoreError::NotFound(_))
    ));
}
