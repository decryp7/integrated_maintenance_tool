//! `step = "script"` end to end: a profile script transforms, enriches and drops events of
//! the gateway sample during import, and failing records are counted in the pipeline stats.

use imt_core::model::{Level, Record};
use imt_core::script::{Script, ScriptError};
use imt_core::{Profile, Workspace};
use proptest::prelude::*;
use std::path::PathBuf;

fn gateway() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples/serilog-json/gateway")
}

const PROFILE: &str = r#"
name = "Script"

[[pipeline]]
step = "script"
script = '''
if fields.Upstream == "web01:8080" { drop(); }
if fields.ElapsedMs > 1000 {
    fields.Slow = true;
    level = "Warning";
}
message = regex_replace(message, "\"web0(\\d):8080\"", "node-$1");
fields.remove("ProcessId");
'''

[[pipeline]]
step = "script"
when = [{ field = "Slow", op = "exists" }]
script = '''
fields.Touched = true;
loop { }
'''
"#;

fn open(profile: &str) -> Workspace {
    let mut ws = Workspace::new(Profile::from_toml(profile).unwrap()).unwrap();
    ws.open(&[gateway()]).unwrap();
    ws
}

#[test]
fn scripts_transform_events_on_import() {
    let plain = open("name = \"Plain\"");
    let from_web01 = plain
        .records()
        .iter()
        .filter(|r| r.fields.get("Upstream").and_then(|v| v.as_str()) == Some("web01:8080"))
        .count();
    let slow = plain
        .records()
        .iter()
        .filter(|r| r.fields.get("Upstream").and_then(|v| v.as_str()) != Some("web01:8080"))
        .filter(|r| r.fields.get("ElapsedMs").and_then(|v| v.as_f64()).is_some_and(|ms| ms > 1000.0))
        .count();
    assert!(
        from_web01 > 10 && slow > 0,
        "the sample has web01 traffic ({from_web01}) and slow requests ({slow})"
    );

    let ws = open(PROFILE);
    assert_eq!(ws.records().len(), plain.records().len() - from_web01);
    assert!(ws.records().iter().all(|r| !r.fields.contains_key("ProcessId")));
    assert!(ws.records().iter().all(|r| !r.message.contains(":8080")), "upstreams are rewritten");
    assert!(ws.records().iter().any(|r| r.message.contains("node-2")));
    let flagged: Vec<_> = ws.records().iter().filter(|r| r.fields.contains_key("Slow")).collect();
    assert_eq!(flagged.len(), slow);
    assert!(flagged.iter().all(|r| r.level == Level::Warning));
    // The runaway second script is stopped on every slow event, which it leaves unchanged.
    assert!(ws.records().iter().all(|r| !r.fields.contains_key("Touched")));

    let stats = ws.summary().pipeline;
    assert_eq!(stats.len(), 2);
    assert_eq!((stats[0].input - stats[0].output, stats[0].errors), (from_web01, 0));
    assert_eq!(
        (stats[1].input, stats[1].output, stats[1].errors),
        (stats[0].output, stats[0].output, slow)
    );
    let json = serde_json::to_value(&stats[1]).unwrap();
    assert_eq!(json["errors"], slow, "the error count reaches the UI");
}

#[test]
fn a_bad_script_is_a_profile_error_with_its_position() {
    let bad = "name = \"Bad\"\n[[pipeline]]\nstep = \"script\"\nscript = '''\nlet x = 1;\nfields.X = x +;\n'''\n";
    let err = format!("{:#}", Workspace::new(Profile::from_toml(bad).unwrap()).err().unwrap());
    assert!(
        err.contains("pipeline step 1 (script): `script` syntax error at line 2, column 15"),
        "{err}"
    );
}

#[test]
fn the_documented_example_compiles() {
    // Windows checkouts may have CRLF line endings.
    let doc = imt_core::profile::DEFAULT_PROFILE_TOML.replace("\r\n", "\n");
    let start = doc
        .find("# step = \"script\"\n# script = '''")
        .expect("the profile schema documents scripts");
    let block: String = doc[start..]
        .lines()
        .skip(2)
        .take_while(|l| *l != "# '''")
        .map(|l| format!("{}\n", l.strip_prefix("# ").unwrap_or(l)))
        .collect();
    let script = Script::compile(&block).unwrap();
    let mut r = record();
    r.message = "login password=hunter2".into();
    r.fields.insert("ElapsedMs".into(), 6000.into());
    assert!(script.runner("s").apply(&mut r).is_ok());
    assert_eq!((r.message.as_str(), r.level), ("login password=***", Level::Warning));
}

fn record() -> Record {
    let mut r = Record::new(1);
    r.ts = Some(1);
    r.message = "m".into();
    r.fields.insert("A".into(), 1.into());
    r
}

/// Fragments that make syntactically plausible scripts more likely than random text.
fn fragment() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("message".to_string()),
        Just("fields.A".to_string()),
        Just("level".to_string()),
        Just("timestamp".to_string()),
        Just("drop()".to_string()),
        Just("loop { }".to_string()),
        Just(" = ".to_string()),
        Just(" + ".to_string()),
        Just(";".to_string()),
        Just("if true {".to_string()),
        Just("}".to_string()),
        Just("\"x\"".to_string()),
        Just("()".to_string()),
        Just("1".to_string()),
        Just("regex_replace(message, \"m\", \"$0$0\")".to_string()),
        "[ -~\n]{0,12}",
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn arbitrary_text_never_panics_the_compiler(text in "\\PC{0,200}") {
        match Script::compile(&text) {
            Ok(_) | Err(ScriptError::Empty | ScriptError::Syntax { .. }) => {}
            Err(e) => prop_assert!(false, "unexpected {e:?}"),
        }
    }

    #[test]
    fn arbitrary_scripts_never_panic(parts in prop::collection::vec(fragment(), 0..16)) {
        if let Ok(s) = Script::compile(&parts.concat()) {
            let mut r = record();
            let before = r.clone();
            let mut runner = s.runner("src");
            if runner.apply(&mut r).is_err() {
                prop_assert_eq!(r.message, before.message);
                prop_assert_eq!(r.fields, before.fields);
            }
        }
    }
}
