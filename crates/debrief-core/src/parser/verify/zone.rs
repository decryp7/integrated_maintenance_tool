//! The time zone of a file whose timestamps may carry no offset (ADR 0032), decided in this order:
//!
//! 1. the offsets the timestamps carry (then the zone does not matter);
//! 2. the offset other files of the same machine carry;
//! 3. the file's modification time against its last event (a log is written until it is closed);
//! 4. textual hints in the file (`UTC+09:00`, `JST`, `CEST`);
//! 5. otherwise the user is asked once, with a suggestion.
//!
//! Pure: the evidence is gathered by the caller.

use crate::user_regex;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// Where a zone came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ZoneSource {
    /// The timestamps carry their offsets.
    Timestamps,
    /// Other files of the same machine carry this offset.
    SameMachine,
    /// The file's modification time against its last event.
    ModificationTime,
    /// A zone named in the file's text.
    TextHint,
    /// Nothing decided it: the user is asked.
    Asked,
    /// The user set it.
    User,
}

impl ZoneSource {
    /// The name written to `[format] timezone_source`.
    pub fn as_str(self) -> &'static str {
        match self {
            ZoneSource::Timestamps => "timestamps",
            ZoneSource::SameMachine => "same-machine",
            ZoneSource::ModificationTime => "modification-time",
            ZoneSource::TextHint => "text-hint",
            ZoneSource::Asked => "asked",
            ZoneSource::User => "user",
        }
    }
}

/// The zone the engine settled on, why and how sure it is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZoneGuess {
    /// `assume_timezone`: `"UTC"` or an offset like `"+09:00"`; with [`ZoneSource::Asked`], the suggestion.
    pub offset: String,
    pub source: ZoneSource,
    /// 0..1.
    pub confidence: f64,
    /// What it rests on, for the user ("the last event is 9 h after the file's modification time").
    pub detail: String,
}

/// What the zone is decided from.
#[derive(Debug, Clone, Default)]
pub struct ZoneEvidence<'a> {
    /// The timestamp texts of some events.
    pub timestamp_texts: &'a [String],
    /// Offsets (minutes east of UTC) other files of the same machine carry.
    pub machine_offsets: &'a [i32],
    /// The file's modification time (ms since the epoch).
    pub modified_ms: Option<i64>,
    /// The last event's wall-clock time read as if it were UTC (ms).
    pub last_wall_ms: Option<i64>,
    /// Text of the file to look for zone names in.
    pub text: &'a str,
}

/// `+09:00` / `-05:30` / `UTC` for minutes east of UTC.
pub fn offset_text(minutes: i32) -> String {
    if minutes == 0 {
        return "UTC".into();
    }
    let sign = if minutes < 0 { '-' } else { '+' };
    format!("{sign}{:02}:{:02}", minutes.abs() / 60, minutes.abs() % 60)
}

/// Minutes east of UTC of `UTC`, `Z` or `+hh:mm`; `None` for `local` or anything else.
pub fn offset_minutes(zone: &str) -> Option<i32> {
    let z = zone.trim();
    if z.is_empty() || z.eq_ignore_ascii_case("utc") || z.eq_ignore_ascii_case("z") || z.eq_ignore_ascii_case("gmt") {
        return Some(0);
    }
    let sign = match z.chars().next()? {
        '+' => 1,
        '-' => -1,
        _ => return None,
    };
    let digits: String = z[1..].chars().filter(|c| c.is_ascii_digit()).collect();
    let (h, m): (i32, i32) = match digits.len() {
        1 | 2 => (digits.parse().ok()?, 0),
        3 => (digits[..1].parse().ok()?, digits[1..].parse().ok()?),
        4 => (digits[..2].parse().ok()?, digits[2..].parse().ok()?),
        _ => return None,
    };
    (h <= 14 && m < 60).then_some(sign * (h * 60 + m))
}

fn trailing_offset_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| user_regex::compile(r"\d\s?(?:Z|UTC|GMT|[+-]\d{2}:?\d{2})\]?$", false).expect("valid regex"))
}

fn hint_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        user_regex::compile(
            r"\b(?:(?:UTC|GMT)\s?(?P<sign>[+-])(?P<h>\d{1,2})(?::?(?P<m>\d{2}))?|(?P<abbr>JST|KST|SGT|HKT|IST|CET|CEST|EET|EEST|WET|WEST|BST|PST|PDT|MST|MDT|EST|EDT|AEST|AEDT|NZST|NZDT))\b",
            false,
        )
        .expect("valid regex")
    })
}

/// Minutes east of UTC of a zone abbreviation (only unambiguous ones).
fn abbreviation(a: &str) -> Option<i32> {
    Some(match a {
        "JST" | "KST" => 540,
        "SGT" | "HKT" => 480,
        "IST" => 330,
        "CET" | "WEST" | "BST" => 60,
        "CEST" | "EET" => 120,
        "EEST" => 180,
        "WET" => 0,
        "PST" => -480,
        "PDT" | "MST" => -420,
        "MDT" => -360,
        "EST" => -300,
        "EDT" => -240,
        "AEST" => 600,
        "AEDT" => 660,
        "NZST" => 720,
        "NZDT" => 780,
        _ => return None,
    })
}

/// The most common value of `values` (the smallest on ties).
fn most_common(values: impl IntoIterator<Item = i32>) -> Option<(i32, usize, usize)> {
    let mut counts = std::collections::BTreeMap::<i32, usize>::new();
    let mut total = 0;
    for v in values {
        *counts.entry(v).or_default() += 1;
        total += 1;
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
        .map(|(v, n)| (v, n, total))
}

fn written_offset_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        user_regex::compile(r"\d{1,2}:\d{2}:\d{2}(?:[.,]\d{1,9})?\s?(?P<z>Z|[+-]\d{2}:?\d{2})(?:\D|$)", false).expect("valid regex")
    })
}

/// The offsets (minutes east of UTC) written after the times of the first lines of a text (another file of the same
/// machine, for [`ZoneEvidence::machine_offsets`]).
pub fn offsets_in(text: &str) -> Vec<i32> {
    text.lines()
        .take(200)
        .filter_map(|l| written_offset_re().captures(l).and_then(|c| offset_minutes(&c["z"])))
        .collect()
}

/// Decides the zone (see the module documentation).
pub fn detect(ev: &ZoneEvidence) -> ZoneGuess {
    let texts = ev.timestamp_texts;
    if !texts.is_empty() {
        let with = texts.iter().filter(|t| trailing_offset_re().is_match(t.trim())).count();
        if with * 10 >= texts.len() * 9 {
            return ZoneGuess {
                offset: "UTC".into(),
                source: ZoneSource::Timestamps,
                confidence: 1.0,
                detail: "the timestamps carry their offsets".into(),
            };
        }
    }
    if let Some((m, n, total)) = most_common(ev.machine_offsets.iter().copied()) {
        return ZoneGuess {
            offset: offset_text(m),
            source: ZoneSource::SameMachine,
            confidence: if n == total { 0.8 } else { 0.6 },
            detail: format!("{n} of {total} other files of this machine write {}", offset_text(m)),
        };
    }
    let mut suggestion = None;
    if let (Some(modified), Some(last)) = (ev.modified_ms, ev.last_wall_ms) {
        // The last line's wall clock minus the true time it was written: the zone's offset (to the quarter hour).
        let diff_min = (last - modified) as f64 / 60_000.0;
        let quarter = (diff_min / 15.0).round() * 15.0;
        if quarter.abs() <= 14.0 * 60.0 {
            let residual = (diff_min - quarter).abs();
            let guess = offset_text(quarter as i32);
            if residual <= 5.0 {
                return ZoneGuess {
                    offset: guess.clone(),
                    source: ZoneSource::ModificationTime,
                    confidence: 0.6,
                    detail: format!("the last event's clock is {guess} against the file's modification time (within {residual:.0} min)"),
                };
            }
            suggestion = Some(guess);
        }
    }
    if let Some(c) = hint_re().captures(ev.text) {
        let minutes = match c.name("abbr") {
            Some(a) => abbreviation(a.as_str()),
            None => {
                let h: i32 = c["h"].parse().unwrap_or(99);
                let m: i32 = c.name("m").and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
                (h <= 14 && m < 60).then(|| if &c["sign"] == "-" { -(h * 60 + m) } else { h * 60 + m })
            }
        };
        if let Some(minutes) = minutes {
            return ZoneGuess {
                offset: offset_text(minutes),
                source: ZoneSource::TextHint,
                confidence: 0.4,
                detail: format!("the file mentions \"{}\"", &c[0]),
            };
        }
    }
    ZoneGuess {
        offset: suggestion.clone().unwrap_or_else(|| "UTC".into()),
        source: ZoneSource::Asked,
        confidence: 0.0,
        detail: match suggestion {
            Some(s) => format!("nothing in the file decides the zone; the modification time suggests {s}"),
            None => "nothing in the file decides the zone".into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = 3_600_000;

    fn texts(t: &[&str]) -> Vec<String> {
        t.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn offsets_written_in_another_file() {
        let text = "2026-10-14 10:00:00.000+09:00 [INF] a\n2026-10-14T10:00:01Z b\nno time\n10:00:02 -0500 c\n2026-10-14 10:00:03 d\n";
        assert_eq!(offsets_in(text), [540, 0, -300]);
        assert!(offsets_in("").is_empty());
    }

    #[test]
    fn offsets_render_and_parse() {
        for (m, t) in [(0, "UTC"), (540, "+09:00"), (-330, "-05:30"), (345, "+05:45")] {
            assert_eq!(offset_text(m), t);
            assert_eq!(offset_minutes(t), Some(m));
        }
        assert_eq!(offset_minutes("+0900"), Some(540));
        assert_eq!(offset_minutes("-5"), Some(-300));
        assert_eq!(offset_minutes(""), Some(0));
        assert_eq!(offset_minutes("local"), None);
        assert_eq!(offset_minutes("+15:00"), None);
        assert_eq!(offset_minutes("+123456"), None);
    }

    /// The order of the evidence, one case each (invented values).
    #[test]
    fn the_zone_is_decided_in_order() {
        let zoned = texts(&["2026-10-14 10:00:00.000+09:00", "2026-10-14T10:00:01Z", "2026-10-14 10:00:02 +0900"]);
        let naive = texts(&["2026-10-14 10:00:00", "2026-10-14 10:00:01"]);
        let modified = 1_791_972_000_000;
        let cases: Vec<(ZoneEvidence, &str, ZoneSource)> = vec![
            (
                ZoneEvidence {
                    timestamp_texts: &zoned,
                    machine_offsets: &[60],
                    ..Default::default()
                },
                "UTC",
                ZoneSource::Timestamps,
            ),
            (
                ZoneEvidence {
                    timestamp_texts: &naive,
                    machine_offsets: &[540, 540, 60],
                    modified_ms: Some(modified),
                    last_wall_ms: Some(modified),
                    ..Default::default()
                },
                "+09:00",
                ZoneSource::SameMachine,
            ),
            (
                ZoneEvidence {
                    timestamp_texts: &naive,
                    modified_ms: Some(modified),
                    last_wall_ms: Some(modified + 9 * HOUR - 90_000),
                    text: "service JST",
                    ..Default::default()
                },
                "+09:00",
                ZoneSource::ModificationTime,
            ),
            (
                ZoneEvidence {
                    timestamp_texts: &naive,
                    modified_ms: Some(modified),
                    last_wall_ms: Some(modified - 2 * HOUR - 37 * 60_000),
                    text: "Host clock: UTC-05:00 (configured)",
                    ..Default::default()
                },
                "-05:00",
                ZoneSource::TextHint,
            ),
            (
                ZoneEvidence {
                    text: "scheduler started, zone CEST",
                    ..Default::default()
                },
                "+02:00",
                ZoneSource::TextHint,
            ),
            (
                ZoneEvidence {
                    timestamp_texts: &naive,
                    modified_ms: Some(modified),
                    last_wall_ms: Some(modified + 3 * HOUR + 38 * 60_000),
                    ..Default::default()
                },
                "+03:45",
                ZoneSource::Asked,
            ),
            (ZoneEvidence::default(), "UTC", ZoneSource::Asked),
        ];
        for (i, (ev, offset, source)) in cases.iter().enumerate() {
            let g = detect(ev);
            assert_eq!((g.offset.as_str(), g.source), (*offset, *source), "case {i}: {g:?}");
            assert!(!g.detail.is_empty());
        }
        // A machine whose files disagree is less certain.
        let g = detect(&ZoneEvidence {
            machine_offsets: &[60, 60, 120],
            ..Default::default()
        });
        assert_eq!((g.offset.as_str(), g.confidence), ("+01:00", 0.6));
        assert_eq!(ZoneSource::ModificationTime.as_str(), "modification-time");
        for s in [
            ZoneSource::Timestamps,
            ZoneSource::SameMachine,
            ZoneSource::TextHint,
            ZoneSource::Asked,
            ZoneSource::User,
        ] {
            assert!(!s.as_str().is_empty());
        }
        assert_eq!(abbreviation("XYZ"), None);
        // An impossible hint is ignored.
        assert_eq!(
            detect(&ZoneEvidence {
                text: "UTC+99",
                ..Default::default()
            })
            .source,
            ZoneSource::Asked
        );
    }
}
