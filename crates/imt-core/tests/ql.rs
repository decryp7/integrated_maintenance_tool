//! Behaviour of the query language through its public API.

use imt_core::model::{Level, Record};
use imt_core::ql::{self, CompletionKind, QlError, QlResult};
use serde_json::{json, Value};

fn rec(id: u32, level: Level, msg: &str, fields: &[(&str, Value)]) -> Record {
    let mut r = Record::new(id + 100);
    r.id = id;
    r.level = level;
    r.message = msg.to_string();
    for (k, v) in fields {
        r.fields.insert((*k).to_string(), v.clone());
    }
    r
}

fn data() -> Vec<(Record, &'static str)> {
    let mut v = vec![
        (
            rec(
                0,
                Level::Information,
                "Started service",
                &[("ms", json!(10)), ("user", json!("alice")), ("code", json!("200"))],
            ),
            "a.log",
        ),
        (
            rec(1, Level::Warning, "Slow request", &[("ms", json!(250)), ("user", json!("bob"))]),
            "a.log",
        ),
        (
            rec(
                2,
                Level::Error,
                "Request failed: Timeout",
                &[("ms", json!(900)), ("user", json!("alice")), ("odd name", json!("x"))],
            ),
            "b.log",
        ),
        (
            rec(3, Level::Debug, "cache hit", &[("ms", json!(2.5)), ("user", json!("carol"))]),
            "b.log",
        ),
        (rec(4, Level::Fatal, "Crash", &[("ms", json!(5))]), "b.log"),
    ];
    v[2].0.exception = Some("System.TimeoutException: boom".into());
    v[3].0.template = Some("Cache {Result}".into());
    v
}

fn run_on(q: &str, d: &[(Record, &str)]) -> Result<QlResult, QlError> {
    let p = ql::parse(q)?;
    ql::run(&p, &mut d.iter().map(|(r, s)| (r, *s)))
}

fn ids(q: &str) -> Vec<u32> {
    match run_on(q, &data()).unwrap_or_else(|e| panic!("{q}: {e}")) {
        QlResult::Events { ids, .. } => ids,
        t => panic!("expected events, got {t:?}"),
    }
}

fn table(q: &str) -> (Vec<String>, Vec<Vec<Value>>) {
    match run_on(q, &data()).unwrap_or_else(|e| panic!("{q}: {e}")) {
        QlResult::Table { columns, rows } => (columns, rows),
        e => panic!("expected table, got {e:?}"),
    }
}

fn err(q: &str) -> QlError {
    match ql::parse(q) {
        Err(e) => e,
        Ok(_) => match run_on(q, &data()) {
            Err(e) => e,
            Ok(_) => panic!("{q} should fail"),
        },
    }
}

fn span<'a>(q: &'a str, e: &QlError) -> &'a str {
    &q[e.offset..e.offset + e.len]
}

#[test]
fn empty_query_matches_everything() {
    assert_eq!(ids(""), vec![0, 1, 2, 3, 4]);
    assert_eq!(ids("   "), vec![0, 1, 2, 3, 4]);
    assert_eq!(ids("| head 2"), vec![0, 1]);
}

#[test]
fn full_text_search() {
    assert_eq!(ids("timeout"), vec![2]);
    assert_eq!(ids("\"SLOW request\""), vec![1]);
    assert_eq!(ids("'cache'"), vec![3]);
    assert_eq!(ids("TimeoutException"), vec![2]);
    assert_eq!(ids("Cache"), vec![3]);
    assert_eq!(ids("nothing-like-this"), Vec::<u32>::new());
    // keywords used as text when quoted
    assert_eq!(ids("\"and\""), Vec::<u32>::new());
    // a bare keyword-ish word that is not followed by an operator is text
    assert_eq!(ids("exists"), Vec::<u32>::new());
    assert_eq!(ids("in"), Vec::<u32>::new());
}

#[test]
fn precedence_not_and_or() {
    // and binds tighter than or
    assert_eq!(ids("crash or slow and request"), vec![1, 4]);
    assert_eq!(ids("(crash or slow) and request"), vec![1]);
    // not binds tighter than and
    assert_eq!(ids("not slow and request"), vec![2]);
    assert_eq!(ids("not (slow and request)"), vec![0, 2, 3, 4]);
    assert_eq!(ids("not not slow"), vec![1]);
    // implicit and
    assert_eq!(ids("slow request"), vec![1]);
    assert_eq!(ids("SLOW AND REQUEST"), vec![1]);
    assert_eq!(ids("slow OR crash"), vec![1, 4]);
    assert_eq!(ids("NOT slow level = error"), vec![2]);
}

#[test]
fn comparisons_numeric_and_text() {
    assert_eq!(ids("ms = 10"), vec![0]);
    assert_eq!(ids("ms = 10.0"), vec![0]);
    assert_eq!(ids("ms != 10"), vec![1, 2, 3, 4]);
    assert_eq!(ids("ms > 10"), vec![1, 2]);
    assert_eq!(ids("ms >= 10"), vec![0, 1, 2]);
    assert_eq!(ids("ms < 10"), vec![3, 4]);
    assert_eq!(ids("ms <= 10"), vec![0, 3, 4]);
    // numeric, not lexicographic: "250" < "5" as text
    assert_eq!(ids("ms > 5"), vec![0, 1, 2]);
    // strings compare as text
    assert_eq!(ids("user = alice"), vec![0, 2]);
    assert_eq!(ids("user = \"alice\""), vec![0, 2]);
    assert_eq!(ids("user != alice"), vec![1, 3, 4]);
    assert_eq!(ids("user > bob"), vec![3]);
    assert_eq!(ids("user >= bob"), vec![1, 3]);
    assert_eq!(ids("user < bob"), vec![0, 2]);
    assert_eq!(ids("user <= bob"), vec![0, 1, 2]);
    // a quoted number is text, so it compares lexicographically ("5" > "250" > "10" > "2.5"; "900" > "5")
    assert_eq!(ids("ms > \"5\""), vec![2]);
    // missing fields never match ordering/equality, but do match !=
    assert_eq!(ids("`odd name` = x"), vec![2]);
    assert_eq!(ids("`odd name` != x"), vec![0, 1, 3, 4]);
    // numeric strings in data are numbers
    assert_eq!(ids("code = 200"), vec![0]);
    assert_eq!(ids("code > 100"), vec![0]);
    // pseudo fields
    assert_eq!(ids("@line = 101"), vec![1]);
    assert_eq!(ids("@line >= 103"), vec![3, 4]);
    assert_eq!(ids("@source = b.log"), vec![2, 3, 4]);
    assert_eq!(ids("source = a.log"), vec![0, 1]);
    assert_eq!(ids("@message = Crash"), vec![4]);
    assert_eq!(ids("message = \"Slow request\""), vec![1]);
}

#[test]
fn level_compares_by_severity() {
    assert_eq!(ids("level >= warning"), vec![1, 2, 4]);
    assert_eq!(ids("level > warning"), vec![2, 4]);
    assert_eq!(ids("level < information"), vec![3]);
    assert_eq!(ids("level <= info"), vec![0, 3]);
    assert_eq!(ids("level = ERROR"), vec![2]);
    assert_eq!(ids("@level != error"), vec![0, 1, 3, 4]);
    assert_eq!(ids("level in (warn, fatal)"), vec![1, 4]);
    assert_eq!(ids("level contains err"), vec![2]);
    assert_eq!(ids("level ~ \"^(Warning|Fatal)$\""), vec![1, 4]);
    assert_eq!(ids("level = \"warning\""), vec![1]);
    let e = err("level > loud");
    assert!(e.message.contains("unknown level"), "{e}");
    assert_eq!(span("level > loud", &e), "loud");
    assert!(err("level in (warning, nope)").message.contains("unknown level"));
}

#[test]
fn contains_regex_in_exists_missing() {
    assert_eq!(ids("user contains LIC"), vec![0, 2]);
    assert_eq!(ids("user !contains lic"), vec![1, 3, 4]);
    assert_eq!(ids("user ! contains lic"), vec![1, 3, 4]);
    assert_eq!(ids("message contains \"slow req\""), vec![1]);
    assert_eq!(ids("exception contains boom"), vec![2]);
    assert_eq!(ids("message ~ \"^S\\w+ \""), vec![0, 1]);
    assert_eq!(ids("message ~ ^Sl"), vec![1]);
    assert_eq!(ids("message !~ \"^S\""), vec![2, 3, 4]);
    assert_eq!(ids("user ~ \"(?i)ALICE\""), vec![0, 2]);
    assert_eq!(ids("user in (alice, carol)"), vec![0, 2, 3]);
    assert_eq!(ids("user in (\"bob\")"), vec![1]);
    assert_eq!(ids("ms in (10, 5)"), vec![0, 4]);
    assert_eq!(ids("ms IN (2.5)"), vec![3]);
    assert_eq!(ids("exists(user)"), vec![0, 1, 2, 3]);
    assert_eq!(ids("missing(user)"), vec![4]);
    assert_eq!(ids("exists(exception)"), vec![2]);
    assert_eq!(ids("missing(@exception)"), vec![0, 1, 3, 4]);
    assert_eq!(ids("EXISTS(`odd name`)"), vec![2]);
    assert_eq!(ids("exists(message) and not missing(user)"), vec![0, 1, 2, 3]);
    assert_eq!(ids("exists(template)"), Vec::<u32>::new());
}

#[test]
fn quoting_and_escaping() {
    let mut d = data();
    d[0].0.message = "say \"hi\" it's".into();
    d[1].0.message = "path C:\\temp\\x".into();
    let q = |q: &str| match run_on(q, &d).unwrap() {
        QlResult::Events { ids, .. } => ids,
        _ => panic!(),
    };
    assert_eq!(q(r#"message contains "say \"hi\"""#), vec![0]);
    assert_eq!(q(r#"message contains 'it\'s'"#), vec![0]);
    assert_eq!(q(r#"message contains "it's""#), vec![0]);
    assert_eq!(q(r#""\"hi\"""#), vec![0]);
    assert_eq!(q(r#"message contains "C:\\temp""#), vec![1]);
    assert_eq!(q(r#"message ~ "C:\\\\temp""#), vec![1]);
    // unknown escapes are kept, so regex classes work
    assert_eq!(q(r#"message ~ "t\w+p""#), vec![1]);
    assert_eq!(q("`user` = bob"), vec![1]);
}

#[test]
fn parse_errors_have_spans() {
    let cases: &[(&str, &str, &str)] = &[
        ("error | frobnicate 3", "unknown stage", "frobnicate"),
        ("level >", "expected a value", ""),
        ("level > |", "expected a value", "|"),
        ("(a or b", "unclosed '('", "("),
        ("a and (b", "unclosed '('", "("),
        ("a)", "unmatched ')'", ")"),
        ("a = \"open", "unterminated string", "\"open"),
        ("a = `open", "unterminated field name", "`open"),
        ("`` = 1", "empty field name", "``"),
        ("a ~ \"(\"", "invalid regex", "\"(\""),
        ("a ~ (", "expected a value", "("),
        ("foo(x)", "unknown function 'foo'", "foo"),
        ("exists(", "expected a field name", ""),
        ("exists(a", "unclosed '('", "("),
        ("exists(= )", "expected a field name", "="),
        ("`field`", "expected an operator", ""),
        ("a in b", "expected '(' after 'in'", "in"),
        ("a in", "expected '(' after 'in'", "in"),
        ("a in (1, 2", "unclosed '('", "("),
        ("a in ()", "expected a value", ")"),
        ("a ! b", "expected 'contains' after '!'", "!"),
        ("a !", "expected 'contains' after '!'", "!"),
        ("a and", "expected an expression", ""),
        ("and a", "expected an expression before 'and'", "and"),
        ("a or or b", "expected an expression before 'or'", "or"),
        ("not", "expected an expression", ""),
        ("a ,", "unexpected ','", ","),
        ("= 3", "expected an expression", "="),
        ("a | where", "expected an expression", ""),
        ("a |", "expected a stage name", ""),
        ("a | (", "expected a stage name", "("),
        ("a | head", "needs a number", ""),
        ("a | head x", "needs a non-negative whole number", "x"),
        ("a | head -1", "needs a non-negative whole number", "-1"),
        ("a | head (", "needs a number", "("),
        ("a | stats", "expected an aggregate", ""),
        ("a | stats (", "expected an aggregate", "("),
        ("a | stats median(x)", "unknown function 'median'", "median"),
        ("a | stats count", "expected '(' after 'count'", "count"),
        ("a | stats avg()", "expected a field name", ")"),
        ("a | stats count(x)", "unclosed '('", "("),
        ("a | stats avg(x", "unclosed '('", "("),
        ("a | stats count() as", "expected an alias name", ""),
        ("a | stats count() as (", "expected an alias name", "("),
        ("a | stats count() by", "expected a field name", ""),
        ("a | stats count(), ", "expected an aggregate", ""),
        ("a | sort", "expected a field name", ""),
        ("a | sort x,", "expected a field name", ""),
        ("a | fields", "expected a field name", ""),
        ("a | fields x y", "unexpected 'y'", "y"),
        ("a | head 2 3", "unexpected '3'", "3"),
    ];
    for (q, msg, bad) in cases {
        let e = ql::parse(q).expect_err(q);
        assert!(e.message.contains(msg), "{q}: {e}");
        assert_eq!(span(q, &e), *bad, "{q}: {e}");
        if bad.is_empty() {
            assert_eq!((e.offset, e.len), (q.len(), 0), "{q}");
        }
    }
}

#[test]
fn deep_nesting_is_rejected_not_overflowed() {
    let q = format!("{}a{}", "(".repeat(5000), ")".repeat(5000));
    assert!(ql::parse(&q).unwrap_err().message.contains("nested too deeply"));
    let q = format!("{}a", "not ".repeat(5000));
    assert!(ql::parse(&q).unwrap_err().message.contains("nested too deeply"));
    let q = format!("{}a{}", "(".repeat(30), ")".repeat(30));
    assert!(ql::parse(&q).is_ok());
}

#[test]
fn error_display_and_json() {
    let e = ql::parse("a )").unwrap_err();
    assert_eq!(e.to_string(), "unmatched ')' (at byte 2)");
    assert_eq!(
        serde_json::to_value(&e).unwrap(),
        json!({"message": "unmatched ')'", "offset": 2, "len": 1})
    );
    let _: &dyn std::error::Error = &e;
}

#[test]
fn where_stage_and_chained_filters() {
    assert_eq!(ids("user = alice | where ms > 100"), vec![2]);
    assert_eq!(ids("| where level >= error | where ms > 100"), vec![2]);
    assert_eq!(ids("| head 3 | where ms > 100"), vec![1, 2]);
}

#[test]
fn stats_aggregates() {
    let (cols, rows) = table("| stats count(), count_distinct(user), sum(ms), min(ms), max(ms), avg(ms)");
    assert_eq!(
        cols,
        vec!["count()", "count_distinct(user)", "sum(ms)", "min(ms)", "max(ms)", "avg(ms)"]
    );
    assert_eq!(
        rows,
        vec![vec![json!(5), json!(3), json!(1167.5), json!(2.5), json!(900), json!(233.5)]]
    );
    let (_, rows) = table("| stats sum(ms)");
    assert_eq!(rows, vec![vec![json!(1167.5)]]);
    let (_, rows) = table("ms >= 10 | stats sum(ms)");
    assert_eq!(rows, vec![vec![json!(1160)]]);
}

#[test]
fn percentiles_are_exact_nearest_rank() {
    let d: Vec<(Record, &str)> = (1..=100)
        .map(|i| (rec(i, Level::Information, "m", &[("v", json!(i))]), "s"))
        .collect();
    let run = |q: &str| match run_on(q, &d).unwrap() {
        QlResult::Table { rows, .. } => rows,
        _ => panic!(),
    };
    assert_eq!(run("| stats p50(v), p95(v), p99(v)"), vec![vec![json!(50), json!(95), json!(99)]]);
    let one = vec![(rec(1, Level::Information, "m", &[("v", json!(7))]), "s")];
    match run_on("| stats p50(v) as a, p99(v) as b", &one).unwrap() {
        QlResult::Table { columns, rows } => {
            assert_eq!(columns, vec!["a", "b"]);
            assert_eq!(rows, vec![vec![json!(7), json!(7)]]);
        }
        _ => panic!(),
    }
}

#[test]
fn stats_with_no_data_or_values() {
    let none: Vec<(Record, &str)> = Vec::new();
    match run_on("| stats count(), sum(x), avg(x), min(x), max(x), p95(x), count_distinct(x)", &none).unwrap() {
        QlResult::Table { rows, .. } => assert_eq!(
            rows,
            vec![vec![
                json!(0),
                json!(0),
                Value::Null,
                Value::Null,
                Value::Null,
                Value::Null,
                json!(0)
            ]]
        ),
        _ => panic!(),
    }
    match run_on("| stats count() by user", &none).unwrap() {
        QlResult::Table { rows, .. } => assert!(rows.is_empty()),
        _ => panic!(),
    }
    // non-numeric values are ignored by numeric aggregates
    let (_, rows) = table("| stats avg(user), p50(user)");
    assert_eq!(rows, vec![vec![Value::Null, Value::Null]]);
    // non-finite averages cannot be represented; they become null
    let big = vec![
        (rec(1, Level::Information, "m", &[("v", json!(1e308))]), "s"),
        (rec(2, Level::Information, "m", &[("v", json!(1e308))]), "s"),
    ];
    match run_on("| stats sum(v)", &big).unwrap() {
        QlResult::Table { rows, .. } => assert_eq!(rows, vec![vec![Value::Null]]),
        _ => panic!(),
    }
}

#[test]
fn stats_by_fields() {
    let (cols, rows) = table("| stats count() as n, avg(ms) as avg_ms by source");
    assert_eq!(cols, vec!["source", "n", "avg_ms"]);
    assert_eq!(
        rows,
        vec![
            vec![json!("a.log"), json!(2), json!(130)],
            vec![json!("b.log"), json!(3), json!(302.5)]
        ]
    );

    let (cols, rows) = table("| stats count() by user, level");
    assert_eq!(cols, vec!["user", "level", "count()"]);
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[0], vec![json!("alice"), json!("Information"), json!(1)]);
    // missing group values become null
    assert_eq!(rows[4], vec![Value::Null, json!("Fatal"), json!(1)]);

    // pseudo-field names and numeric groups
    let (cols, rows) = table("| stats count() by @source, ms");
    assert_eq!(cols[0], "@source");
    assert_eq!(cols, vec!["@source", "ms", "count()"]);
    assert_eq!(rows.len(), 5);
}

#[test]
fn sort_head_fields_on_events() {
    assert_eq!(ids("| sort ms"), vec![3, 4, 0, 1, 2]);
    assert_eq!(ids("| sort ms desc"), vec![2, 1, 0, 4, 3]);
    assert_eq!(ids("| sort ms asc | head 2"), vec![3, 4]);
    assert_eq!(ids("| sort level desc"), vec![4, 2, 1, 0, 3]);
    assert_eq!(ids("| sort user, ms desc"), vec![2, 0, 1, 3, 4]);
    assert_eq!(ids("| sort user desc"), vec![3, 1, 0, 2, 4]);
    // byte order: "Crash" < "Request failed: Timeout" < "Slow request" < "Started service" < "cache hit"
    assert_eq!(ids("| sort @message"), vec![4, 2, 1, 0, 3]);
    // missing values sort last in both directions
    assert_eq!(ids("| sort user"), vec![0, 2, 1, 3, 4]);
    assert_eq!(ids("| sort `odd name` desc"), vec![2, 0, 1, 3, 4]);
    assert_eq!(ids("| limit 2"), vec![0, 1]);
    assert_eq!(ids("| head 0"), Vec::<u32>::new());
    assert_eq!(ids("| head 100"), vec![0, 1, 2, 3, 4]);
    // a mixed column: numbers sort before text
    let mut d = data();
    d[0].0.fields.insert("ms".into(), json!("fast"));
    match run_on("| sort ms", &d).unwrap() {
        QlResult::Events { ids, .. } => assert_eq!(ids, vec![3, 4, 1, 2, 0]),
        _ => panic!(),
    }
    d[3].0.fields.insert("ms".into(), json!("slow"));
    for (q, want) in [("| sort ms", vec![4, 1, 2, 0, 3]), ("| sort ms desc", vec![3, 0, 2, 1, 4])] {
        match run_on(q, &d).unwrap() {
            QlResult::Events { ids, .. } => assert_eq!(ids, want, "{q}"),
            _ => panic!(),
        }
    }
    match run_on("| fields user, ms, `odd name` | head 1", &data()).unwrap() {
        QlResult::Events { ids, fields } => {
            assert_eq!(ids, vec![0]);
            assert_eq!(fields, Some(vec!["user".into(), "ms".into(), "odd name".into()]));
        }
        _ => panic!(),
    }
    match run_on("", &data()).unwrap() {
        QlResult::Events { fields, .. } => assert_eq!(fields, None),
        _ => panic!(),
    }
}

#[test]
fn stages_after_stats_work_on_the_table() {
    let q = "| stats count() as n, avg(ms) as m by source | sort m desc";
    let (_, rows) = table(q);
    assert_eq!(rows[0][0], json!("b.log"));
    let (_, rows) = table("| stats count() as n by user | sort n desc, user | head 2");
    assert_eq!(rows, vec![vec![json!("alice"), json!(2)], vec![json!("bob"), json!(1)]]);
    let (_, rows) = table("| stats count() as n by user | sort n");
    assert_eq!(rows.last().unwrap(), &vec![json!("alice"), json!(2)]);
    // where on the result table, including aliases of pseudo-fields and backticks
    let (_, rows) = table("| stats count() as n by level | where n >= 1 and level >= error");
    assert_eq!(rows, vec![vec![json!("Error"), json!(1)], vec![json!("Fatal"), json!(1)]]);
    let (_, rows) = table("| stats count(), sum(ms) by source | where `sum(ms)` > 300");
    assert_eq!(rows, vec![vec![json!("b.log"), json!(3), json!(907.5)]]);
    let (_, rows) = table("| stats count() as n by user | where user = alice");
    assert_eq!(rows, vec![vec![json!("alice"), json!(2)]]);
    let (_, rows) = table("| stats count() as n by user | where alice");
    assert_eq!(rows, vec![vec![json!("alice"), json!(2)]]);
    let (_, rows) = table("| stats count() as n by user | where missing(user)");
    assert_eq!(rows, vec![vec![Value::Null, json!(1)]]);
    let (_, rows) = table("| stats count() as n by user | where n > 1 and exists(user)");
    assert_eq!(rows.len(), 1);
    // fields projects columns (and reorders)
    let (cols, rows) = table("| stats count() as n, sum(ms) as s by source | fields s, source");
    assert_eq!(cols, vec!["s", "source"]);
    assert_eq!(rows[0], vec![json!(260), json!("a.log")]);
    // a second stats aggregates the first result
    let (cols, rows) = table("| stats count() as n by user | stats count() as users, sum(n) as total");
    assert_eq!(cols, vec!["users", "total"]);
    assert_eq!(rows, vec![vec![json!(4), json!(5)]]);
    // table level text sorts by value
    let (_, rows) = table("| stats count() by level | sort level");
    assert_eq!(rows.len(), 5);
    let (_, rows) = table("| stats count() by level | where level = error");
    assert_eq!(rows, vec![vec![json!("Error"), json!(1)]]);
    let (_, rows) = table("| stats count() by ms | sort ms | head 1");
    assert_eq!(rows[0][0], json!(2.5));
    let (_, rows) = table("| stats count() by ms | head 1");
    assert_eq!(rows.len(), 1);
    let (cols, rows) = table("| stats count() as n by user | stats count() as users by n | sort n");
    assert_eq!(cols, vec!["n", "users"]);
    assert_eq!(rows, vec![vec![json!(1), json!(3)], vec![json!(2), json!(1)]]);
    // non-scalar cells (booleans here) group and sort as text
    let mut d = data();
    d[0].0.fields.insert("flag".into(), json!(true));
    d[1].0.fields.insert("flag".into(), json!([1]));
    match run_on("| stats count() by flag | sort flag", &d).unwrap() {
        QlResult::Table { rows, .. } => assert_eq!(rows.len(), 3),
        _ => panic!(),
    }
}

#[test]
fn run_errors_point_at_the_query() {
    let q = "| stats count() as n | sort missing_col";
    let e = err(q);
    assert!(e.message.contains("no column 'missing_col'") && e.message.contains("n"), "{e}");
    assert_eq!(span(q, &e), "missing_col");
    let q = "| stats count() as n | fields n, zzz";
    assert_eq!(span(q, &err(q)), "zzz");
}

#[test]
fn percentile_cap_is_an_error() {
    let cap = ql::MAX_PERCENTILE_VALUES;
    let r = rec(1, Level::Information, "m", &[("v", json!(1))]);
    let q = "| stats p95(v)";
    let p = ql::parse(q).unwrap();
    let e = ql::run(&p, &mut std::iter::repeat_n((&r, "s"), cap + 1)).unwrap_err();
    assert!(e.message.contains("exact percentile"), "{e}");
    assert_eq!(span(q, &e), "p95(v)");
    assert!(ql::run(&p, &mut std::iter::repeat_n((&r, "s"), cap)).is_ok());
}

#[test]
fn result_json_shape() {
    let r = run_on("| sort ms | fields ms | head 1", &data()).unwrap();
    assert_eq!(
        serde_json::to_value(&r).unwrap(),
        json!({"kind": "events", "ids": [3], "fields": ["ms"]})
    );
    let r = run_on("| stats count() as n", &data()).unwrap();
    assert_eq!(
        serde_json::to_value(&r).unwrap(),
        json!({"kind": "table", "columns": ["n"], "rows": [[5]]})
    );
}

#[test]
fn filter_matches_directly() {
    let p = ql::parse("level >= error and source = b.log").unwrap();
    let d = data();
    let hits: Vec<u32> = d.iter().filter(|(r, s)| p.filter.matches(r, s)).map(|(r, _)| r.id).collect();
    assert_eq!(hits, vec![2, 4]);
    assert!(p.stages.is_empty());
}

#[test]
fn never_panics_on_arbitrary_input() {
    let alphabet: Vec<&str> = vec![
        "(", ")", "|", ",", "=", "!=", ">", ">=", "<", "<=", "~", "!~", "!", "\"", "'", "`", "\\", " ", "and", "or", "not", "in",
        "contains", "exists", "missing", "where", "stats", "sort", "head", "limit", "fields", "by", "as", "asc", "desc", "count", "p95",
        "avg", "level", "@level", "ms", "user", "1", "-3", "2.5", "é", "日本", "\u{0}", "\n",
    ];
    let d = data();
    // xorshift: deterministic pseudo-random generator, no dependencies
    let mut x: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for _ in 0..4000 {
        let n = (next() % 14) as usize;
        let mut q = String::new();
        for _ in 0..n {
            q.push_str(alphabet[(next() % alphabet.len() as u64) as usize]);
            if next() % 3 != 0 {
                q.push(' ');
            }
        }
        if let Ok(p) = ql::parse(&q) {
            let _ = ql::run(&p, &mut d.iter().map(|(r, s)| (r, *s)));
        } else {
            let e = ql::parse(&q).unwrap_err();
            assert!(e.offset + e.len <= q.len(), "{q:?}: {e:?}");
            assert!(q.is_char_boundary(e.offset) && q.is_char_boundary(e.offset + e.len), "{q:?}: {e:?}");
        }
        for cut in 0..=q.len() {
            let _ = ql::completions(&q, cut, &[]);
        }
    }
    // every prefix of a rich query, and a cursor beyond the end or inside a character
    let rich = "level >= warning and message ~ \"t.*out\" or user in (a, \"b\") and not exists(`x y`) | where ms > 1 | stats count(), p95(ms) as p by source, user | sort p desc | head 5 | fields source, p é";
    for end in (0..=rich.len()).filter(|i| rich.is_char_boundary(*i)) {
        let _ = ql::parse(&rich[..end]);
    }
    let _ = ql::completions("é", 1, &[]);
    let _ = ql::completions("é", 99, &[]);
}

fn labels(text: &str, names: &[&str]) -> Vec<String> {
    let names: Vec<String> = names.iter().map(|s| s.to_string()).collect();
    ql::completions(text, text.len(), &names).into_iter().map(|c| c.label).collect()
}

fn has(v: &[String], l: &str) -> bool {
    v.iter().any(|x| x == l)
}

#[test]
fn completions_at_the_start_offer_fields_and_keywords() {
    let v = labels("", &["ms", "odd name"]);
    for l in [
        "@level",
        "level",
        "message",
        "source",
        "ms",
        "`odd name`",
        "not",
        "exists",
        "missing",
    ] {
        assert!(has(&v, l), "{l} in {v:?}");
    }
    let c = ql::completions("", 0, &["ms".into()]);
    assert!(c
        .iter()
        .any(|c| c.label == "ms" && c.kind == CompletionKind::Field && c.insert == "ms"));
    assert!(c
        .iter()
        .any(|c| c.label == "exists" && c.kind == CompletionKind::Function && c.insert == "exists("));
    assert!(c.iter().any(|c| c.label == "not" && c.kind == CompletionKind::Keyword));
    // prefix filtering of the word being typed
    assert_eq!(labels("lev", &[]), vec!["level"]);
    assert_eq!(labels("LEV", &[]), vec!["level"]);
    assert_eq!(labels("@m", &[]), vec!["@message"]);
    assert!(labels("zzz", &[]).is_empty());
    // after and/or/not/( the same set again
    for t in ["level = error and ", "ms > 1 or ", "not ", "(", "a and (", "level = error AND "] {
        assert!(has(&labels(t, &["ms"]), "ms"), "{t}");
    }
    // duplicates are removed
    let dup = ql::completions("", 0, &["level".into()]);
    assert_eq!(dup.iter().filter(|c| c.label == "level").count(), 1);
}

#[test]
fn completions_after_a_field_offer_operators() {
    let v = labels("level ", &[]);
    for l in [
        "=",
        "!=",
        ">",
        ">=",
        "<",
        "<=",
        "~",
        "!~",
        "contains",
        "!contains",
        "in",
        "and",
        "or",
    ] {
        assert!(has(&v, l), "{l} in {v:?}");
    }
    assert!(ql::completions("ms ", 3, &[])
        .iter()
        .any(|c| c.label == "~" && c.kind == CompletionKind::Operator));
    assert!(has(&labels("`odd name` ", &[]), "="));
    assert_eq!(labels("level con", &[]), vec!["contains"]);
    assert!(has(&labels("ms > 1 | where user ", &[]), "!="));
}

#[test]
fn completions_for_values() {
    for t in [
        "level = ",
        "level >= ",
        "level != ",
        "@level < ",
        "level contains ",
        "level in (",
        "level in (warning, ",
    ] {
        let v = labels(t, &[]);
        assert!(has(&v, "warning") && has(&v, "fatal"), "{t}: {v:?}");
    }
    assert_eq!(labels("level = w", &[]), vec!["warning"]);
    assert!(ql::completions("level = ", 8, &[]).iter().all(|c| c.kind == CompletionKind::Value));
    assert!(labels("user = ", &[]).is_empty());
    assert!(labels("user contains ", &[]).is_empty());
    assert!(labels("user in (", &[]).is_empty());
    assert!(labels("user in (a, ", &[]).is_empty());
    assert!(labels("in (", &[]).is_empty());
    assert!(labels("a, ", &[]).is_empty());
    assert_eq!(labels("user in ", &[]), vec!["("]);
    assert_eq!(labels("user !", &[]), vec!["contains"]);
}

#[test]
fn completions_after_a_complete_term() {
    for t in [
        "level = error ",
        "user = \"bob\" ",
        "(a) ",
        "exists(x) ",
        "ms in (1, 2) ",
        "\"text\" ",
        "user contains bob ",
    ] {
        let v = labels(t, &[]);
        assert!(has(&v, "and") && has(&v, "or") && has(&v, "|"), "{t}: {v:?}");
        assert!(!has(&v, "="), "{t}");
    }
}

#[test]
fn completions_inside_exists_offer_fields() {
    assert!(has(&labels("exists(", &["ms"]), "ms"));
    assert!(has(&labels("missing(", &[]), "message"));
}

#[test]
fn completions_after_pipe_offer_stages() {
    for t in ["a | ", "a|", "level = error | "] {
        let c = ql::completions(t, t.len(), &[]);
        let l: Vec<&str> = c.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(l, vec!["where", "stats", "sort", "head", "limit", "fields"], "{t}");
        assert!(c.iter().all(|c| c.kind == CompletionKind::Keyword));
    }
    assert_eq!(labels("a | st", &[]), vec!["stats"]);
    assert_eq!(labels("a | stats count() | s", &[]), vec!["stats", "sort"]);
}

#[test]
fn completions_in_stats() {
    let c = ql::completions("a | stats ", 10, &[]);
    for l in ["count", "count_distinct", "avg", "min", "max", "sum", "p50", "p95", "p99"] {
        assert!(c.iter().any(|c| c.label == l && c.kind == CompletionKind::Function), "{l}");
    }
    assert!(c.iter().any(|c| c.label == "count" && c.insert == "count()"));
    assert!(c.iter().any(|c| c.label == "p95" && c.insert == "p95("));
    assert!(has(&labels("| stats count(), ", &[]), "avg"));
    assert!(has(&labels("| stats avg(", &["ms"]), "ms"));
    assert!(has(&labels("| stats count() ", &[]), "by") && has(&labels("| stats count() ", &[]), "as"));
    assert!(has(&labels("| stats count() as n ", &[]), "by"));
    assert!(labels("| stats count() as ", &[]).is_empty());
    assert!(has(&labels("| stats count() by ", &["ms"]), "ms"));
    assert!(has(&labels("| stats count() by user, ", &["ms"]), "ms"));
    assert!(labels("| stats count() by user ", &[]).is_empty());
    assert_eq!(labels("| stats a", &[]), vec!["avg"]);
}

#[test]
fn completions_in_other_stages() {
    assert!(has(&labels("| sort ", &["ms"]), "ms"));
    assert!(has(&labels("| sort ms, ", &["ms"]), "level"));
    assert_eq!(labels("| sort ms ", &[]), vec!["asc", "desc"]);
    assert_eq!(labels("| sort ms d", &[]), vec!["desc"]);
    assert!(labels("| sort ms desc ", &[]).is_empty());
    assert!(has(&labels("| fields ", &["ms"]), "ms"));
    assert!(has(&labels("| fields ms, ", &[]), "level"));
    assert!(labels("| fields ms ", &[]).is_empty());
    assert!(labels("| head ", &[]).is_empty());
    assert!(labels("| limit 5 ", &[]).is_empty());
    assert!(has(&labels("| where ", &[]), "level"));
    assert!(has(&labels("| where ms > 1 and ", &[]), "user") || has(&labels("| where ms > 1 and ", &[]), "source"));
    assert!(has(&labels("| ( ", &[]), "level"));
}

#[test]
fn completions_are_empty_in_unfinished_strings_and_clamp_the_cursor() {
    assert!(labels("a = \"op", &[]).is_empty());
    assert!(labels("a = `op", &[]).is_empty());
    let c = ql::completions("level = ", 1000, &[]);
    assert!(c.iter().any(|c| c.label == "error"));
    // cursor in the middle: only the text before it counts
    let c = ql::completions("level = error | head 3", 8, &[]);
    assert!(c.iter().any(|c| c.label == "warning"));
    // a cursor inside a multi-byte character moves back to the boundary
    assert!(!ql::completions("é", 1, &[]).is_empty());
    let json = serde_json::to_value(ql::completions("lev", 3, &[])).unwrap();
    assert_eq!(json, json!([{"label": "level", "kind": "field", "insert": "level"}]));
}
