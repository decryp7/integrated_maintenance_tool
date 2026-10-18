//! Every built-in profile template compiles, and parses its sample (and the
//! matching files in /samples) the way the gallery's preview does.

use imt_core::api::ParsePreviewRequest;
use imt_core::model::Level;
use imt_core::templates::{ProfileTemplate, TEMPLATES};
use imt_core::{Profile, Workspace};
use std::path::PathBuf;

fn samples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples")
}

/// Header lines that are not events (IIS's #Software/#Version/#Date/#Fields).
fn allowed_skips(t: &ProfileTemplate) -> usize {
    if t.id == "iis-w3c" {
        4
    } else {
        0
    }
}

#[test]
fn every_template_compiles() {
    let ws = Workspace::new(Profile::default_profile()).unwrap();
    for t in TEMPLATES {
        let profile = Profile::from_toml(t.toml).unwrap_or_else(|e| panic!("{}: {e}", t.id));
        assert_eq!(profile.name, t.name, "{}: the profile's name is the template's", t.id);
        // The EVTX template needs the Windows plugin only to read files, not to compile.
        ws.validate_profile(profile).unwrap_or_else(|e| panic!("{}: {e:#}", t.id));
    }
}

#[test]
fn every_template_parses_its_sample() {
    let ws = Workspace::new(Profile::default_profile()).unwrap();
    for t in TEMPLATES.iter().filter(|t| !t.sample_text.is_empty()) {
        let p = ws.preview_parse(&ParsePreviewRequest {
            toml: t.toml.to_string(),
            sample: t.sample_text.to_string(),
            file_name: t.sample_name.to_string(),
            limit: 100,
        });
        assert_eq!(p.error, None, "{}", t.id);
        assert!(p.total >= 3, "{}: {} events", t.id, p.total);
        assert!(p.skipped <= allowed_skips(t), "{}: {} lines skipped", t.id, p.skipped);
        for r in &p.records {
            assert!(r.ts.is_some(), "{}: line {} has no timestamp", t.id, r.line);
        }
        let levels: Vec<Level> = p.records.iter().map(|r| r.level).collect();
        assert!(levels.contains(&Level::Error), "{}: {levels:?}", t.id);
        assert!(levels.iter().any(|l| *l != Level::Error), "{}: {levels:?}", t.id);
    }
}

#[test]
fn templates_keep_details_of_their_format() {
    let ws = Workspace::new(Profile::default_profile()).unwrap();
    let preview = |id: &str| {
        let t = imt_core::templates::find(id).unwrap();
        ws.preview_parse(&ParsePreviewRequest {
            toml: t.toml.to_string(),
            sample: t.sample_text.to_string(),
            file_name: t.sample_name.to_string(),
            limit: 100,
        })
    };
    let syslog = preview("syslog");
    assert_eq!(syslog.total, 6);
    let lv: Vec<Level> = syslog.records.iter().map(|r| r.level).collect();
    assert_eq!(
        lv,
        [
            Level::Information,
            Level::Warning,
            Level::Error,
            Level::Fatal,
            Level::Information,
            Level::Debug
        ]
    );
    assert_eq!(syslog.records[3].fields["Host"], "web01");
    assert_eq!(syslog.records[4].fields["App"], "systemd");
    // RFC 5424 files.
    let t = imt_core::templates::find("syslog").unwrap();
    let rfc5424 = ws.preview_parse(&ParsePreviewRequest {
        toml: t.toml.to_string(),
        sample: "<14>1 2026-10-14T10:41:00.120Z web01 orders 4211 - - Service started\n\
                 <12>1 2026-10-14T10:41:02.540Z web01 orders 4211 SLOW [meta sequenceId=\"1\"] Slow request\n\
                 <11>1 2026-10-14T10:41:24.050Z web01 orders 4211 - - Payment failed\n"
            .to_string(),
        file_name: "syslog.log".into(),
        limit: 10,
    });
    let lv: Vec<(Level, bool)> = rfc5424.records.iter().map(|r| (r.level, r.ts.is_some())).collect();
    assert_eq!(lv, [(Level::Information, true), (Level::Warning, true), (Level::Error, true)]);
    assert_eq!(rfc5424.records[1].message, "Slow request");
    let iis = preview("iis-w3c");
    assert_eq!(iis.records[1].level, Level::Warning);
    assert_eq!(iis.records[2].level, Level::Error);
    assert_eq!(iis.records[2].message, "POST /api/payments");
    let access = preview("access-combined");
    assert_eq!(access.records[2].level, Level::Error);
    assert_eq!(access.records.len(), 4, "the common format (no referrer) too");
    let mel = preview("mel-console");
    assert!(mel.records[3].exception.as_deref().is_some_and(|x| x.contains("TimeoutException")));
    assert!(mel.records[0].message.contains("Now listening"));
    let xml = preview("log4net-xml");
    assert_eq!(xml.records[0].fields["Logger"], "Billing.Service");
    assert!(xml.records[2].exception.is_some());
    assert_eq!(xml.records[3].message, "Refreshed tariff cache (412 entries)");
    let pattern = preview("log4net-pattern");
    assert_eq!(pattern.records[3].fields["Ndc"], "tariffs");
    assert!(pattern.records[2].exception.is_some());
    let apache = preview("apache-error");
    let lv: Vec<Level> = apache.records.iter().map(|r| r.level).collect();
    assert_eq!(lv, [Level::Information, Level::Warning, Level::Error, Level::Error]);
    assert_eq!(apache.records[2].fields["Module"], "proxy_http");
    assert_eq!(apache.records[2].fields["Tid"], 151);
    assert_eq!(apache.records[2].fields["Client"], "10.0.0.5:50514");
    assert_eq!(apache.records[3].message, "File does not exist: /var/www/favicon.ico");
    assert_eq!(
        apache.records[3].ts,
        Some(1_791_974_490_000),
        "2026-10-14 10:41:30 UTC, the 2.2 layout"
    );
    let logcat = preview("android-logcat");
    let lv: Vec<Level> = logcat.records.iter().map(|r| r.level).collect();
    assert_eq!(lv[..2], [Level::Information, Level::Warning]);
    assert_eq!(lv[5], Level::Debug);
    assert_eq!(logcat.records[1].fields["Tag"], "OkHttp");
    assert_eq!(logcat.records[1].fields["Tid"], 4230);
    assert_eq!(logcat.records[1].message, "Slow response from /api/orders (1500 ms)");
    let spark = preview("hadoop-spark");
    assert_eq!(spark.records[2].fields["Logger"], "Executor");
    assert!(spark.records[2]
        .exception
        .as_deref()
        .is_some_and(|x| x.starts_with("java.io.IOException")));
    // The older HDFS layout, with a thread number.
    let t = imt_core::templates::find("hadoop-spark").unwrap();
    let hdfs = ws.preview_parse(&ParsePreviewRequest {
        toml: t.toml.to_string(),
        sample: "081109 203615 148 INFO dfs.DataNode$PacketResponder: PacketResponder 1 terminating\n\
                 081109 203807 222 WARN dfs.DataNode: Slow BlockReceiver\n"
            .to_string(),
        file_name: "hdfs.log".into(),
        limit: 10,
    });
    let r = &hdfs.records[0];
    assert_eq!(
        (r.fields["Thread"].clone(), r.fields["Logger"].clone()),
        (148.into(), "dfs.DataNode$PacketResponder".into())
    );
    assert_eq!(r.ts, Some(1_226_262_975_000), "2008-11-09 20:36:15 UTC");
    assert_eq!(hdfs.records[1].level, Level::Warning);
}

#[test]
fn templates_read_the_matching_sample_files() {
    for (id, folder) in [
        ("serilog-clef", "serilog-clef"),
        ("serilog-json", "serilog-json"),
        ("serilog-text", "serilog-text"),
        ("log4net-pattern", "log4net"),
        ("csv", "csv"),
    ] {
        let t = imt_core::templates::find(id).unwrap();
        let mut ws = Workspace::new(Profile::from_toml(t.toml).unwrap()).unwrap();
        ws.open(&[samples().join(folder)]).unwrap();
        let s = ws.summary();
        assert!(!s.sources.is_empty() && s.unrecognized.is_empty(), "{id}: {:?}", s.unrecognized);
        for src in &s.sources {
            assert_eq!(src.skipped, 0, "{id}: {} skipped lines in {}", src.skipped, src.name);
            assert!(src.records > 0 && src.from.is_some(), "{id}: {}", src.name);
        }
    }
}
