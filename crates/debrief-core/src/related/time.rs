//! When a screenshot was taken, from the evidence in order: a date and time on the screen, a time of day on the
//! screen (with a date from the other evidence), the EXIF capture time, a time in the file name, the file's
//! modification time. Pure; times without a zone are read in the profile's zone.

use super::{ExifStamp, ImageTime, TimeEvidence, TimeSource};
use crate::api::SourceInfo;
use crate::time::TimestampParser;
use chrono::{DateTime, FixedOffset, Local, NaiveDate, TimeZone, Utc};
use regex::Regex;
use std::sync::OnceLock;

/// Precision of a time with seconds, without seconds, and of a file's modification time.
const SECOND: i64 = 1_000;
const MINUTE: i64 = 60_000;
const MTIME: i64 = 5_000;

fn file_name_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        // Screenshot 2026-10-21 091403, Screenshot_20261021-091403, Screenshot 2026-10-21 at 09.14.03,
        // IMG_20261021_091403, 2026-10-21_09-14-03.
        Regex::new(
            r"(?x)(?P<y>20\d{2}|19\d{2})[-_.]?(?P<mo>[01]\d)[-_.]?(?P<d>[0-3]\d)
              (?:[\sT_-]+(?:at[\s_]+)?)(?P<h>[0-2]\d)[-_.:h]?(?P<mi>[0-5]\d)[-_.:m]?(?P<s>[0-5]\d)",
        )
        .unwrap_or_else(|_| unreachable!("static pattern"))
    })
}

fn hhmm_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^(\d{1,2}:\d{2})\s*([AaPp][Mm])?$").unwrap_or_else(|_| unreachable!("static pattern")))
}

fn parser(tz: &str) -> TimestampParser {
    TimestampParser::new(&[], tz, None).unwrap_or_default()
}

/// The calendar date of `ms` in the zone `tz` (`UTC`, `local` or `+HH:MM`).
pub fn date_in_zone(ms: i64, tz: &str) -> Option<NaiveDate> {
    let utc: DateTime<Utc> = Utc.timestamp_millis_opt(ms).single()?;
    Some(match tz.trim().to_ascii_lowercase().as_str() {
        "" | "utc" | "z" => utc.date_naive(),
        "local" => utc.with_timezone(&Local).date_naive(),
        other => utc.with_timezone(&parse_offset(other)?).date_naive(),
    })
}

fn parse_offset(s: &str) -> Option<FixedOffset> {
    let b = s.as_bytes();
    let sign = match b.first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let digits: String = s[1..].chars().filter(char::is_ascii_digit).collect();
    if digits.len() != 4 {
        return None;
    }
    let h: i32 = digits[..2].parse().ok()?;
    let m: i32 = digits[2..].parse().ok()?;
    FixedOffset::east_opt(sign * (h * 3600 + m * 60))
}

/// A shown time of day with seconds and how exact it is: `10:41` becomes `10:41:00` (± 1 min).
fn time_of_day(text: &str) -> (String, i64) {
    match hhmm_re().captures(text.trim()) {
        Some(c) => (
            format!("{}:00{}", &c[1], c.get(2).map(|m| format!(" {}", m.as_str())).unwrap_or_default()),
            MINUTE,
        ),
        None => (text.trim().to_string(), SECOND),
    }
}

/// Seconds shown in a dated text? (`2026-10-21 10:41` is ± 1 min.)
fn dated_precision(text: &str) -> i64 {
    let has_seconds = Regex::new(r"\d{1,2}:\d{2}:\d{2}").map(|r| r.is_match(text)).unwrap_or(false);
    if has_seconds {
        SECOND
    } else {
        MINUTE
    }
}

fn exif_ms(e: &ExifStamp, tz: &str) -> Option<i64> {
    let iso = e.local.replacen(' ', "T", 1);
    if let Some(o) = e.offset.as_deref() {
        if let Ok(dt) = DateTime::parse_from_rfc3339(&format!("{iso}{o}")) {
            return Some(dt.timestamp_millis());
        }
    }
    parser(tz).parse(&e.local)
}

fn file_name_ms(name: &str, tz: &str) -> Option<(i64, String)> {
    let c = file_name_re().captures(name)?;
    let text = format!("{}-{}-{} {}:{}:{}", &c["y"], &c["mo"], &c["d"], &c["h"], &c["mi"], &c["s"]);
    Some((
        parser(tz).parse(&text)?,
        c.get(0).map(|m| m.as_str().to_string()).unwrap_or_default(),
    ))
}

/// The one date the loaded data covers (in `tz`), when it covers exactly one.
fn data_day((from, to): (Option<i64>, Option<i64>), tz: &str) -> Option<NaiveDate> {
    let (a, b) = (date_in_zone(from?, tz)?, date_in_zone(to?, tz)?);
    (a == b).then_some(a)
}

/// Decides when the screenshot was taken (see the module docs), or `None` when nothing tells.
pub fn resolve(ev: &TimeEvidence, tz: &str, data: (Option<i64>, Option<i64>)) -> Option<ImageTime> {
    let p = parser(tz);
    for s in ev.shown.iter().filter(|s| s.has_date) {
        if let Some(ms) = p.parse(&s.text) {
            return Some(ImageTime {
                at_ms: ms,
                precision_ms: dated_precision(&s.text),
                source: TimeSource::Screen,
                text: s.text.clone(),
            });
        }
    }
    let exif = ev.exif.as_ref().and_then(|e| exif_ms(e, tz).map(|ms| (ms, e.local.clone())));
    let named = ev.file_name.as_deref().and_then(|n| file_name_ms(n, tz));
    let day = exif
        .as_ref()
        .map(|(ms, _)| *ms)
        .or_else(|| named.as_ref().map(|(ms, _)| *ms))
        .or(ev.file_modified_ms)
        .and_then(|ms| date_in_zone(ms, tz))
        .or_else(|| data_day(data, tz));
    if let Some(day) = day {
        for s in ev.shown.iter().filter(|s| !s.has_date) {
            let (text, precision) = time_of_day(&s.text);
            if let Some(ms) = p.parse_time_on(&text, day) {
                return Some(ImageTime {
                    at_ms: ms,
                    precision_ms: precision,
                    source: TimeSource::ScreenTimeOfDay,
                    text: s.text.clone(),
                });
            }
        }
    }
    if let Some((ms, text)) = exif {
        return Some(ImageTime {
            at_ms: ms,
            precision_ms: SECOND,
            source: TimeSource::Exif,
            text,
        });
    }
    if let Some((ms, text)) = named {
        return Some(ImageTime {
            at_ms: ms,
            precision_ms: SECOND,
            source: TimeSource::FileName,
            text,
        });
    }
    ev.file_modified_ms.map(|ms| ImageTime {
        at_ms: ms,
        precision_ms: MTIME,
        source: TimeSource::FileModified,
        text: String::new(),
    })
}

fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(str::to_lowercase)
        .collect()
}

/// The clock offset of the screenshot's machine: a source with an offset whose name, path or group names one of
/// `machines`. Its logs were shifted by it, so the screen's clock is too.
pub fn machine_offset(machines: &[String], sources: &[SourceInfo]) -> Option<(String, i64)> {
    for m in machines {
        let m = m.to_lowercase();
        if m.len() < 3 {
            continue;
        }
        let hit = sources.iter().find(|s| {
            s.offset_ms != 0
                && [&s.name, &s.path, &s.group].iter().any(|t| {
                    words(t).contains(&m) || t.to_lowercase().contains(&format!("{m}.")) || t.to_lowercase().contains(&format!("{m}/"))
                })
        });
        if let Some(s) = hit {
            return Some((s.name.clone(), s.offset_ms));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::super::ShownTime;
    use super::*;

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> i64 {
        NaiveDate::from_ymd_opt(y, mo, d)
            .and_then(|d| d.and_hms_opt(h, mi, s))
            .map(|t| t.and_utc().timestamp_millis())
            .unwrap_or_default()
    }

    fn shown(text: &str, has_date: bool) -> ShownTime {
        ShownTime {
            text: text.into(),
            has_date,
        }
    }

    #[test]
    fn a_dated_time_on_the_screen_wins() {
        let ev = TimeEvidence {
            shown: vec![shown("10:00:00", false), shown("2026-10-21 10:41:22", true)],
            exif: Some(ExifStamp {
                local: "2026-10-21 09:00:00".into(),
                offset: None,
            }),
            file_name: Some("Screenshot 2026-10-20 080000.png".into()),
            file_modified_ms: Some(1),
        };
        let t = resolve(&ev, "UTC", (None, None)).unwrap();
        assert_eq!(
            (t.at_ms, t.precision_ms, t.source),
            (utc(2026, 10, 21, 10, 41, 22), SECOND, TimeSource::Screen)
        );
        // In the profile's zone, and ± 1 minute without seconds.
        let t = resolve(
            &TimeEvidence {
                shown: vec![shown("2026-10-21 10:41", true)],
                ..TimeEvidence::default()
            },
            "+02:00",
            (None, None),
        )
        .unwrap();
        assert_eq!((t.at_ms, t.precision_ms), (utc(2026, 10, 21, 8, 41, 0), MINUTE));
    }

    #[test]
    fn a_time_of_day_takes_its_date_from_the_other_evidence() {
        let exif = ExifStamp {
            local: "2026-10-21 09:00:00".into(),
            offset: None,
        };
        let ev = TimeEvidence {
            shown: vec![shown("10:41", false)],
            exif: Some(exif.clone()),
            ..TimeEvidence::default()
        };
        let t = resolve(&ev, "UTC", (None, None)).unwrap();
        assert_eq!(
            (t.at_ms, t.precision_ms, t.source),
            (utc(2026, 10, 21, 10, 41, 0), MINUTE, TimeSource::ScreenTimeOfDay)
        );
        let pm = TimeEvidence {
            shown: vec![shown("10:41:05 PM", false)],
            file_name: Some("IMG_20261021_091403.jpg".into()),
            ..TimeEvidence::default()
        };
        assert_eq!(resolve(&pm, "UTC", (None, None)).unwrap().at_ms, utc(2026, 10, 21, 22, 41, 5));
        // Only the data's day to go on (it covers one day).
        let only = TimeEvidence {
            shown: vec![shown("07:00:01", false)],
            ..TimeEvidence::default()
        };
        let data = (Some(utc(2026, 3, 4, 1, 0, 0)), Some(utc(2026, 3, 4, 23, 0, 0)));
        assert_eq!(resolve(&only, "UTC", data).unwrap().at_ms, utc(2026, 3, 4, 7, 0, 1));
        // Data over several days and nothing else: no time at all.
        assert_eq!(resolve(&only, "UTC", (Some(0), Some(utc(2026, 3, 4, 1, 0, 0)))), None);
    }

    #[test]
    fn then_exif_then_the_file_name_then_the_file_time() {
        let mut ev = TimeEvidence {
            exif: Some(ExifStamp {
                local: "2026-10-21 09:14:03".into(),
                offset: Some("+02:00".into()),
            }),
            file_name: Some("Screenshot 2026-10-21 at 11.00.00.png".into()),
            file_modified_ms: Some(utc(2026, 10, 22, 0, 0, 0)),
            ..TimeEvidence::default()
        };
        let t = resolve(&ev, "UTC", (None, None)).unwrap();
        assert_eq!((t.at_ms, t.source), (utc(2026, 10, 21, 7, 14, 3), TimeSource::Exif));
        ev.exif = None;
        let t = resolve(&ev, "UTC", (None, None)).unwrap();
        assert_eq!((t.at_ms, t.source), (utc(2026, 10, 21, 11, 0, 0), TimeSource::FileName));
        assert_eq!(t.text, "2026-10-21 at 11.00.00");
        ev.file_name = Some("bug.png".into());
        let t = resolve(&ev, "UTC", (None, None)).unwrap();
        assert_eq!(
            (t.at_ms, t.precision_ms, t.source),
            (utc(2026, 10, 22, 0, 0, 0), MTIME, TimeSource::FileModified)
        );
        assert_eq!(resolve(&TimeEvidence::default(), "UTC", (None, None)), None);
        for name in [
            "Screenshot_20261021-091403.png",
            "2026-10-21_09-14-03.png",
            "Screenshot 2026-10-21 091403.png",
        ] {
            assert_eq!(
                file_name_ms(name, "UTC").map(|(ms, _)| ms),
                Some(utc(2026, 10, 21, 9, 14, 3)),
                "{name}"
            );
        }
        assert_eq!(file_name_ms("report-v2.10.3.png", "UTC"), None);
    }

    #[test]
    fn zones_and_machine_offsets() {
        assert_eq!(
            date_in_zone(utc(2026, 1, 1, 23, 30, 0), "+02:00"),
            NaiveDate::from_ymd_opt(2026, 1, 2)
        );
        assert_eq!(
            date_in_zone(utc(2026, 1, 1, 0, 30, 0), "-0100"),
            NaiveDate::from_ymd_opt(2025, 12, 31)
        );
        assert_eq!(date_in_zone(0, "Mars"), None);
        assert!(date_in_zone(0, "local").is_some());
        assert_eq!(parse_offset("+2"), None);
        let sources = vec![
            super::super::tests::source(1, "web01.log", 0, None, None),
            super::super::tests::source(2, "web02-app.log", -3_000, None, None),
        ];
        assert_eq!(machine_offset(&["WEB02".into()], &sources), Some(("web02-app.log".into(), -3_000)));
        assert_eq!(machine_offset(&["web01".into()], &sources), None, "no offset");
        assert_eq!(machine_offset(&["x".into()], &sources), None);
    }
}
