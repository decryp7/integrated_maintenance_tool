//! Dashboard tiles (`imt_core::tiles::evaluate`) over the in-memory store ports, on the serilog-clef samples:
//! every tile kind answers from the store, limits and live windows apply, and invalid tiles are typed errors.

use imt_core::dashboard::{TileCfg, TileData, TileKind, TileRequest, SPARKLINE_POINTS};
use imt_core::plugin::annotate::{AnnotationDto, AnnotationsRequest};
use imt_core::ports::{Query as _, StoreError};
use imt_core::query::{Query, TimeWindow};
use imt_core::tiles;
use imt_core::workspace::WorkspaceStore;
use imt_core::{Profile, Workspace};
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::OnceLock;

fn store() -> &'static WorkspaceStore {
    static S: OnceLock<WorkspaceStore> = OnceLock::new();
    S.get_or_init(|| {
        let mut ws = Workspace::new(Profile::default_profile()).unwrap();
        ws.open(&[PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples/serilog-clef")])
            .unwrap();
        WorkspaceStore::new(ws)
    })
}

fn tile(toml: &str) -> TileCfg {
    toml::from_str(&format!("id = \"t\"\n{toml}")).unwrap()
}

fn no_annotations(_: &AnnotationsRequest) -> Vec<AnnotationDto> {
    Vec::new()
}

fn eval(t: TileCfg, query: Query) -> Result<TileData, StoreError> {
    tiles::evaluate(store(), &no_annotations, &TileRequest { tile: t, query })
}

fn total() -> u64 {
    store().summary().unwrap().total as u64
}

#[test]
fn a_stat_tile_counts_and_draws_a_sparkline_that_adds_up() {
    let TileData::Stat {
        value,
        text,
        label,
        unit,
        sparkline,
    } = eval(
        tile("type = \"stat\"\nquery = \"| stats count()\"\nunit = \"events\""),
        Query::default(),
    )
    .unwrap()
    else {
        panic!("not a stat")
    };
    assert_eq!(value, Some(total() as f64));
    assert_eq!(text, total().to_string());
    assert_eq!(label, "count()");
    assert_eq!(unit.as_deref(), Some("events"));
    assert_eq!(sparkline.len(), SPARKLINE_POINTS);
    // The buckets partition the range: their counts add up to the total.
    let sum: f64 = sparkline.iter().filter_map(|p| p.1).sum();
    assert_eq!(sum, total() as f64);
    assert!(sparkline.windows(2).all(|w| w[0].0 < w[1].0), "bucket starts increase");
}

#[test]
fn a_stat_tile_combines_its_filter_with_the_base_query() {
    let base = Query {
        ql: "level >= error".into(),
        ..Query::default()
    };
    let errors = match eval(tile("type = \"stat\"\nquery = \"| stats count()\""), base.clone()).unwrap() {
        TileData::Stat { value, .. } => value.unwrap(),
        other => panic!("{other:?}"),
    };
    assert!(errors > 0.0 && errors < total() as f64, "{errors}");
    let none = match eval(
        tile("type = \"stat\"\nquery = \"@message = \\\"no such message\\\" | stats count()\""),
        base,
    )
    .unwrap()
    {
        TileData::Stat { value, .. } => value,
        other => panic!("{other:?}"),
    };
    assert_eq!(none, Some(0.0));
}

#[test]
fn a_table_tile_truncates_tables_and_turns_events_into_rows() {
    let TileData::Table { columns, rows, truncated } = eval(
        tile("type = \"table\"\nquery = \"| stats count() by @level\"\nlimit = 1"),
        Query::default(),
    )
    .unwrap() else {
        panic!("not a table")
    };
    assert_eq!(columns.last().map(String::as_str), Some("count()"));
    assert_eq!(rows.len(), 1);
    assert!(truncated, "more than one level in the samples");

    let TileData::Table { columns, rows, truncated } =
        eval(tile("type = \"table\"\nquery = \"level >= error\"\nlimit = 3"), Query::default()).unwrap()
    else {
        panic!("not a table")
    };
    assert_eq!(columns, ["@t", "@level", "@message"]);
    assert_eq!(rows.len(), 3);
    assert!(truncated);
    for r in &rows {
        assert!(matches!(r[1].as_str(), Some("Error" | "Fatal")), "{r:?}");
        assert!(r[0].is_number());
    }
}

#[test]
fn chart_top_and_alert_tiles_answer_from_the_store() {
    match eval(
        tile("type = \"chart\"\nvalue = \"count()\"\nby = [\"@level\"]\nstyle = \"bar\""),
        Query::default(),
    )
    .unwrap()
    {
        TileData::Chart { data } => {
            let points: f64 = data.series.iter().flat_map(|s| s.points.iter().filter_map(|p| p.1)).sum();
            assert_eq!(points, total() as f64, "the bars add up to the events");
            assert!(data.series.len() > 1, "split by level");
        }
        other => panic!("{other:?}"),
    }
    match eval(tile("type = \"top\"\nfield = \"@level\"\nlimit = 2"), Query::default()).unwrap() {
        TileData::Top { facet } => {
            assert!(facet.values.len() <= 2 && !facet.values.is_empty());
            assert!(facet.values.windows(2).all(|w| w[0].count >= w[1].count), "most frequent first");
        }
        other => panic!("{other:?}"),
    }
    match eval(tile("type = \"alerts\"\nlimit = 1"), Query::default()).unwrap() {
        TileData::Alerts { items } => assert!(items.len() <= 1),
        other => panic!("{other:?}"),
    }
}

#[test]
fn annotation_tiles_ask_the_host_for_the_tile_window_and_keep_the_limit() {
    let asked = RefCell::new(Vec::new());
    let annotations = |r: &AnnotationsRequest| {
        asked.borrow_mut().push((r.from, r.to));
        let sample = |i: i64| AnnotationDto {
            annotator: "a".into(),
            owner: "p".into(),
            kind: "p/reboot".into(),
            from: i,
            to: i + 1,
            instance: None,
            on: None,
            label: format!("reboot {i}"),
            severity: imt_core::api::Severity::Info,
            evidence_query: Default::default(),
            run: None,
        };
        (0..5).map(sample).collect()
    };
    let newest = store().summary().unwrap().to.unwrap();
    let data = tiles::evaluate(
        store(),
        &annotations,
        &TileRequest {
            tile: tile("type = \"annotations\"\nwindow = \"1m\"\nlimit = 2"),
            query: Query::default(),
        },
    )
    .unwrap();
    let TileData::Annotations { items } = data else {
        panic!("not annotations")
    };
    assert_eq!(items.len(), 2);
    // The live window became a range ending at the newest event.
    assert_eq!(asked.borrow().as_slice(), [(Some(newest - 60_000), Some(newest))]);
}

#[test]
fn a_live_base_window_resolves_against_the_newest_event() {
    let base = Query {
        window: Some(TimeWindow::Live { last_ms: 1 }),
        ..Query::default()
    };
    let in_window = match eval(tile("type = \"stat\"\nquery = \"| stats count()\""), base).unwrap() {
        TileData::Stat { value, .. } => value.unwrap(),
        other => panic!("{other:?}"),
    };
    assert!(in_window >= 1.0 && in_window < total() as f64, "{in_window}");
}

#[test]
fn invalid_tiles_are_typed_errors() {
    for (toml, needle) in [
        ("type = \"stat\"\nquery = \"level >= error\"", "stats"),
        ("type = \"stat\"", "needs a query"),
        ("type = \"top\"", "field"),
        ("type = \"chart\"", "chart"),
        ("type = \"chart\"\nvalue = \"count()\"\nstyle = \"pie\"", "style"),
        ("type = \"table\"\nquery = \"where ===\"", "query"),
        ("type = \"stat\"\nquery = \"| stats count()\"\nwindow = \"forever\"", "window"),
    ] {
        match eval(tile(toml), Query::default()) {
            Err(StoreError::Invalid(m)) => assert!(m.contains(needle), "{toml}: {m}"),
            other => panic!("{toml}: {other:?}"),
        }
    }
    let wide = TileCfg {
        w: 99,
        ..tile("type = \"annotations\"")
    };
    assert!(matches!(eval(wide, Query::default()), Err(StoreError::Invalid(_))));
    assert_eq!(TileKind::Stat, tile("type = \"stat\"").kind);
}

#[test]
fn a_chart_tile_can_point_at_a_profile_chart_or_carry_a_title() {
    match eval(tile("type = \"chart\"\nchart = \"events-by-level\""), Query::default()).unwrap() {
        TileData::Chart { data } => {
            assert_eq!(
                data.def.title, "Events per interval by level",
                "the profile chart's own definition is used"
            );
            assert!(!data.series.is_empty());
        }
        other => panic!("{other:?}"),
    }
    match eval(tile("type = \"chart\"\ntitle = \"Volume\"\nvalue = \"count()\""), Query::default()).unwrap() {
        TileData::Chart { data } => assert_eq!(data.def.title, "Volume"),
        other => panic!("{other:?}"),
    }
    match eval(tile("type = \"chart\"\nvalue = \"count()\""), Query::default()).unwrap() {
        TileData::Chart { data } => assert_eq!(data.def.title, "t", "an untitled tile is named by its id"),
        other => panic!("{other:?}"),
    }
}
