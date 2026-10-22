//! The timestamp rule of a parser (ADR 0032): accepted only when (nearly) every event has a timestamp, the times
//! run forwards (continuations and small reorderings allowed) and they are plausible against the file's
//! modification time. When it is not, the engine looks for a format that reads the timestamp texts itself.

use crate::parser::Parser;
use crate::time::{NumberClock, TimestampParser};
use serde::{Deserialize, Serialize};

/// Share of the events that must have a timestamp.
pub const MIN_TIMED: f64 = 0.98;
/// A step back by more than this is out of order (smaller ones are reorderings between threads).
const BACKWARDS_MS: i64 = 5_000;
/// Events may end this long after the file's modification time (a wrong zone is at most 14 h off).
const FUTURE_SLACK_MS: i64 = 15 * 3_600_000;
/// Events may start this long before the file's modification time.
const PAST_SPAN_MS: i64 = 10 * 366 * 86_400_000;
/// 1990-01-01: nothing in a log file is older.
const OLDEST_MS: i64 = 631_152_000_000;

/// How the timestamp rule fared.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimestampCheck {
    pub accepted: bool,
    /// Events with a timestamp, and all events.
    pub timed: usize,
    pub events: usize,
    /// Steps back by more than a few seconds.
    pub backwards: usize,
    pub first: Option<i64>,
    pub last: Option<i64>,
    pub plausible: bool,
    /// Why it was rejected.
    pub reason: Option<String>,
    /// A format (for `timestamp_formats`) that reads the timestamp texts, when the engine found one.
    pub suggested_format: Option<String>,
}

/// Checks the events' times, in file order (`(file line, time)`), against the modification time.
pub fn check(times: &[(u32, Option<i64>)], modified_ms: Option<i64>) -> TimestampCheck {
    let timed: Vec<(u32, i64)> = times.iter().filter_map(|(l, t)| t.map(|t| (*l, t))).collect();
    let mut out = TimestampCheck {
        timed: timed.len(),
        events: times.len(),
        first: timed.iter().map(|(_, t)| *t).min(),
        last: timed.iter().map(|(_, t)| *t).max(),
        ..Default::default()
    };
    let mut max = i64::MIN;
    let mut first_back = None;
    for (line, t) in &timed {
        if *t + BACKWARDS_MS < max {
            out.backwards += 1;
            first_back.get_or_insert(*line);
        }
        max = max.max(*t);
    }
    out.plausible = match (out.first, out.last) {
        (Some(first), Some(last)) => match modified_ms {
            Some(m) => first >= OLDEST_MS && first >= m - PAST_SPAN_MS && last <= m + FUTURE_SLACK_MS,
            None => first >= OLDEST_MS && last <= chrono::Utc::now().timestamp_millis() + FUTURE_SLACK_MS,
        },
        _ => false,
    };
    let allowed_back = (timed.len() / 100).max(1);
    out.reason = if times.is_empty() {
        Some("the parser reads no event".into())
    } else if (timed.len() as f64) < times.len() as f64 * MIN_TIMED {
        Some(format!("only {} of {} events have a timestamp", timed.len(), times.len()))
    } else if out.backwards > allowed_back {
        Some(format!(
            "the times go back {} times (first at line {}): the timestamp is not read in full (date order, 12-hour clock, year?)",
            out.backwards,
            first_back.unwrap_or(0)
        ))
    } else if !out.plausible {
        Some(match (out.last, modified_ms) {
            (Some(last), Some(m)) if last > m + FUTURE_SLACK_MS => {
                "the events end after the file was last written: check the year, the date order and the time zone".to_string()
            }
            _ => "the times are not plausible for this file (before 1990 or years before it was written)".to_string(),
        })
    } else {
        None
    };
    out.accepted = out.reason.is_none();
    out
}

/// The boot time of a file whose events count seconds since boot: its modification time minus the last uptime.
pub fn uptime_boot(texts: &[String], modified_ms: i64) -> Option<i64> {
    let last = texts.iter().rev().find_map(|t| NumberClock::Uptime.read(t, 0))?;
    Some(modified_ms - last)
}

/// [`uptime_boot`] as the RFC 3339 text of `[format] boot_time`.
pub fn uptime_boot_time(texts: &[String], modified_ms: i64) -> Option<String> {
    let boot = uptime_boot(texts, modified_ms)?;
    chrono::DateTime::from_timestamp_millis(boot).map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

/// A format that reads (nearly) all of `texts` with plausible times, when the parser's own formats do not.
pub fn suggest_format(parser: &Parser, texts: &[String], modified_ms: Option<i64>) -> Option<String> {
    if texts.is_empty() {
        return None;
    }
    let f = &parser.format;
    let base = TimestampParser::new(&f.timestamp_formats, &f.assume_timezone, f.day_first.resolve().ok().flatten())
        .ok()?
        .with_reference(modified_ms);
    let reads =
        |p: &TimestampParser| -> Vec<(u32, Option<i64>)> { texts.iter().enumerate().map(|(i, t)| (i as u32 + 1, p.parse(t))).collect() };
    if check(&reads(&base), modified_ms).accepted {
        return None;
    }
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let mut candidates: Vec<String> = [
        "asctime",
        "epoch_s",
        "epoch_ms",
        "epoch_us",
        "epoch_ns",
        "dotnet_ticks",
        "filetime",
        "uptime",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if let Some(p) = crate::timestamp_issue::suggest::propose(&refs, &base) {
        candidates.insert(0, p.format);
    }
    candidates.into_iter().find(|format| {
        let mut p = base.with_format(format);
        if format == "uptime" {
            match modified_ms.and_then(|m| uptime_boot(texts, m)) {
                Some(boot) => p = p.with_boot(boot),
                None => return false,
            }
        }
        check(&reads(&p), modified_ms).accepted
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;
    /// 2026-10-14 10:00:00 UTC.
    const T0: i64 = 1_791_972_000_000;

    fn timed(ts: &[i64]) -> Vec<(u32, Option<i64>)> {
        ts.iter().enumerate().map(|(i, t)| (i as u32 + 1, Some(*t))).collect()
    }

    #[test]
    fn accepted_when_nearly_all_are_timed_forward_and_plausible() {
        let ok = check(&timed(&[T0, T0 + MIN, T0 + MIN - 2_000, T0 + 2 * MIN]), Some(T0 + 3 * MIN));
        assert!(ok.accepted, "a small reordering is fine: {ok:?}");
        assert_eq!((ok.first, ok.last, ok.backwards), (Some(T0), Some(T0 + 2 * MIN), 0));
        let mut gaps = timed(&[T0, T0 + MIN]);
        gaps.push((3, None));
        assert!(check(&gaps, None).reason.unwrap().contains("2 of 3"));
        let back = check(&timed(&[T0, T0 + 60 * MIN, T0, T0 + 61 * MIN, T0 + MIN]), Some(T0 + 62 * MIN));
        assert!(
            back.reason.as_deref().unwrap().contains("go back 2 times (first at line 3)"),
            "{back:?}"
        );
        let future = check(&timed(&[T0, T0 + MIN]), Some(T0 - 20 * 60 * MIN));
        assert!(future.reason.unwrap().contains("after the file was last written"));
        let old = check(&timed(&[1000, 2000]), None);
        assert!(old.reason.unwrap().contains("before 1990"));
        assert!(check(&[], None).reason.unwrap().contains("no event"));
        assert!(!check(&[(1, None)], None).plausible);
    }

    fn generic() -> Parser {
        crate::parser::builtin::generic()
    }

    /// Unusual timestamps the engine finds a format for itself (invented values).
    #[test]
    fn the_engine_suggests_a_format_for_unusual_timestamps() {
        let texts = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let modified = Some(T0 + 60 * MIN);
        let at = |ms: i64| ms.to_string();
        let cases: Vec<(Vec<String>, Option<&str>)> = vec![
            (texts(&["Tue Oct 14 10:00:03 2026", "Tue Oct 14 10:00:04 2026"]), Some("asctime")),
            (vec![at(T0), at(T0 + 1000)], None), // epoch ms: read already
            (vec![at(T0 / 1000), at(T0 / 1000 + 1)], None),
            (
                vec![
                    (T0 as i128 * 10_000 + 621_355_968_000_000_000).to_string(),
                    ((T0 + 5) as i128 * 10_000 + 621_355_968_000_000_000).to_string(),
                ],
                Some("dotnet_ticks"),
            ),
            (
                vec![
                    (T0 as i128 * 10_000 + 116_444_736_000_000_000).to_string(),
                    ((T0 + 5) as i128 * 10_000 + 116_444_736_000_000_000).to_string(),
                ],
                Some("filetime"),
            ),
            (texts(&["[    1.250000]", "[   12.345678]", "[ 3600.000001]"]), Some("uptime")),
            (texts(&["2026年10月14日 10時00分03秒", "2026年10月14日 10時00分04秒"]), None),
            (texts(&["14.10.2026_10:00:03", "14.10.2026_10:00:04"]), Some("%d.%m.%Y_%H:%M:%S")),
            (texts(&["no time here", "nor here"]), None),
            (vec![], None),
        ];
        for (i, (t, want)) in cases.iter().enumerate() {
            assert_eq!(suggest_format(&generic(), t, modified).as_deref(), *want, "case {i}: {t:?}");
        }
        // Uptime needs the modification time to anchor its boot.
        assert_eq!(suggest_format(&generic(), &texts(&["[ 1.0]", "[ 2.0]"]), None), None);
        assert_eq!(uptime_boot(&texts(&["[ 1.0]", "[ 2.5]"]), T0), Some(T0 - 2_500));
        assert_eq!(
            uptime_boot_time(&texts(&["[ 2.5]"]), T0).as_deref(),
            Some("2026-10-14T09:59:57.500Z")
        );
    }
}
