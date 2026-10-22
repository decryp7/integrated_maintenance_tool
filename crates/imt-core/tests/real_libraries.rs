//! The built-in parsers on what the real .NET logging libraries write (`samples/real-libraries`, from
//! `tools/dotnet-sample-gen`): Serilog (CLEF, JSON, text), log4net (PatternLayout, a CSV-like layout),
//! NLog's default layout and the Microsoft.Extensions.Logging console. Every file holds the same seven
//! events: inner exceptions, structured properties, Japanese text and Tokyo (+09:00) times.
//! log4net's XmlLayoutSchemaLog4j file is read by the Formats plugin (tested in imt-plugins).

use imt_core::model::{Level, Record};
use imt_core::{Profile, Workspace};
use std::path::PathBuf;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples/real-libraries")
}

/// The file's parser and its events, read on their own.
fn read(file: &str) -> (Option<String>, Vec<Record>) {
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(&[dir().join(file)]).unwrap();
    let s = ws.summary();
    assert!(s.unrecognized.is_empty(), "{file}: {:?}", s.unrecognized);
    assert_eq!(s.sources.len(), 1, "{file}");
    assert_eq!(s.sources[0].skipped, 0, "{file}");
    (s.sources[0].parser.clone(), ws.records().to_vec())
}

/// The scenario's levels, in order.
const LEVELS: [Level; 7] = [
    Level::Information,
    Level::Debug,
    Level::Information,
    Level::Warning,
    Level::Error,
    Level::Warning,
    Level::Fatal,
];

/// 2026-10-14 19:30:00.120 +09:00, the first event.
const FIRST: i64 = 1_791_973_800_120;
const HOUR: i64 = 3_600_000;

/// Message and exception, for checks that hold wherever the parser puts the exception's first line.
fn text(r: &Record) -> String {
    format!("{}\n{}", r.message, r.exception.as_deref().unwrap_or_default())
}

/// What every library writes of the scenario: the levels, the Japanese text and both exceptions with their inner ones.
fn assert_scenario(file: &str, recs: &[Record]) {
    let levels: Vec<Level> = recs.iter().map(|r| r.level).collect();
    assert_eq!(levels, LEVELS, "{file}");
    assert!(recs.iter().all(|r| r.ts.is_some()), "{file}");
    assert!(recs.windows(2).all(|w| w[0].ts <= w[1].ts), "{file}: events out of order");
    assert!(
        text(&recs[2]).contains("注文 88412 を受け付けました"),
        "{file}: {:?}",
        recs[2].message
    );
    let capture = text(&recs[4]);
    assert!(
        capture.contains("System.InvalidOperationException: Payment capture failed for order 88412"),
        "{file}: {capture}"
    );
    assert!(
        capture.contains(" ---> System.TimeoutException: 決済ゲートウェイがタイムアウトしました"),
        "{file}"
    );
    assert!(capture.contains("--- End of inner exception stack trace ---"), "{file}");
    assert!(recs[4].exception.is_some(), "{file}: no exception on the error");
    let fatal = text(&recs[6]);
    assert!(
        fatal.contains("System.AggregateException: One or more background tasks failed."),
        "{file}"
    );
    assert!(
        fatal.contains("(Inner Exception #1) System.IO.IOException: ディスクに空き容量がありません"),
        "{file}"
    );
    // Each exception stays with its event: the event after it starts cleanly.
    assert!(
        text(&recs[5]).contains("決済を再試行します（2/3）") && recs[5].exception.is_none(),
        "{file}"
    );
}

#[test]
fn serilog_clef_json_and_text_keep_offsets_properties_and_exceptions() {
    for (file, parser) in [
        ("serilog-compact.clef", "serilog-clef"),
        ("serilog-json.json", "serilog-json"),
        ("serilog-text.txt", "serilog-text"),
    ] {
        let (p, recs) = read(file);
        assert_eq!(p.as_deref(), Some(parser));
        assert_scenario(file, &recs);
        // CLEF writes UTC, the JSON formatter and the text template +09:00: the same instants.
        assert_eq!(recs[0].ts, Some(FIRST), "{file}");
        assert_eq!(recs[6].ts, Some(FIRST + 12 * 60_000 + 879), "{file}: 19:42:00.999 +09:00");
    }
    let (_, clef) = read("serilog-compact.clef");
    assert_eq!(
        clef[2].template.as_deref(),
        Some("注文 {OrderNo} を受け付けました（顧客 {CustomerId}、合計 {Total} 円）")
    );
    assert_eq!(clef[2].message, "注文 88412 を受け付けました（顧客 C-20931、合計 12800 円）");
    assert_eq!(
        (&clef[2].fields["OrderNo"], &clef[2].fields["CustomerId"]),
        (&88412.into(), &"C-20931".into())
    );
    assert_eq!(clef[1].fields["Stats.Entries"], 412, "a destructured object's properties");
    assert_eq!(clef[0].fields["SourceContext"], "Contoso.Orders.Program");
    assert!(clef[4]
        .exception
        .as_deref()
        .is_some_and(|x| x.starts_with("System.InvalidOperationException")));
    let (_, json) = read("serilog-json.json");
    assert_eq!(json[1].fields["Stats"]["HitRatio"], 0.93);
    assert_eq!(json[3].fields["ElapsedMs"], 4870);
    let (_, txt) = read("serilog-text.txt");
    assert_eq!(txt[1].fields["Entries"], 412, "the {{@Stats}} JSON in the text");
    assert!(txt[6]
        .exception
        .as_deref()
        .is_some_and(|x| x.starts_with("System.AggregateException")));
}

#[test]
fn log4net_pattern_layout_reads_its_local_times_as_utc() {
    let (p, recs) = read("log4net-pattern.log");
    assert_eq!(p.as_deref(), Some("log4net-pattern"));
    assert_scenario("log4net-pattern.log", &recs);
    assert_eq!(
        (&recs[0].fields["Thread"], &recs[0].fields["Logger"]),
        (&1.into(), &"Contoso.Orders.Program".into())
    );
    assert!(recs[4]
        .exception
        .as_deref()
        .is_some_and(|x| x.starts_with("System.InvalidOperationException")));
    // %date is the machine's local time without an offset (+08:00 where the file was written); the parser reads
    // it as UTC unless [format] assume_timezone says otherwise.
    assert_eq!(recs[0].ts, Some(FIRST + 8 * HOUR));
}

#[test]
fn log4net_csv_layout_keeps_the_offset() {
    let (p, recs) = read("log4net-csv.log");
    assert_eq!(p.as_deref(), Some("log4net-csv"));
    assert_eq!(recs[0].ts, Some(FIRST), "18:30:00.120+08:00");
    assert_eq!(
        (&recs[0].fields["Code"], &recs[0].fields["Process"], &recs[0].fields["Pid"]),
        (&1000.into(), &"OrderService".into(), &4120.into())
    );
    let jp = recs.iter().find(|r| r.message.starts_with("注文")).unwrap();
    assert_eq!(jp.message, "注文 88412 を受け付けました（顧客 C-20931、合計 12800 円）");
}

#[test]
fn log4net_csv_layout_keeps_messages_with_commas_and_exceptions() {
    let (_, recs) = read("log4net-csv.log");
    assert_scenario("log4net-csv.log", &recs);
    assert_eq!(recs[6].message, "Unhandled exception, the service stops");
}

#[test]
fn nlog_default_layout_reads_its_wall_clock_as_utc() {
    let (p, recs) = read("nlog-default.log");
    assert_eq!(p.as_deref(), Some("nlog"));
    assert_scenario("nlog-default.log", &recs);
    assert_eq!(recs[0].fields["Logger"], "Contoso.Orders.Program");
    // longdate is the event's wall-clock time (Tokyo here) without an offset: read as UTC.
    assert_eq!(recs[0].ts, Some(FIRST + 9 * HOUR));
    assert_eq!(recs[1].fields["Entries"], 412, "the {{@Stats}} JSON in the message");
}

#[test]
fn nlog_exception_after_the_separator_is_the_exception() {
    let (_, recs) = read("nlog-default.log");
    assert_eq!(recs[4].message, "Payment capture failed for order 88412");
    assert!(recs[4]
        .exception
        .as_deref()
        .is_some_and(|x| x.starts_with("System.InvalidOperationException")));
}

#[test]
fn mel_console_keeps_the_offset_category_and_event_id() {
    let (p, recs) = read("mel-console.log");
    assert_eq!(p.as_deref(), Some("mel-console"));
    assert_scenario("mel-console.log", &recs);
    assert_eq!(
        (&recs[4].fields["Category"], &recs[4].fields["EventId"]),
        (&"Contoso.Orders.Payments".into(), &1004.into())
    );
    // The console formatter takes the clock's time (the run's), written with its offset.
    let first = std::fs::read_to_string(dir().join("mel-console.log")).unwrap();
    let stamp = &first[..first.find(" info:").unwrap()];
    let want = chrono::DateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M:%S%.3f%:z").unwrap();
    assert_ne!(want.offset().local_minus_utc(), 0, "the sample has a non-UTC offset");
    assert_eq!(recs[0].ts, Some(want.timestamp_millis()));
}

#[test]
fn mel_console_message_stays_the_message_when_there_is_an_exception() {
    let (_, recs) = read("mel-console.log");
    assert!(
        recs[4].message.contains("Payment capture failed for order 88412"),
        "{:?}",
        recs[4].message
    );
    assert!(recs[4]
        .exception
        .as_deref()
        .is_some_and(|x| x.trim_start().starts_with("System.InvalidOperationException")));
}
