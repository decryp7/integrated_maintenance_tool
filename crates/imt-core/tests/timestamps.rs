//! Timestamps end to end: unusual formats import with their timestamps, and a file whose timestamps
//! cannot be read is flagged on its source with a format the engine verified (fixtures in
//! `tests/fixtures/timestamps`).

use chrono::{NaiveDate, TimeZone, Utc};
use imt_core::api::SourceInfo;
use imt_core::{Profile, Workspace};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/timestamps").join(name)
}

fn open(names: &[&str], profile: Profile) -> Workspace {
    let mut ws = Workspace::new(profile).unwrap();
    ws.open(&names.iter().map(|n| fixture(n)).collect::<Vec<_>>()).unwrap();
    ws
}

fn source(ws: &Workspace, name: &str) -> SourceInfo {
    let summary = ws.summary();
    let found = summary.sources.iter().find(|s| s.name.ends_with(name));
    found
        .cloned()
        .unwrap_or_else(|| panic!("{name} not imported: {:?}", summary.unrecognized))
}

fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32, ms: u32) -> i64 {
    let t = NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_milli_opt(h, mi, s, ms).unwrap();
    Utc.from_utc_datetime(&t).timestamp_millis()
}

#[test]
fn slash_comma_offset_timestamps_import() {
    let ws = open(&["comma-offset.log"], Profile::default_profile());
    let s = source(&ws, "comma-offset.log");
    assert_eq!(s.importer, "text");
    // Four events; the stack trace stays with the error.
    assert_eq!((s.records, s.skipped), (4, 0));
    assert_eq!(s.from, Some(utc(2025, 11, 7, 23, 4, 37, 759)));
    assert_eq!(s.to, Some(utc(2025, 11, 7, 23, 4, 40, 0)));
    assert_eq!(s.timestamp_issue, None);
    let summary = ws.summary();
    assert_eq!(summary.levels.get(&imt_core::model::Level::Error), Some(&1));
}

#[test]
fn comma_variants_and_log4j_commas_import() {
    let ws = open(&["comma-variants.log", "log4j-comma.log"], Profile::default_profile());
    let s = source(&ws, "comma-variants.log");
    assert_eq!((s.importer.as_str(), s.records), ("text", 3));
    assert_eq!(s.from, Some(utc(2025, 11, 7, 23, 4, 37, 759)));
    assert_eq!(s.to, Some(utc(2025, 11, 8, 0, 4, 37, 759)));
    assert_eq!(s.timestamp_issue, None);
    // The comma as the decimal separator still reads as before.
    let s = source(&ws, "log4j-comma.log");
    assert_eq!(s.records, 2);
    assert_eq!(s.from, Some(utc(2025, 11, 8, 0, 4, 37, 759)));
    assert_eq!(s.to, Some(utc(2025, 11, 8, 0, 4, 38, 1)));
}

#[test]
fn comma_separated_dates_stay_csv() {
    let ws = open(&["date-time-columns.csv", "headerless.csv"], Profile::default_profile());
    let s = source(&ws, "date-time-columns.csv");
    assert_eq!((s.importer.as_str(), s.records), ("csv", 2));
    assert_eq!(s.from, Some(utc(2025, 11, 8, 0, 0, 0, 0)));
    let s = source(&ws, "headerless.csv");
    assert_eq!(s.importer, "csv");
}

#[test]
fn unreadable_timestamps_are_flagged_with_a_verified_format() {
    let ws = open(&["ctime.csv"], Profile::default_profile());
    let s = source(&ws, "ctime.csv");
    assert_eq!((s.importer.as_str(), s.records, s.from), ("csv", 4, None));
    let issue = s.timestamp_issue.expect("flagged");
    assert_eq!((issue.unparsed_lines, issue.total_lines), (4, 4));
    assert_eq!(
        issue.examples,
        [
            "Sat Nov 08 00:04:37 2025,INFO,started",
            "Sat Nov 08 00:04:38 2025,WARN,slow",
            "Sat Nov 08 00:04:39 2025,ERROR,failed"
        ]
    );
    let suggestion = issue.suggestion.expect("a format");
    assert_eq!(suggestion.format, "%a %b %d %H:%M:%S %Y");
    assert_eq!(suggestion.sample, "Sat Nov 08 00:04:37 2025");
    assert_eq!(issue.example_lines, [2, 3, 4]);
    assert_eq!((suggestion.parsed_samples, suggestion.checked_samples), (4, 4));

    // The suggestion's snippet, merged into the source's parser, reads the file: the issue is gone.
    let parser_id = source(&ws, "ctime.csv").parser.unwrap();
    let changed = imt_core::parser::builtin::find(&parser_id)
        .unwrap()
        .with_override(&suggestion.parser_snippet)
        .unwrap();
    let mut setup = imt_core::parser::ParserSetup::builtins();
    for p in &mut setup.parsers {
        if p.toml.contains(&format!("id = \"{parser_id}\"")) {
            p.toml = changed.canonical_toml();
        }
    }
    let mut ws = Workspace::with_parsers(Profile::default_profile(), imt_core::PluginRegistry::new(), &setup).unwrap();
    ws.open(&[fixture("ctime.csv")]).unwrap();
    let s = source(&ws, "ctime.csv");
    assert_eq!(s.timestamp_issue, None);
    assert_eq!(s.from, Some(utc(2025, 11, 8, 0, 4, 37, 0)));

    // The wire format the UI reads.
    let ws = open(&["ctime.csv"], Profile::default_profile());
    let json = serde_json::to_value(ws.summary()).unwrap();
    let issue = &json["sources"][0]["timestampIssue"];
    assert_eq!(issue["unparsedLines"], 4);
    assert_eq!(issue["totalLines"], 4);
    assert_eq!(issue["exampleLines"], serde_json::json!([2, 3, 4]));
    assert_eq!(issue["suggestion"]["parsedSamples"], 4);
    assert_eq!(issue["suggestion"]["checkedSamples"], 4);
    assert_eq!(issue["suggestion"]["format"], "%a %b %d %H:%M:%S %Y");
    assert!(issue["suggestion"]["parserSnippet"].as_str().unwrap().contains("timestamp_formats"));
    let fine = open(&["comma-offset.log"], Profile::default_profile());
    assert!(serde_json::to_value(fine.summary()).unwrap()["sources"][0]["timestampIssue"].is_null());
}

#[test]
fn the_issue_survives_a_reload_once() {
    let mut ws = open(&["ctime.csv"], Profile::default_profile());
    ws.reload().unwrap();
    let summary = ws.summary();
    assert_eq!(summary.sources.len(), 1);
    assert!(summary.sources[0].timestamp_issue.is_some());
}

#[test]
fn no_sample_is_flagged() {
    // Settings files, profiler captures and logs alike: none has unreadable timestamps.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples");
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(&[root]).unwrap();
    let flagged: Vec<String> = ws
        .summary()
        .sources
        .into_iter()
        .filter(|s| s.timestamp_issue.is_some())
        .map(|s| s.name)
        .collect();
    // Known bug (ignored test in real_libraries.rs): the log4net CSV parser turns the lines of a multi-line exception
    // into untimed events. Expect nothing flagged once it is fixed.
    assert_eq!(flagged, ["real-libraries/log4net-csv.log"]);
}

// Regression: a log4net CSV layout without a header, its date and time split by the comma delimiter. The first
// event was read as a header, and only the date became the timestamp. (Invented values in the fixture.)
#[test]
fn a_headerless_csv_with_date_and_time_in_two_columns_imports() {
    let ws = open(&["log4net-csv.log"], Profile::default_profile());
    let s = source(&ws, "log4net-csv.log");
    assert_eq!(s.importer, "csv");
    assert_eq!((s.records, s.skipped), (5, 0), "every row is an event");
    assert_eq!(s.from, Some(utc(2024, 3, 2, 8, 15, 0, 13)));
    assert_eq!(s.to, Some(utc(2024, 3, 2, 22, 59, 59, 999)));
}
