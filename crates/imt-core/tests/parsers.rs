//! The built-in parsers (ADR 0026) end to end: each reads its sample (as the Settings ▸ Parsers preview does),
//! keeps the details of its format, and the library chooses the right parser per file of /samples.

use imt_core::api::ParsePreviewRequest;
use imt_core::model::Level;
use imt_core::parser::builtin::{self, BuiltinFile};
use imt_core::parser::{ParserSetup, SetupParser};
use imt_core::{PluginRegistry, Profile, Workspace};
use std::path::PathBuf;

fn samples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples")
}

/// Formats a plugin reads (their built-in parsers name the plugin's importer): not read without plugins.
const PLUGIN_FORMATS: [&str; 5] = ["log4net-xml", "iis-w3c", "access-combined", "syslog", "windows-event"];

fn preview(ws: &Workspace, f: &BuiltinFile) -> imt_core::api::ParsePreview {
    ws.preview_parse(&ParsePreviewRequest {
        toml: f.toml.to_string(),
        profile: String::new(),
        sample: f.sample.to_string(),
        file_name: f.sample_name.to_string(),
        limit: 100,
    })
}

fn file(id: &str) -> &'static BuiltinFile {
    builtin::file(id).unwrap()
}

#[test]
fn every_text_builtin_parses_its_sample() {
    let ws = Workspace::new(Profile::default_profile()).unwrap();
    for (f, p) in builtin::FILES.iter().zip(builtin::parsers()) {
        if f.sample.is_empty() || PLUGIN_FORMATS.contains(&p.id.as_str()) {
            continue;
        }
        let pv = preview(&ws, f);
        assert_eq!(pv.error, None, "{}", p.id);
        assert_eq!(pv.parser.as_deref(), Some(p.id.as_str()));
        assert!(pv.total >= 3, "{}: {} events", p.id, pv.total);
        assert_eq!(pv.skipped, 0, "{}: lines skipped", p.id);
        for r in &pv.records {
            assert!(r.ts.is_some(), "{}: line {} has no timestamp", p.id, r.line);
        }
        let levels: Vec<Level> = pv.records.iter().map(|r| r.level).collect();
        assert!(levels.contains(&Level::Error), "{}: {levels:?}", p.id);
        assert!(levels.iter().any(|l| *l != Level::Error), "{}: {levels:?}", p.id);
    }
}

#[test]
fn builtins_keep_details_of_their_format() {
    let ws = Workspace::new(Profile::default_profile()).unwrap();
    let mel = preview(&ws, file("mel-console"));
    assert!(mel.records[3].exception.as_deref().is_some_and(|x| x.contains("TimeoutException")));
    assert!(mel.records[0].message.contains("Now listening"));
    let pattern = preview(&ws, file("log4net-pattern"));
    assert_eq!(pattern.records[3].fields["Ndc"], "tariffs");
    assert!(pattern.records[2].exception.is_some());
    let apache = preview(&ws, file("apache-error"));
    let lv: Vec<Level> = apache.records.iter().map(|r| r.level).collect();
    assert_eq!(lv, [Level::Information, Level::Warning, Level::Error, Level::Error]);
    assert_eq!(apache.records[2].fields["Module"], "proxy_http");
    assert_eq!(apache.records[2].fields["Tid"], 151);
    assert_eq!(
        apache.records[3].ts,
        Some(1_791_974_490_000),
        "2026-10-14 10:41:30 UTC, the 2.2 layout"
    );
    let logcat = preview(&ws, file("android-logcat"));
    assert_eq!(logcat.records[1].fields["Tag"], "OkHttp");
    assert_eq!(logcat.records[1].message, "Slow response from /api/orders (1500 ms)");
    let spark = preview(&ws, file("hadoop-spark"));
    assert_eq!(spark.records[2].fields["Logger"], "Executor");
    assert!(spark.records[2]
        .exception
        .as_deref()
        .is_some_and(|x| x.starts_with("java.io.IOException")));
    // The older HDFS layout, with a thread number.
    let hdfs = ws.preview_parse(&ParsePreviewRequest {
        toml: file("hadoop-spark").toml.to_string(),
        profile: String::new(),
        sample: "081109 203615 148 INFO dfs.DataNode$PacketResponder: PacketResponder 1 terminating\n\
                 081109 203807 222 WARN dfs.DataNode: Slow BlockReceiver\n"
            .to_string(),
        file_name: "hdfs.log".into(),
        limit: 10,
    });
    assert_eq!(hdfs.records[0].ts, Some(1_226_262_975_000), "2008-11-09 20:36:15 UTC");
    assert_eq!(hdfs.records[1].level, Level::Warning);
    // log4net CSV without a header: date and time in two columns, offset kept.
    let csv = preview(&ws, file("log4net-csv"));
    assert_eq!(csv.importer.as_deref(), Some("csv"));
    assert_eq!(csv.records[0].ts, Some(1_762_556_400_013), "2025-11-08 00:00:00.013+01:00");
    assert_eq!(csv.records[1].level, Level::Warning);
    assert_eq!(csv.records[2].message, "Failed to capture payment for order 88412");
    assert_eq!(csv.records[3].fields["Process"], "BillingAgent");
}

#[test]
fn an_empty_parser_text_previews_with_the_librarys_choice() {
    let ws = Workspace::new(Profile::default_profile()).unwrap();
    let clef = file("serilog-clef");
    let pv = ws.preview_parse(&ParsePreviewRequest {
        toml: String::new(),
        profile: "[[extract]]\nname = \"order\"\nfrom = \"@message\"\nregex = 'order (?P<OrderNo>\\d+)'\n".into(),
        sample: clef.sample.to_string(),
        file_name: clef.sample_name.to_string(),
        limit: 100,
    });
    assert_eq!(pv.parser.as_deref(), Some("serilog-clef"));
    assert!(
        pv.records.iter().any(|r| r.fields.contains_key("OrderNo")),
        "the candidate profile's extractor ran"
    );
    let bad = ws.preview_parse(&ParsePreviewRequest {
        toml: "schema = 1\nid = \"x\"\nname = \"x\"\nversion = \"1.0.0\"\n[format]\nregexes = ['(']\n".into(),
        profile: String::new(),
        sample: "x".into(),
        file_name: "x.log".into(),
        limit: 1,
    });
    assert!(bad.error.is_some_and(|e| e.contains("format.regexes")));
}

#[test]
fn the_library_reads_each_sample_folder_with_its_parser() {
    for (folder, want) in [
        ("serilog-clef", "serilog-clef"),
        ("serilog-json", "serilog-json"),
        ("serilog-text", "serilog-text"),
        ("log4net", "log4net-pattern"),
        ("csv", "generic"),
    ] {
        let mut ws = Workspace::new(Profile::default_profile()).unwrap();
        ws.open(&[samples().join(folder)]).unwrap();
        let s = ws.summary();
        assert!(!s.sources.is_empty() && s.unrecognized.is_empty(), "{folder}: {:?}", s.unrecognized);
        for src in &s.sources {
            assert_eq!(src.parser.as_deref(), Some(want), "{folder}: {}", src.name);
            assert_eq!(src.skipped, 0, "{folder}: {} skipped lines in {}", src.skipped, src.name);
            assert!(src.records > 0 && src.from.is_some(), "{folder}: {}", src.name);
        }
    }
}

#[test]
fn a_mixed_folder_uses_a_parser_per_file_and_a_disabled_parser_is_skipped() {
    let dir = std::env::temp_dir().join(format!("imt-parsers-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for id in ["serilog-clef", "log4net-csv", "nlog", "android-logcat"] {
        let f = file(id);
        std::fs::write(dir.join(f.sample_name), f.sample).unwrap();
    }
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(std::slice::from_ref(&dir)).unwrap();
    let by_name = |ws: &Workspace| -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> = ws
            .sources()
            .iter()
            .map(|s| (s.name.clone(), s.parser.clone().unwrap_or_default()))
            .collect();
        v.sort();
        v
    };
    assert_eq!(
        by_name(&ws),
        [
            ("logcat.txt".to_string(), "android-logcat".to_string()),
            ("nlog.log".into(), "nlog".into()),
            ("orders-log4net.csv".into(), "log4net-csv".into()),
            ("orders.clef".into(), "serilog-clef".into()),
        ]
    );
    // Disabling the CSV parser: its source is stale; read again it falls back to the generic parser.
    let mut setup = ParserSetup::builtins();
    for p in &mut setup.parsers {
        p.enabled = !p.toml.contains("id = \"log4net-csv\"");
    }
    let stale = ws.set_parsers(&setup).unwrap();
    assert_eq!(stale.len(), 1);
    ws.reread(&stale).unwrap();
    let csv = ws.sources().iter().find(|s| s.name == "orders-log4net.csv").unwrap();
    assert_eq!(csv.parser.as_deref(), Some("generic"));
    // A user parser for the CSV with the same rules wins ties against the built-in once enabled again.
    let mut user = builtin::find("log4net-csv").unwrap().clone();
    user.id = "orders-csv".to_string().try_into().unwrap();
    setup.parsers.push(SetupParser {
        toml: user.canonical_toml(),
        enabled: true,
        user: true,
    });
    for p in &mut setup.parsers {
        p.enabled = true;
    }
    let stale = ws.set_parsers(&setup).unwrap();
    assert!(stale.is_empty(), "nothing read with a changed parser");
    ws.reload().unwrap();
    let csv = ws.sources().iter().find(|s| s.name == "orders-log4net.csv").unwrap();
    assert_eq!(csv.parser.as_deref(), Some("orders-csv"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn plugin_formats_need_their_plugin() {
    // Without the Syslog plugin the syslog sample is the generic parser's (as text).
    let mut ws = Workspace::with_registry(Profile::default_profile(), PluginRegistry::new()).unwrap();
    let dir = std::env::temp_dir().join(format!("imt-parsers-plugin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("syslog.log"), file("syslog").sample).unwrap();
    ws.open(std::slice::from_ref(&dir)).unwrap();
    let s = ws.summary();
    assert!(
        s.sources.iter().all(|s| s.parser.as_deref() == Some("generic")),
        "{:?}",
        s.unrecognized
    );
    std::fs::remove_dir_all(&dir).ok();
}
