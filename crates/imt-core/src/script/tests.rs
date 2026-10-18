use super::value::{map_to_fields, to_dynamic, to_json, MAX_DEPTH};
use super::*;
use serde_json::{json, Value};

fn rec() -> Record {
    let mut r = Record::new(3);
    r.ts = Some(1_000);
    r.message = "login password=hunter2 user=ann".into();
    r.fields.insert("ElapsedMs".into(), json!(6000));
    r.fields.insert("User".into(), json!("ann"));
    r.fields.insert("Ratio".into(), json!(0.25));
    r.fields.insert("Ok".into(), json!(true));
    r.fields.insert("Nothing".into(), Value::Null);
    r.fields
        .insert("Nested".into(), json!({"a": [1, 2.5, null, "x"], "b": {"c": false}}));
    r.fields.insert("Big".into(), json!(u64::MAX));
    r
}

/// Runs `script` on [`rec`] from source "web01.log".
fn run(script: &str) -> (Result<Verdict, ScriptError>, Record) {
    let s = Script::compile(script).unwrap_or_else(|e| panic!("{script}: {e}"));
    let mut r = rec();
    let v = s.runner("web01.log").apply(&mut r);
    (v, r)
}

fn runtime_error(script: &str) -> String {
    let (v, r) = run(script);
    let before = rec();
    assert_eq!(
        (r.message.as_str(), r.level, r.ts),
        (before.message.as_str(), before.level, before.ts),
        "{script}"
    );
    assert_eq!(r.fields, before.fields, "a failing script leaves the record unchanged: {script}");
    match v {
        Err(ScriptError::Runtime(m)) => m,
        other => panic!("{script}: expected a runtime error, got {other:?}"),
    }
}

fn syntax_error(script: &str) -> ScriptError {
    Script::compile(script)
        .err()
        .unwrap_or_else(|| panic!("{script} should not compile"))
}

#[test]
fn reads_and_changes_the_record() {
    let (v, r) = run(r#"
        if fields.ElapsedMs > 5000 { fields.Slow = true; level = "Warning"; }
        if message.contains("password=") { message = regex_replace(message, "password=\\S+", "password=***"); }
        timestamp += 500;
        fields.remove("User");
        fields.From = source;
        fields["odd name"] = #{ list: [1, 2.5, (), "s", 'c'], yes: true };
        fields.Ratio *= 4;
    "#);
    assert_eq!(v, Ok(Verdict::Keep));
    assert_eq!(r.message, "login password=*** user=ann");
    assert_eq!(r.level, Level::Warning);
    assert_eq!(r.ts, Some(1_500));
    assert!(!r.fields.contains_key("User"));
    assert_eq!(r.fields["Slow"], true);
    assert_eq!(r.fields["From"], "web01.log");
    assert_eq!(r.fields["odd name"], json!({"list": [1, 2.5, null, "s", "c"], "yes": true}));
    assert_eq!(r.fields["Ratio"], 1.0);
    // Existing fields keep their order; new ones follow sorted by name.
    let keys: Vec<&str> = r.fields.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        ["ElapsedMs", "Ratio", "Ok", "Nothing", "Nested", "Big", "From", "Slow", "odd name"]
    );
}

#[test]
fn unchanged_fields_round_trip() {
    let (v, r) = run("let unused = fields.len();");
    assert_eq!(v, Ok(Verdict::Keep));
    let before = rec();
    assert_eq!(
        (r.message.as_str(), r.level, r.ts),
        (before.message.as_str(), before.level, before.ts)
    );
    for (k, want) in &before.fields {
        if k == "Big" {
            // Integers beyond i64 are floats in scripts.
            assert_eq!(r.fields[k].as_f64(), want.as_f64());
        } else {
            assert_eq!(&r.fields[k], want, "{k}");
        }
    }
    // The script sees JSON-like types.
    let (_, r) = run(
        "fields.T = [type_of(fields.ElapsedMs), type_of(fields.Ratio), type_of(fields.Ok), type_of(fields.Nothing), \
         type_of(fields.Nested), type_of(fields.Nested.a), type_of(fields.User), type_of(timestamp), type_of(level)];",
    );
    assert_eq!(
        r.fields["T"],
        json!(["i64", "f64", "bool", "()", "map", "array", "string", "i64", "string"])
    );
}

#[test]
fn timestamps_and_levels() {
    let (_, r) = run("timestamp = ();");
    assert_eq!(r.ts, None);
    let (_, r) = run("timestamp = 1.6;");
    assert_eq!(r.ts, Some(2));
    let (_, r) = run("level = \"err\";");
    assert_eq!(r.level, Level::Error);
    let s = Script::compile("fields.HadTime = timestamp != ();").unwrap();
    let mut r = Record::new(1);
    assert_eq!(s.runner("x").apply(&mut r), Ok(Verdict::Keep));
    assert_eq!(r.fields["HadTime"], false);
}

#[test]
fn drop_removes_the_record() {
    assert_eq!(
        run("if source == \"web01.log\" { drop(); } message = \"kept\";").0,
        Ok(Verdict::Drop)
    );
    assert_eq!(run("fn gone() { drop() } gone();").0, Ok(Verdict::Drop), "also from a function");
    assert_eq!(
        run("try { drop(); } catch { message = \"caught\"; }").0,
        Ok(Verdict::Drop),
        "drop cannot be caught"
    );
    let (v, r) = run("if source == \"other\" { drop(); }");
    assert_eq!(v, Ok(Verdict::Keep), "keep is implicit");
    assert_eq!(r.message, rec().message);
}

#[test]
fn exit_ends_the_script_and_keeps_changes() {
    let (v, r) = run("message = \"short\"; exit(); message = \"never\";");
    assert_eq!(v, Ok(Verdict::Keep));
    assert_eq!(r.message, "short");
}

#[test]
fn limits_stop_runaway_scripts() {
    assert!(runtime_error("loop { }").contains("Too many operations"));
    assert!(runtime_error("let i = 0; while true { i += 1; }").contains("Too many operations"));
    assert!(runtime_error("fn f(n) { f(n + 1) } f(0);").contains("Stack overflow"));
    assert!(runtime_error("let s = message; loop { s += s; }").contains("Length of string"));
    assert!(runtime_error("let a = []; loop { a.push(1); }").contains("Size of array"));
    let e = runtime_error("let m = #{}; for i in 0..10001 { m[`${i}`] = 0; } fields.L = m.len();");
    assert!(e.contains("Size of object map"), "{e}");
    // A string built by a regex replacement is checked while it grows.
    let e = runtime_error(&format!(
        "let t = \"\"; t.pad(70000, 'a'); message = regex_replace(t, \"\", \"{}\");",
        "b".repeat(64)
    ));
    assert!(e.contains("longer than"), "{e}");
}

#[test]
fn runtime_errors_leave_the_record_unchanged() {
    for (script, want) in [
        ("message = 5;", "`message` must be a string, not i64"),
        ("level = 3;", "`level` must be a string"),
        ("level = \"loud\";", "\"loud\" is not a level"),
        ("timestamp = \"soon\";", "`timestamp` must be milliseconds or ()"),
        ("timestamp = 1.0 / 0.0;", "`timestamp` must be"),
        ("fields = 1;", "`fields` must be a map, not i64"),
        ("source = \"x\";", "constant"),
        ("message = 1 / 0;", "Division by zero"),
        ("sleep(1);", "not available"),
        ("let message = \"x\";", "already defined"),
        ("for i in 0..10001 { fields[`f${i}`] = i; }", "`fields` has more than 10000 entries"),
        ("sleep(0.5);", "not available"),
        ("throw \"bad record\";", "bad record"),
        ("fields.X = 1; message.nope();", "Function not found"),
        ("regex_match(message, \"(\");", "invalid regex"),
        (
            "message.replace(\"a\", \"b\"); message = message.replace(\"a\", \"b\");",
            "`message` must be a string, not ()",
        ),
    ] {
        let e = runtime_error(script);
        assert!(e.contains(want), "{script}: {e}");
    }
}

#[test]
fn syntax_errors_name_line_and_column() {
    assert_eq!(syntax_error(""), ScriptError::Empty);
    assert_eq!(syntax_error("  \n "), ScriptError::Empty);
    let long = format!("let a = 1;{}", " ".repeat(limits::MAX_SCRIPT_LEN));
    assert!(matches!(syntax_error(&long), ScriptError::TooLong(n) if n == long.len()));
    assert!(syntax_error(&long).to_string().starts_with("is too long"));
    let e = syntax_error("let x = 1;\nmessage = ;");
    let ScriptError::Syntax { line, column, .. } = &e else {
        panic!("{e:?}")
    };
    assert_eq!((*line, *column), (2, 11), "{e}");
    assert!(e.to_string().starts_with("syntax error at line 2, column 11: "), "{e}");
    // Variables must exist: a typo is caught when the profile loads.
    assert!(syntax_error("mesage = \"x\";").to_string().contains("mesage"));
    // No eval, sleep, modules or shadowing of the record variables.
    for bad in ["eval(\"1\");", "import \"x\" as y;", "if true {"] {
        assert!(matches!(syntax_error(bad), ScriptError::Syntax { .. }), "{bad}");
    }
    let deep = format!("let a = {}1{};", "(".repeat(200), ")".repeat(200));
    assert!(matches!(syntax_error(&deep), ScriptError::Syntax { .. }));
}

#[test]
fn regex_functions() {
    let (_, r) = run(
        "fields.Has = regex_match(message, \"user=\\\\w+\"); fields.User2 = regex_capture(message, \"user=(\\\\w+)\", 1); \
         fields.Whole = message.regex_capture(\"login\", 0); fields.NoGroup = regex_capture(message, \"login\", 3); \
         fields.Negative = regex_capture(message, \"login\", -1); fields.NoMatch = regex_capture(message, \"zzz\", 0); \
         fields.Upper = regex_replace(\"a1b22\", \"(\\\\d+)\", \"<$1>\");",
    );
    assert_eq!(r.fields["Has"], true);
    assert_eq!(r.fields["User2"], "ann");
    assert_eq!(r.fields["Whole"], "login");
    for k in ["NoGroup", "Negative", "NoMatch"] {
        assert_eq!(r.fields[k], Value::Null, "{k}");
    }
    assert_eq!(r.fields["Upper"], "a<1>b<22>");
    // More patterns than the cache holds still work.
    let (_, r) = run("let n = 0; for i in 0..100 { if regex_match(`a${i}`, `^a${i}$`) { n += 1; } } fields.N = n;");
    assert_eq!(r.fields["N"], 100);
}

#[test]
fn values_without_a_json_form() {
    let (_, r) = run(
        "fields.F = Fn(\"f\"); fields.R = 1..3; fields.N = 0.0 / 0.0; let x = 1; let c = || x; x = 2; fields.Shared = x; \
         let deep = 1; for i in 0..200 { deep = [deep]; } fields.Deep = deep;",
    );
    assert_eq!(r.fields["F"], "Fn(f)");
    assert_eq!(r.fields["R"], "1..3");
    assert_eq!(r.fields["N"], Value::Null);
    assert_eq!(r.fields["Shared"], 2);
    let mut v = &r.fields["Deep"];
    let mut depth = 0;
    while let Value::Array(a) = v {
        v = &a[0];
        depth += 1;
    }
    assert_eq!((depth, v), (MAX_DEPTH, &Value::Null), "nesting is cut at MAX_DEPTH");
}

#[test]
fn deep_json_is_cut_when_converted() {
    let mut v = json!(1);
    for _ in 0..MAX_DEPTH + 10 {
        v = json!({ "a": v });
    }
    let mut back = to_json(to_dynamic(&v, 0), 0);
    let mut depth = 0;
    while let Value::Object(o) = back {
        back = o["a"].clone();
        depth += 1;
    }
    assert_eq!((depth, back), (MAX_DEPTH, Value::Null));
    assert!(map_to_fields(&rec().fields, rhai::Map::new()).is_empty());
    assert!(format!("{:?}", Script::compile("1").unwrap()).starts_with("Script"));
}

#[test]
fn scope_is_reused_across_records() {
    let s = Script::compile("let n = fields.N ?? 0; fields.N = n + 1;").unwrap();
    let mut runner = s.runner("f");
    let mut r = Record::new(1);
    for want in 1..=3 {
        assert_eq!(runner.apply(&mut r), Ok(Verdict::Keep));
        assert_eq!(r.fields["N"], want, "top-level variables do not leak into the next record");
    }
}
