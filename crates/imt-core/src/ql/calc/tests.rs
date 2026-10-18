use super::*;
use crate::model::Level;
use serde_json::json;

fn rec() -> Record {
    let mut r = Record::new(42);
    r.level = Level::Warning;
    r.message = "Slow request".into();
    r.template = Some("Slow {Path}".into());
    for (k, v) in [
        ("ElapsedMs", json!(1234)),
        ("Ratio", json!(0.5)),
        ("Text", json!("12.5")),
        ("Name", json!("Web01")),
        ("Flag", json!(true)),
        ("Nothing", json!(null)),
        ("Obj", json!({"a": 1})),
        ("log.level", json!("warn")),
        ("odd name", json!(7)),
        ("Zero", json!(0)),
        ("Big", json!(1e307)),
    ] {
        r.fields.insert(k.into(), v);
    }
    r
}

fn val(text: &str) -> Option<Value> {
    let e = CalcExpr::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"));
    e.eval_json(&rec(), "app.log")
}

fn err(text: &str) -> QlError {
    CalcExpr::parse(text).expect_err(text)
}

#[test]
fn arithmetic_and_precedence() {
    assert_eq!(val("ElapsedMs / 1000"), Some(json!(1.234)));
    assert_eq!(val("1 + 2 * 3"), Some(json!(7)));
    assert_eq!(val("(1 + 2) * 3"), Some(json!(9)));
    assert_eq!(val("10 - 4 - 3"), Some(json!(3)));
    assert_eq!(val("7 % 4"), Some(json!(3)));
    assert_eq!(val("-ElapsedMs + 4"), Some(json!(-1230)));
    assert_eq!(val("- -3"), Some(json!(3)));
    assert_eq!(val("+4"), Some(json!(4)));
    assert_eq!(val("2*-3"), Some(json!(-6)));
    assert_eq!(val("1e3 + 2.5E-1 + .5"), Some(json!(1000.75)));
    assert_eq!(val("Ratio * 3"), Some(json!(1.5)));
    assert_eq!(val("Text * 2"), Some(json!(25)), "numeric text is a number in arithmetic");
    assert_eq!(val("@line + 1"), Some(json!(43)));
    assert_eq!(val("`odd name` * 2"), Some(json!(14)));
}

#[test]
fn nulls_mismatches_and_division_by_zero() {
    for e in [
        "Missing + 1",
        "Nothing * 2",
        "ElapsedMs / 0",
        "ElapsedMs % Zero",
        "Name * 2",
        "Flag + 1",
        "-Name",
        "Big * Big",
        "null",
        "not Name",
        "Name and true",
        "false or 3",
        "3 or true",
        "Missing = 1",
        "1 < Missing",
    ] {
        assert_eq!(val(e), None, "{e}");
    }
}

#[test]
fn strings_and_concatenation() {
    assert_eq!(val("Name + \"-\" + 'x'"), Some(json!("Web01-x")));
    assert_eq!(val("Name + ElapsedMs"), Some(json!("Web011234")));
    assert_eq!(val("\"r=\" + Ratio"), Some(json!("r=0.5")));
    assert_eq!(val("\"\" + Flag"), Some(json!("true")));
    assert_eq!(val("'' + false"), Some(json!("false")));
    assert_eq!(val("level + ':' + @message"), Some(json!("Warning:Slow request")));
    assert_eq!(val("source"), Some(json!("app.log")));
    assert_eq!(val("@template"), Some(json!("Slow {Path}")));
    assert_eq!(val("@exception"), None);
    assert_eq!(val("log.level"), Some(json!("warn")));
    assert_eq!(val("Obj"), Some(json!("{\"a\":1}")));
    assert_eq!(val("'' + 1e20"), Some(json!("100000000000000000000")));
}

#[test]
fn comparisons_and_logic() {
    assert_eq!(val("ElapsedMs > 1000"), Some(json!(true)));
    assert_eq!(val("ElapsedMs >= 1234 and ElapsedMs <= 1234"), Some(json!(true)));
    assert_eq!(val("ElapsedMs < 1000 or Name = 'Web01'"), Some(json!(true)));
    assert_eq!(val("Name == \"Web01\""), Some(json!(true)));
    assert_eq!(val("Name != 'Web01'"), Some(json!(false)));
    assert_eq!(val("Text = 12.5"), Some(json!(true)), "numeric text compares as a number");
    assert_eq!(val("'10' > '9'"), Some(json!(true)));
    assert_eq!(val("'b' > 'a'"), Some(json!(true)));
    assert_eq!(val("Flag = true"), Some(json!(true)));
    assert_eq!(val("Flag > false"), Some(json!(true)));
    assert_eq!(val("Flag = 'true'"), Some(json!(true)));
    assert_eq!(val("not ElapsedMs > 2000"), Some(json!(true)));
    assert_eq!(val("!Flag"), Some(json!(false)));
    assert_eq!(val("NOT (1 = 1) OR 2 < 1"), Some(json!(false)));
    // Three-valued logic: a decisive operand wins over null.
    assert_eq!(val("false and Missing = 1"), Some(json!(false)));
    assert_eq!(val("Missing = 1 and false"), Some(json!(false)));
    assert_eq!(val("Missing = 1 and true"), None);
    assert_eq!(val("true or Missing = 1"), Some(json!(true)));
    assert_eq!(val("Missing = 1 or true"), Some(json!(true)));
    assert_eq!(val("Missing = 1 or false"), None);
    assert_eq!(val("false or false"), Some(json!(false)));
    assert_eq!(val("true and true"), Some(json!(true)));
    assert_eq!(val("true and 'x'"), None);
}

#[test]
fn functions() {
    let cases: &[(&str, Option<Value>)] = &[
        ("round(ElapsedMs / 1000)", Some(json!(1))),
        ("round(ElapsedMs / 1000, 2)", Some(json!(1.23))),
        ("round(1234, -2)", Some(json!(1200))),
        ("round(2.5)", Some(json!(3))),
        ("round(Big, 2)", Some(json!(1e307))),
        ("round(1, Name)", None),
        ("round(Name)", None),
        ("floor(-1.5)", Some(json!(-2))),
        ("ceil(1.2)", Some(json!(2))),
        ("abs(-3)", Some(json!(3))),
        ("abs(Missing)", None),
        ("min(3, ElapsedMs, -1)", Some(json!(-1))),
        ("max(3, ElapsedMs, -1)", Some(json!(1234))),
        ("MAX(2)", Some(json!(2))),
        ("min(1, Missing)", None),
        ("len(Name)", Some(json!(5))),
        ("len('héllo')", Some(json!(5))),
        ("len(ElapsedMs)", Some(json!(4))),
        ("len(Missing)", None),
        ("lower(Name)", Some(json!("web01"))),
        ("upper(Name)", Some(json!("WEB01"))),
        ("upper(Missing)", None),
        ("coalesce(Missing, Nothing, Name)", Some(json!("Web01"))),
        ("coalesce(Missing)", None),
        ("if(ElapsedMs > 1000, 'slow', 'ok')", Some(json!("slow"))),
        ("if(ElapsedMs > 5000, 'slow', 'ok')", Some(json!("ok"))),
        ("if(Missing > 1, 'a', 'b')", Some(json!("b"))),
        ("if(Name, 'a', 'b')", None),
        ("substr(Name, 0, 3)", Some(json!("Web"))),
        ("substr(Name, 3)", Some(json!("01"))),
        ("substr(Name, -2)", Some(json!("01"))),
        ("substr(Name, -20, 2)", Some(json!("We"))),
        ("substr(Name, 10)", Some(json!(""))),
        ("substr(Name, 1, -1)", None),
        ("substr(Name, 'x')", None),
        ("substr(Missing, 1)", None),
        ("to_number(Text) + 1", Some(json!(13.5))),
        ("to_number(' 7 ')", Some(json!(7))),
        ("to_number(Flag)", Some(json!(1))),
        ("to_number(false)", Some(json!(0))),
        ("to_number(Name)", None),
    ];
    for (e, want) in cases {
        assert_eq!(&val(e), want, "{e}");
    }
}

#[test]
fn eval_returns_borrowed_values() {
    let r = rec();
    let e = CalcExpr::parse("Name").unwrap();
    assert_eq!(e.eval(&r, "s"), Val::Str(std::borrow::Cow::Borrowed("Web01")));
    assert_eq!(Val::Num(f64::NAN).into_json(), None);
    assert_eq!(Val::Num(0.25).into_json(), Some(json!(0.25)));
    assert_eq!(Val::Null.text(), "");
}

#[test]
fn concatenation_is_bounded() {
    let mut r = rec();
    r.fields.insert("Huge".into(), json!("x".repeat(MAX_CALC_TEXT)));
    let e = CalcExpr::parse("Huge + 'y'").unwrap();
    assert_eq!(e.eval_json(&r, "s"), None);
    let e = CalcExpr::parse("Huge + ''").unwrap();
    assert!(e.eval_json(&r, "s").is_some());
}

#[test]
fn errors_point_at_the_problem() {
    let cases: &[(&str, &str, usize, usize)] = &[
        ("", "expected an expression", 0, 0),
        ("   ", "expected an expression", 0, 3),
        ("1 +", "expected a value", 3, 0),
        ("1 + )", "expected a value, found ')'", 4, 1),
        ("(1 + 2", "unclosed '('", 0, 1),
        ("1 + 2)", "unmatched ')'", 5, 1),
        ("1 2", "unexpected '2'", 2, 1),
        ("foo(1)", "unknown function 'foo'", 0, 3),
        ("round()", "round() takes 1 to 2 arguments, got 0", 0, 5),
        ("abs(1, 2)", "abs() takes 1 argument, got 2", 0, 3),
        ("if(1, 2)", "if() takes 3 arguments, got 2", 0, 2),
        ("min()", "min() takes at least 1 argument, got 0", 0, 3),
        ("abs(1 2)", "unclosed '(' (expected ')' or ',')", 3, 1),
        ("abs(1,)", "expected a value, found ')'", 6, 1),
        ("1.2.3 + 1", "invalid number '1.2.3'", 0, 5),
        ("1 + and", "expected a value, found 'and'", 4, 3),
        ("a ~ 'x'", "regex matching", 2, 1),
        ("a < b < c", "comparisons cannot be chained", 6, 1),
        ("'open", "unterminated string", 0, 5),
        ("a | b", "unexpected '|'", 2, 1),
        ("a, b", "unexpected ','", 1, 1),
    ];
    for (text, msg, offset, len) in cases {
        let e = err(text);
        assert!(e.message.contains(msg), "{text}: {}", e.message);
        assert_eq!((e.offset, e.len), (*offset, *len), "{text}: {}", e.message);
    }
    assert_eq!(err("lowr(Name)").suggestions[0], "lower");
}

#[test]
fn limits() {
    let long = format!("1{}", " + 1".repeat(MAX_CALC_LEN));
    let e = err(&long);
    assert!(e.message.contains("too long"), "{}", e.message);
    assert_eq!(e.offset, MAX_CALC_LEN);

    let deep_parens = format!("{}1{}", "(".repeat(100), ")".repeat(100));
    assert!(err(&deep_parens).message.contains("nested too deeply"));
    let deep_neg = format!("{}x", "-".repeat(100));
    assert!(err(&deep_neg).message.contains("nested too deeply"));
    let deep_not = format!("{}true", "not ".repeat(100));
    assert!(err(&deep_not).message.contains("nested too deeply"));
    let deep_calls = format!("{}1{}", "abs(".repeat(100), ")".repeat(100));
    assert!(err(&deep_calls).message.contains("nested too deeply"));
    // A long flat chain builds a deep tree too; it is refused at the operator that overflows.
    let chain = format!("a{}", " + a".repeat(MAX_CALC_DEPTH + 1));
    let e = err(&chain);
    assert!(e.message.contains("at most 64 levels"), "{}", e.message);
    assert_eq!(&chain[e.offset..e.offset + e.len], "+");
    for (op, kw) in [("and", "and"), ("or", "or"), ("*", "*")] {
        let chain = format!("true{}", format!(" {op} true").repeat(MAX_CALC_DEPTH + 1));
        let e = err(&chain);
        assert_eq!(&chain[e.offset..e.offset + e.len], kw);
    }
    // Within the limits everything parses.
    assert!(CalcExpr::parse(&format!("{}1{}", "(".repeat(20), ")".repeat(20))).is_ok());
    assert!(CalcExpr::parse(&format!("a{}", " + a".repeat(MAX_CALC_DEPTH - 1))).is_ok());
}
