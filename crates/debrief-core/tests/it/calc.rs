//! Calculated fields (`step = "calc"`) end to end: a profile computes fields over the
//! gateway sample, and the results work in queries, facets and charts like any other field.

use debrief_core::api::QueryRequest;
use debrief_core::chart::ChartDef;
use debrief_core::ql::{CalcExpr, QlResult};
use debrief_core::query::Query;
use debrief_core::workspace::{ChartRequest, FacetRequest};
use debrief_core::{Profile, Workspace};
use proptest::prelude::*;
use std::path::PathBuf;

fn gateway() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples/serilog-json/gateway")
}

const PROFILE: &str = r#"
name = "Calc"

[[pipeline]]
step = "calc"
field = "ElapsedSec"
expr = "ElapsedMs / 1000"

[[pipeline]]
step = "calc"
field = "Speed"
expr = "if(ElapsedMs >= 1000, 'slow', 'fast')"

[[pipeline]]
step = "calc"
field = "Route"
expr = "lower(Upstream) + coalesce(Path, '')"
"#;

fn workspace() -> Workspace {
    let mut ws = Workspace::new(Profile::from_toml(PROFILE).unwrap()).unwrap();
    ws.open(&[gateway()]).unwrap();
    ws
}

fn query(ql: &str) -> Query {
    Query {
        ql: ql.into(),
        ..Query::default()
    }
}

#[test]
fn calculated_fields_are_written_on_import() {
    let ws = workspace();
    let timed: Vec<_> = ws.records().iter().filter(|r| r.fields.contains_key("ElapsedMs")).collect();
    assert!(timed.len() > 10, "the gateway sample has timed requests");
    for r in &timed {
        let ms = r.fields["ElapsedMs"].as_f64().unwrap();
        let sec = r.fields["ElapsedSec"].as_f64().unwrap();
        assert!((sec - ms / 1000.0).abs() < 1e-9, "{ms} ms -> {sec} s");
        let speed = if ms >= 1000.0 { "slow" } else { "fast" };
        assert_eq!(r.fields["Speed"], speed);
    }
    // Events without ElapsedMs get no ElapsedSec (null writes nothing); if() takes its
    // else branch when the condition is null.
    let untimed = ws.records().iter().find(|r| !r.fields.contains_key("ElapsedMs")).unwrap();
    assert!(!untimed.fields.contains_key("ElapsedSec"));
    assert_eq!(untimed.fields["Speed"], "fast");
    let s = ws.summary();
    let sec = s
        .fields
        .iter()
        .find(|f| f.name == "ElapsedSec")
        .expect("listed with the other fields");
    assert!(sec.numeric);
    // Calculated fields are marked derived, like extractor fields.
    assert!(sec.derived);
    assert!(!s.fields.iter().find(|f| f.name == "ElapsedMs").unwrap().derived);
}

#[test]
fn calculated_fields_work_in_queries_facets_and_charts() {
    let ws = workspace();
    let slow = ws
        .records()
        .iter()
        .filter(|r| r.fields.get("Speed").is_some_and(|v| v == "slow"))
        .count();
    assert!(slow > 0, "the sample has slow upstream calls");

    let QlResult::Events { ids, .. } = ws
        .run_query(&QueryRequest {
            query: query("ElapsedSec >= 1"),
        })
        .unwrap()
    else {
        panic!("expected events");
    };
    assert_eq!(ids.len(), slow);

    let QlResult::Table { columns, rows } = ws
        .run_query(&QueryRequest {
            query: query("| stats count() by Speed | sort Speed"),
        })
        .unwrap()
    else {
        panic!("expected a table");
    };
    assert_eq!(columns, ["Speed", "count()"]);
    assert_eq!(rows.iter().find(|r| r[0] == "slow").unwrap()[1], slow);

    let facet = ws
        .facet(&FacetRequest {
            query: Query::default(),
            field: "Speed".into(),
            limit: 10,
        })
        .unwrap();
    let count = |v: &str| facet.values.iter().find(|x| x.value == v).map(|x| x.count);
    assert_eq!(count("slow"), Some(slow));
    assert!(count("fast").is_some_and(|n| n > 0));

    let chart = ws
        .chart(&ChartRequest {
            query: Query::default(),
            chart_id: None,
            def: Some(ChartDef {
                id: "elapsed".into(),
                title: "Elapsed".into(),
                y: "max(ElapsedSec)".into(),
                bucket: "auto".into(),
                split_by: Vec::new(),
                kind: "line".into(),
                unit: "s".into(),
            }),
            buckets: 50,
        })
        .unwrap();
    let peak = chart
        .series
        .iter()
        .flat_map(|s| s.points.iter().filter_map(|p| p.1))
        .fold(0.0, f64::max);
    let want = ws
        .records()
        .iter()
        .filter_map(|r| r.fields.get("ElapsedSec").and_then(|v| v.as_f64()))
        .fold(0.0, f64::max);
    assert!(peak > 0.0 && (peak - want).abs() < 1e-9, "chart peak {peak}, data peak {want}");
}

#[test]
fn a_bad_expression_is_a_profile_error() {
    let bad = PROFILE.replace("ElapsedMs / 1000", "ElapsedMs / (1000");
    let profile = Profile::from_toml(&bad).unwrap();
    let err = format!("{:#}", Workspace::new(profile).err().expect("refused"));
    assert!(
        err.contains("pipeline step 1 (calc)") && err.contains("column 13: unclosed '('"),
        "{err}"
    );
}

/// Text with the characters that matter to the calc grammar, so cases reach deep into the parser.
fn calc_text() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        Just("("),
        Just(")"),
        Just(","),
        Just(" + "),
        Just("-"),
        Just("*"),
        Just("/"),
        Just("%"),
        Just(" = "),
        Just("=="),
        Just("!"),
        Just("<="),
        Just("~"),
        Just(" and "),
        Just(" or "),
        Just("not "),
        Just("1e-3"),
        Just("12"),
        Just(".5"),
        Just("'s'"),
        Just("\""),
        Just("`f`"),
        Just("x"),
        Just("@line"),
        Just("round("),
        Just("substr("),
        Just("if("),
        Just("coalesce("),
        Just("null"),
        Just("é"),
    ];
    prop_oneof![prop::collection::vec(piece, 0..40).prop_map(|v| v.concat()), any::<String>(),]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn parsing_and_evaluating_never_panic(text in calc_text()) {
        let mut r = debrief_core::model::Record::new(1);
        r.fields.insert("x".into(), serde_json::json!("3"));
        r.fields.insert("f".into(), serde_json::json!(0));
        match CalcExpr::parse(&text) {
            Ok(e) => {
                let _ = e.eval_json(&r, "s");
            }
            Err(e) => {
                prop_assert!(e.offset + e.len <= text.len(), "span {}+{} outside {} bytes", e.offset, e.len, text.len());
                prop_assert!(!e.message.is_empty());
            }
        }
    }
}
