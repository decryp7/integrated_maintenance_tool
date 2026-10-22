//! Verification end to end through the parse preview, on invented logs.

use super::*;
use crate::{Profile, Workspace};

fn ws() -> Workspace {
    Workspace::new(Profile::default_profile()).unwrap()
}

/// A parser for `regex` (TOML literal string) with optional extra `[format]` lines.
fn parser(regex: &str, extra: &str) -> String {
    format!(
        "schema = 1\nid = \"test-shop\"\nname = \"Shop\"\nversion = \"1.0.0\"\n[format]\nkind = \"text\"\nregexes = ['{regex}']\n{extra}"
    )
}

const PIPES: &str = r"^(?P<timestamp>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3})\|(?P<level>[A-Z]+)\|(?P<Worker>w\d+)\|(?P<message>.*)$";

/// An invented shop log: pipe-separated, a stack trace after errors.
fn shop(n: usize) -> String {
    let mut s = String::new();
    for i in 0..n {
        let level = if i % 10 == 9 { "ERROR" } else { "INFO" };
        s.push_str(&format!(
            "2026-03-01 10:{:02}:{:02}.{:03}|{level}|w{}|cart {i} updated\n",
            i / 60 % 60,
            i % 60,
            i % 1000,
            i % 3
        ));
        if level == "ERROR" {
            s.push_str("System.TimeoutException: stock service\n   at Cart.Save()\n");
        }
    }
    s
}

fn run(toml: &str, content: &str, facts: &FileFacts, metrics: &[MetricRule]) -> ParserVerdict {
    let ws = ws();
    let sample = verification_sample(content, MAX_VERIFY_LINES);
    verify_text(toml, &sample, "shop.log", facts, metrics, |r| ws.preview_parse(r))
}

#[test]
fn a_parser_that_reads_everything_is_done() {
    let facts = FileFacts {
        modified_ms: Some(1_772_359_200_000), // 2026-03-01 10:00 UTC
        machine_offsets: vec![],
    };
    let v = run(&parser(PIPES, "identity = ['Worker']"), &shop(120), &facts, &[]);
    assert_eq!(v.error, None);
    assert_eq!((v.events, v.with_timestamp, v.with_level), (120, 120, 120));
    assert_eq!(v.lines, 120 + 12 * 2);
    assert_eq!(v.readable, v.lines, "{:?}", v.failures);
    assert!(v.failures.is_empty() && v.timestamps.accepted && v.reads_levels);
    assert!(v.done());
    assert_eq!(
        (v.identity[0].field.as_str(), v.identity[0].accepted, v.identity[0].distinct),
        ("Worker", true, 3)
    );
    // Naive timestamps, the last one minutes after the modification time: nothing decides the zone but the
    // modification time.
    assert_eq!(v.zone.source, ZoneSource::ModificationTime);
    assert_eq!(v.zone.offset, "UTC");
    assert!(v.feedback().contains("100.0%"));
}

#[test]
fn missed_entries_bad_timestamps_and_levels_are_named_by_line() {
    let mut content = shop(30);
    // Lines in another layout (a space instead of the bar before the level), at known places.
    content.push_str("2026-03-01 10:00:31.000 WARN|w1|stock low\n");
    content.push_str("2026-03-01 10:00:32.000 WARN|w2|stock low again\n");
    // An unknown time format and a missing level.
    content.push_str("2026-03-01T10:00:33|INFO|w1|odd time\n");
    content.push_str("2026-03-01 10:00:34.000||w1|no level\n");
    let lines: Vec<&str> = content.lines().collect();
    let regex = r"^(?P<timestamp>[0-9T:. -]+)\|(?P<level>[A-Z]*)\|(?P<Worker>w\d+)\|(?P<message>.*)$";
    let v = run(&parser(regex, ""), &content, &FileFacts::default(), &[]);
    let by_rule = |r: FailedRule| v.failures.iter().filter(|f| f.rule == r).map(|f| f.line).collect::<Vec<_>>();
    let line_of = |prefix: &str| lines.iter().position(|l| l.starts_with(prefix)).unwrap() as u32 + 1;
    assert_eq!(
        by_rule(FailedRule::MissedEntry),
        [line_of("2026-03-01 10:00:31"), line_of("2026-03-01 10:00:32")]
    );
    assert_eq!(by_rule(FailedRule::Level), [line_of("2026-03-01 10:00:34")]);
    assert!(
        by_rule(FailedRule::Timestamp).is_empty(),
        "the generic reader understands the T form: {:?}",
        v.failures
    );
    let missed = v.failures.iter().find(|f| f.rule == FailedRule::MissedEntry).unwrap();
    assert!(missed.fragment.starts_with("2026-03-01 10:00:31.000 WARN"));
    assert!(v.score < TARGET_SCORE && !v.done());
    assert!(v.feedback().contains(&format!("line {}", missed.line)));

    // A timestamp group that captures too much: every event fails its timestamp; reported twice, counted all.
    let greedy = r"^(?P<timestamp>.+?\|[A-Z]+)\|(?P<Worker>w\d+)\|(?P<message>.*)$";
    let v = run(&parser(greedy, ""), &shop(50), &FileFacts::default(), &[]);
    let ts: Vec<&LineFailure> = v.failures.iter().filter(|f| f.rule == FailedRule::Timestamp).collect();
    assert_eq!(ts.len(), 2, "two examples of one reason");
    assert_eq!(ts[0].fragment, "2026-03-01 10:00:00.000|INFO");
    assert_eq!((ts[0].line, ts[1].line), (1, 2));
    assert!(!v.timestamps.accepted && v.timestamps.reason.as_deref().unwrap().contains("0 of 50"));
    assert_eq!(v.readable, 0);
    assert!(!v.reads_levels, "no level group");
}

#[test]
fn failures_are_capped_and_errors_are_verdicts() {
    let content: String = (0..200).map(|i| format!("line {i} with nothing to read\n")).collect();
    let v = run(&parser(PIPES, ""), &content, &FileFacts::default(), &[]);
    assert!(v.failures.len() <= MAX_FAILURES);
    assert_eq!(v.failures.len(), 1, "no event: the start of the file, once: {:?}", v.failures);
    assert_eq!(v.events, 0);
    let broken = run("schema = 1\nid = \"x\"", &shop(5), &FileFacts::default(), &[]);
    assert!(broken.error.as_deref().unwrap().contains("does not load"));
    assert!(!broken.done());
    let bad_regex = run(&parser("(?P<timestamp>[", ""), &shop(5), &FileFacts::default(), &[]);
    assert!(bad_regex.error.is_some(), "{bad_regex:?}");
    assert!(bad_regex.feedback().contains("cannot be used"));
}

#[test]
fn metrics_and_the_timestamp_suggestion_reach_the_verdict() {
    // An invented monitor line with numbers, its time in .NET ticks.
    let ticks = |ms: i64| (ms as i128 * 10_000 + 621_355_968_000_000_000).to_string();
    let t0 = 1_772_359_200_000;
    let content: String = (0..40)
        .map(|i| {
            format!(
                "{} INFO WorkingSet {} MB · CPU {}.5% · Threads {}\n",
                ticks(t0 + i * 1000),
                600 + i,
                20 + i,
                60 + i
            )
        })
        .collect();
    let regex = r"^(?P<timestamp>\d{18}) (?P<level>[A-Z]+) (?P<message>.*)$";
    let metrics = [MetricRule {
        name: "WorkingSet".into(),
        regex: r"WorkingSet (?P<value>\d+) MB".into(),
        unit: "MB".into(),
    }];
    let facts = FileFacts {
        modified_ms: Some(t0 + 60_000),
        machine_offsets: vec![540],
    };
    let v = run(&parser(regex, ""), &content, &facts, &metrics);
    assert!(!v.timestamps.accepted);
    assert_eq!(v.timestamps.suggested_format.as_deref(), Some("dotnet_ticks"));
    assert!(v.feedback().contains("\"dotnet_ticks\""));
    let fixed = run(&parser(regex, "timestamp_formats = ['dotnet_ticks']"), &content, &facts, &metrics);
    assert!(fixed.done(), "{}", fixed.feedback());
    assert_eq!(fixed.timestamps.suggested_format, None);
    assert!(fixed.metrics[0].accepted && fixed.metrics[0].matches == 40);
    // Ticks carry no zone; another file of the machine writes +09:00.
    assert_eq!((fixed.zone.source, fixed.zone.offset.as_str()), (ZoneSource::SameMachine, "+09:00"));
}

#[test]
fn uptime_events_count_from_the_boot_time() {
    let content = "[    0.000000] Linux version 6.8.0\n[    1.250000] usb 1-1: new device\n[   12.500000] eth0: link up\n";
    let regex = r"^\[\s*(?P<timestamp>\d+\.\d+)\] (?P<message>.*)$";
    let modified = 1_772_359_200_000;
    let facts = FileFacts {
        modified_ms: Some(modified),
        machine_offsets: vec![],
    };
    let v = run(&parser(regex, "timestamp_formats = ['uptime']"), content, &facts, &[]);
    assert!(!v.timestamps.accepted, "1970 without a boot time");
    let boot = timestamps::uptime_boot(&["12.500000".to_string()], modified).unwrap();
    let boot_time = chrono::DateTime::from_timestamp_millis(boot).unwrap().to_rfc3339();
    let v = run(
        &parser(regex, &format!("timestamp_formats = ['uptime']\nboot_time = '{boot_time}'")),
        content,
        &facts,
        &[],
    );
    assert!(v.timestamps.accepted, "{v:?}");
    assert_eq!(v.timestamps.last, Some(modified));
    assert!(v.done(), "no level group: levels are not required: {}", v.feedback());
}
