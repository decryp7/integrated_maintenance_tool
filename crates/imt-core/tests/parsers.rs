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

/// The samples of the parsers that had none (`samples/generate.py`): folder, file and the parser that must read it.
const PARSER_SAMPLES: [(&str, &str, &str); 6] = [
    ("android-logcat", "qa-pixel07/logcat-20261014.txt", "android-logcat"),
    ("hadoop-spark", "analytics01/spark-orders-export.log", "hadoop-spark"),
    ("hadoop-spark", "dn01/hadoop-hdfs-datanode-dn01.log", "hadoop-spark"),
    ("apache-error", "edge02/error.log", "apache-error"),
    ("nlog", "notify01/notify-scheduler-20261014.log", "nlog"),
    ("mel-console", "payments01/payments-adapter-20261014.log", "mel-console"),
];

/// Opens `paths` with the default profile and returns the workspace and each source's records by file name.
fn open_sources(paths: &[PathBuf]) -> (Workspace, Vec<(String, Vec<imt_core::model::Record>)>) {
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(paths).unwrap();
    let s = ws.summary();
    assert!(s.unrecognized.is_empty(), "{:?}", s.unrecognized);
    let by_source = s
        .sources
        .iter()
        .map(|src| {
            let recs = ws.records().iter().filter(|r| r.source == src.id).cloned().collect();
            (src.path.replace('\\', "/"), recs)
        })
        .collect();
    (ws, by_source)
}

fn records_of<'a>(sources: &'a [(String, Vec<imt_core::model::Record>)], suffix: &str) -> &'a [imt_core::model::Record] {
    &sources
        .iter()
        .find(|(p, _)| p.ends_with(suffix))
        .unwrap_or_else(|| panic!("{suffix} not opened"))
        .1
}

#[test]
fn each_parser_sample_is_read_by_its_parser_with_times_levels_and_grouped_lines() {
    for (folder, file, want) in PARSER_SAMPLES {
        let path = samples().join(folder).join(file);
        let lines = std::fs::read_to_string(&path).unwrap().lines().count();
        let (ws, sources) = open_sources(std::slice::from_ref(&path));
        let src = &ws.summary().sources[0];
        assert_eq!(src.parser.as_deref(), Some(want), "{file}");
        assert_eq!(src.skipped, 0, "{file}");
        let recs = records_of(&sources, file);
        assert!(recs.iter().all(|r| r.ts.is_some()), "{file}: an event without a timestamp");
        assert!(recs.windows(2).all(|w| w[0].ts <= w[1].ts), "{file}: events out of order");
        let levels: std::collections::BTreeSet<Level> = recs.iter().map(|r| r.level).collect();
        assert!(levels.len() >= 3 && levels.contains(&Level::Error), "{file}: {levels:?}");
        // Continuation lines (stack traces, indented messages, the lines no regex matches) join their event.
        assert!(recs.len() < lines, "{file}: {} events from {lines} lines", recs.len());
        let multi = recs
            .iter()
            .filter(|r| r.exception.as_deref().is_some_and(|x| x.contains('\n')) || r.message.contains('\n'));
        assert!(multi.count() > 0, "{file}: no multi-line event");
    }
}

#[test]
fn parser_samples_keep_the_details_of_their_format() {
    let paths: Vec<PathBuf> = PARSER_SAMPLES
        .iter()
        .map(|(folder, file, _)| samples().join(folder).join(file))
        .collect();
    let (_ws, sources) = open_sources(&paths);
    let levels = |suffix: &str| -> std::collections::BTreeSet<Level> { records_of(&sources, suffix).iter().map(|r| r.level).collect() };
    let all = [
        Level::Verbose,
        Level::Debug,
        Level::Information,
        Level::Warning,
        Level::Error,
        Level::Fatal,
    ];

    // logcat: every frame of the crash is its own line (and event) with the same time, pid and tag; the year comes
    // from the file date, so only month, day and time are checked.
    let logcat = records_of(&sources, "logcat-20261014.txt");
    assert_eq!(levels("logcat-20261014.txt"), all.into());
    let fatal = logcat.iter().position(|r| r.message == "FATAL EXCEPTION: main").unwrap();
    let crash = &logcat[fatal..fatal + 9];
    assert!(crash
        .iter()
        .all(|r| r.ts == crash[0].ts && r.fields["Pid"] == 4211 && r.fields["Tag"] == "AndroidRuntime"));
    assert_eq!(crash[8].message, "\t... 12 more");
    let when = chrono::DateTime::from_timestamp_millis(crash[0].ts.unwrap()).unwrap();
    assert_eq!(when.format("%m-%d %H:%M:%S%.3f").to_string(), "10-14 10:41:24.460");
    // logcat's buffer divider and a line cut short are kept with the event before them, never an event of their own.
    assert!(
        logcat[fatal - 1].message.ends_with("\n--------- beginning of crash"),
        "{:?}",
        logcat[fatal - 1].message
    );
    assert!(logcat.iter().any(|r| r.fields["Pid"] == 4588), "the restarted app");

    // Spark: the logger is a field, the stack trace with its "Caused by" stays with the executor's error, and the
    // JVM's own stdout line joins the event before it.
    let spark = records_of(&sources, "spark-orders-export.log");
    assert_eq!(spark[0].fields["Logger"], "SparkContext");
    assert_eq!(spark[0].ts, Some(1_791_968_402_000), "26/10/14 09:00:02 UTC");
    let lost = spark.iter().find(|r| r.level == Level::Error).unwrap();
    assert_eq!(lost.fields["Logger"], "Executor");
    let x = lost.exception.as_deref().unwrap();
    assert!(x.starts_with("java.sql.SQLTransientConnectionException") && x.contains("Caused by: ") && x.ends_with("... 14 more"));
    assert!(
        spark[2].message.contains("Picked up JAVA_TOOL_OPTIONS") || spark[2].exception.as_deref().is_some_and(|x| x.contains("Picked up"))
    );
    // HDFS's older layout, with the thread number as a field.
    let dn = records_of(&sources, "hadoop-hdfs-datanode-dn01.log");
    assert_eq!(dn[0].ts, Some(1_791_968_400_000), "261014 090000 UTC");
    assert!(dn.iter().all(|r| r.fields["Thread"].is_number()));
    assert!(dn.iter().any(|r| r
        .exception
        .as_deref()
        .is_some_and(|x| x.starts_with("java.net.SocketTimeoutException"))));

    // Apache: module, process, thread and client are fields; the CGI script's traceback (its stderr, written without
    // a prefix) is kept with the event before it.
    let apache = records_of(&sources, "edge02/error.log");
    assert_eq!(
        levels("edge02/error.log"),
        [Level::Debug, Level::Information, Level::Warning, Level::Error].into()
    );
    assert_eq!(apache[0].fields["Module"], "mpm_event");
    assert_eq!(apache[0].ts, Some(1_791_968_400_000));
    let refused = apache.iter().find(|r| r.message.starts_with("(111)Connection refused")).unwrap();
    assert_eq!(
        (refused.level, &refused.fields["Module"]),
        (Level::Error, &serde_json::json!("proxy"))
    );
    assert!(refused.fields["Client"].as_str().is_some_and(|c| c.starts_with("10.0.")));
    let text = |r: &imt_core::model::Record| format!("{}\n{}", r.message, r.exception.as_deref().unwrap_or_default());
    assert!(apache
        .iter()
        .any(|r| text(r).contains("Traceback (most recent call last):\n  File \"/usr/lib/cgi-bin/order-report.cgi\"")));
    assert!(apache.iter().any(|r| text(r).contains("\n[Wed Oct 14 11:")), "the line cut short");

    // NLog: the logger is a field, the four fraction digits are read, the exception's lines stay with the event and
    // a stray Console.WriteLine joins the event before it.
    let nlog = records_of(&sources, "notify-scheduler-20261014.log");
    assert_eq!(levels("notify-scheduler-20261014.log"), all.into());
    assert_eq!(nlog[0].fields["Logger"], "Contoso.Notify.Program");
    let smtp = nlog.iter().find(|r| r.level == Level::Error).unwrap();
    assert!(
        text(smtp).contains("SmtpException: Failure sending mail.") && text(smtp).contains("--- End of inner exception stack trace ---")
    );
    assert!(text(smtp).ends_with("SendOrderMails.cs:line 64"), "{:?}", text(smtp));
    assert!(nlog.iter().any(|r| r.message.contains("\nDEBUG queue=")));
    let fatal = nlog.iter().find(|r| r.level == Level::Fatal).unwrap();
    assert_eq!(fatal.ts, Some(1_791_974_484_950), "10:41:24.9500, after the crash");

    // MEL console: category and event id are fields, the indented lines are the message, the exception follows.
    let mel = records_of(&sources, "payments-adapter-20261014.log");
    assert_eq!(levels("payments-adapter-20261014.log"), all.into());
    assert_eq!(
        (&mel[0].fields["Category"], &mel[0].fields["EventId"]),
        (&serde_json::json!("Microsoft.Hosting.Lifetime"), &serde_json::json!(14))
    );
    assert!(mel[2].message.contains("Hosting environment: Production\n") && mel[2].message.ends_with("Content root path: /app"));
    let failed = mel.iter().find(|r| r.level == Level::Error).unwrap();
    assert!(
        text(failed).contains("TimeoutRejectedException") && text(failed).contains(" ---> System.Threading.Tasks.TaskCanceledException")
    );
    assert!(mel.iter().any(|r| r.level == Level::Fatal && r.fields["EventId"] == 9001));
    assert!(mel.iter().any(|r| text(r).contains("\nPolly retry ")), "the library's own line");
}

#[test]
fn a_folder_of_every_parser_sample_splits_by_parser() {
    let dir = std::env::temp_dir().join(format!("imt-parsers-mixed-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (folder, file, _) in PARSER_SAMPLES {
        let name = file.rsplit('/').next().unwrap();
        std::fs::copy(samples().join(folder).join(file), dir.join(name)).unwrap();
    }
    // A built-in parser's own sample beside them (log4net with a PatternLayout).
    std::fs::write(dir.join("billing.log"), file("log4net-pattern").sample).unwrap();
    let (ws, _) = open_sources(std::slice::from_ref(&dir));
    let mut got: Vec<(String, String)> = ws
        .sources()
        .iter()
        .map(|s| (s.name.clone(), s.parser.clone().unwrap_or_default()))
        .collect();
    got.sort();
    let mut want: Vec<(String, String)> = PARSER_SAMPLES
        .iter()
        .map(|(_, file, parser)| (file.rsplit('/').next().unwrap().to_string(), parser.to_string()))
        .chain([("billing.log".to_string(), "log4net-pattern".to_string())])
        .collect();
    want.sort();
    assert_eq!(got, want);
    assert!(ws.sources().iter().all(|s| s.skipped == 0 && s.from.is_some()));
    std::fs::remove_dir_all(&dir).ok();
}
