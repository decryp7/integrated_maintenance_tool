//! Timestamp parsing that copes with the many formats found in log files:
//! ISO 8601 / RFC 3339, RFC 2822, Serilog defaults, slash/dot separated dates
//! in either day-first or month-first order, month names, 12-hour clocks,
//! epoch seconds/milliseconds, plus user-supplied .NET or strftime formats.

use anyhow::{bail, Result};
use chrono::{DateTime, FixedOffset, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy)]
pub enum AssumedZone {
    Utc,
    Local,
    Fixed(FixedOffset),
}

#[derive(Debug, Clone)]
pub struct TimestampParser {
    custom: Vec<String>,
    zone: AssumedZone,
    /// None = decide from the value itself (falls back to month-first).
    day_first: Option<bool>,
}

impl Default for TimestampParser {
    fn default() -> Self {
        TimestampParser {
            custom: Vec::new(),
            zone: AssumedZone::Utc,
            day_first: None,
        }
    }
}

fn numeric_date_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)^
            (?P<a>\d{4}|\d{1,2})[-/.](?P<b>\d{1,2})[-/.](?P<c>\d{1,4})
            (?:(?:[\sT]+|:)(?P<time>\d{1,2}:\d{2}(?::\d{2})?(?:[.,]\d{1,9})?)\s*(?P<ampm>[AaPp][Mm])?)?
            \s*(?P<zone>Z|UTC|GMT|[+-]\d{2}:?\d{2}|[+-]\d{2})?\s*$",
        )
        .unwrap()
    })
}

fn named_month_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)^(?:[A-Za-z]{3,9},?\s+)?
            (?:
              (?P<d1>\d{1,2})[\s-](?P<m1>[A-Za-z]{3,9})[\s,-]+(?P<y1>\d{4})
            | (?P<m2>[A-Za-z]{3,9})\s+(?P<d2>\d{1,2}),?\s+(?P<y2>\d{4})
            )
            (?:[\sT,]+(?P<time>\d{1,2}:\d{2}(?::\d{2})?(?:[.,]\d{1,9})?)\s*(?P<ampm>[AaPp][Mm])?)?
            \s*(?P<zone>Z|UTC|GMT|[+-]\d{2}:?\d{2})?\s*$",
        )
        .unwrap()
    })
}

fn time_only_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(\d{1,2}:\d{2}:\d{2}(?:[.,]\d{1,9})?)\s*([AaPp][Mm])?$").unwrap())
}

impl TimestampParser {
    pub fn new(formats: &[String], zone: &str, day_first: Option<bool>) -> Result<Self> {
        let zone = match zone.trim().to_ascii_lowercase().as_str() {
            "" | "utc" | "z" => AssumedZone::Utc,
            "local" => AssumedZone::Local,
            other => match parse_offset(other) {
                Some(o) => AssumedZone::Fixed(o),
                None => bail!("assume_timezone must be \"UTC\", \"local\" or an offset like \"+08:00\", got \"{zone}\""),
            },
        };
        let custom = formats.iter().map(|f| to_strftime(f)).collect();
        Ok(TimestampParser { custom, zone, day_first })
    }

    pub fn day_first(&self) -> Option<bool> {
        self.day_first
    }

    pub fn with_day_first(&self, day_first: Option<bool>) -> Self {
        TimestampParser { day_first, ..self.clone() }
    }

    /// Parses a full timestamp into epoch milliseconds.
    pub fn parse(&self, s: &str) -> Option<i64> {
        let s = s.trim().trim_matches(|c| c == '[' || c == ']' || c == '"');
        if s.is_empty() {
            return None;
        }
        for fmt in &self.custom {
            if let Some(ms) = self.parse_with_format(s, fmt) {
                return Some(ms);
            }
        }
        if let Some(ms) = parse_epoch(s) {
            return Some(ms);
        }
        if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
            return Some(dt.timestamp_millis());
        }
        if let Some(c) = numeric_date_re().captures(s) {
            let a = &c["a"];
            let b: u32 = c["b"].parse().ok()?;
            let cc = &c["c"];
            let (y, m, d) = if a.len() == 4 {
                (a.parse::<i32>().ok()?, b, cc.parse::<u32>().ok()?)
            } else {
                // A year written last needs 2 or 4 digits ("1.2.3" is a version, not a date).
                if cc.len() != 2 && cc.len() != 4 {
                    return None;
                }
                let first: u32 = a.parse().ok()?;
                let year = expand_year(cc.parse().ok()?);
                let day_first = self.day_first.unwrap_or(first > 12);
                if day_first {
                    (year, b, first)
                } else {
                    (year, first, b)
                }
            };
            let date = NaiveDate::from_ymd_opt(y, m, d)?;
            return self.finish(
                date,
                c.name("time").map(|m| m.as_str()),
                c.name("ampm").map(|m| m.as_str()),
                c.name("zone").map(|m| m.as_str()),
            );
        }
        if let Some(c) = named_month_re().captures(s) {
            let (d, m, y) = match c.name("d1") {
                Some(d) => (d.as_str(), &c["m1"], &c["y1"]),
                None => (&c["d2"], &c["m2"], &c["y2"]),
            };
            let date = NaiveDate::from_ymd_opt(y.parse().ok()?, month_number(m)?, d.parse().ok()?)?;
            return self.finish(
                date,
                c.name("time").map(|m| m.as_str()),
                c.name("ampm").map(|m| m.as_str()),
                c.name("zone").map(|m| m.as_str()),
            );
        }
        if let Ok(dt) = DateTime::parse_from_rfc2822(s) {
            return Some(dt.timestamp_millis());
        }
        None
    }

    /// Parses a time of day ("10:41:22.318") on a given date. Used for console
    /// style logs that carry no date.
    pub fn parse_time_on(&self, s: &str, date: NaiveDate) -> Option<i64> {
        let s = s.trim();
        let c = time_only_re().captures(s)?;
        self.finish(date, Some(&c[1]), c.get(2).map(|m| m.as_str()), None)
    }

    fn parse_with_format(&self, s: &str, fmt: &str) -> Option<i64> {
        if let Ok(dt) = DateTime::parse_from_str(s, fmt) {
            return Some(dt.timestamp_millis());
        }
        if let Ok(ndt) = NaiveDateTime::parse_from_str(s, fmt) {
            return self.localize(ndt);
        }
        if let Ok(date) = NaiveDate::parse_from_str(s, fmt) {
            return self.localize(date.and_hms_opt(0, 0, 0)?);
        }
        None
    }

    fn finish(&self, date: NaiveDate, time: Option<&str>, ampm: Option<&str>, zone: Option<&str>) -> Option<i64> {
        let time = match time {
            Some(t) => parse_time(t, ampm)?,
            None => NaiveTime::from_hms_opt(0, 0, 0)?,
        };
        let ndt = date.and_time(time);
        match zone {
            Some(z) => {
                let offset = parse_offset(z)?;
                Some(offset.from_local_datetime(&ndt).single()?.timestamp_millis())
            }
            None => self.localize(ndt),
        }
    }

    fn localize(&self, ndt: NaiveDateTime) -> Option<i64> {
        match self.zone {
            AssumedZone::Utc => Some(ndt.and_utc().timestamp_millis()),
            AssumedZone::Local => Some(Local.from_local_datetime(&ndt).earliest()?.timestamp_millis()),
            AssumedZone::Fixed(o) => Some(o.from_local_datetime(&ndt).single()?.timestamp_millis()),
        }
    }
}

/// Looks at sample timestamps written as `a/b/yyyy` and decides whether the
/// first component is the day. Returns None when nothing disambiguates.
pub fn detect_day_first<'a>(samples: impl IntoIterator<Item = &'a str>) -> Option<bool> {
    for s in samples {
        if let Some(c) = numeric_date_re().captures(s.trim()) {
            if c["a"].len() == 4 {
                continue;
            }
            let a: u32 = c["a"].parse().ok()?;
            let b: u32 = c["b"].parse().ok()?;
            if a > 12 {
                return Some(true);
            }
            if b > 12 {
                return Some(false);
            }
        }
    }
    None
}

fn parse_time(t: &str, ampm: Option<&str>) -> Option<NaiveTime> {
    let (hms, frac) = match t.find(['.', ',']) {
        Some(i) => (&t[..i], &t[i + 1..]),
        None => (t, ""),
    };
    let mut parts = hms.split(':');
    let mut h: u32 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    let s: u32 = parts.next().map(|p| p.parse().ok()).unwrap_or(Some(0))?;
    if let Some(ampm) = ampm {
        let pm = ampm.eq_ignore_ascii_case("pm");
        if h == 12 {
            h = if pm { 12 } else { 0 };
        } else if pm {
            h += 12;
        }
    }
    let nanos = if frac.is_empty() {
        0
    } else {
        let digits: String = frac.chars().take(9).collect();
        let scale = 10u32.pow(9 - digits.len() as u32);
        digits.parse::<u32>().ok()? * scale
    };
    NaiveTime::from_hms_nano_opt(h, m, s.min(59), nanos)
}

fn parse_offset(z: &str) -> Option<FixedOffset> {
    let z = z.trim();
    if z.eq_ignore_ascii_case("z") || z.eq_ignore_ascii_case("utc") || z.eq_ignore_ascii_case("gmt") {
        return FixedOffset::east_opt(0);
    }
    let sign = match z.chars().next()? {
        '+' => 1,
        '-' => -1,
        _ => return None,
    };
    let digits: String = z[1..].chars().filter(|c| c.is_ascii_digit()).collect();
    let (h, m) = match digits.len() {
        2 => (digits.parse::<i32>().ok()?, 0),
        4 => (digits[..2].parse::<i32>().ok()?, digits[2..].parse::<i32>().ok()?),
        _ => return None,
    };
    FixedOffset::east_opt(sign * (h * 3600 + m * 60))
}

fn parse_epoch(s: &str) -> Option<i64> {
    let (int, frac) = match s.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (s, None),
    };
    if int.is_empty() || !int.bytes().all(|b| b.is_ascii_digit()) || frac.is_some_and(|f| !f.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    let v: f64 = s.parse().ok()?;
    // Only accept magnitudes that land between 2000 and 2100.
    let ms = match int.len() {
        10 => v * 1000.0,
        13 => v,
        16 => v / 1000.0,
        19 => v / 1_000_000.0,
        _ => return None,
    };
    (946_684_800_000.0..4_102_444_800_000.0).contains(&ms).then_some(ms as i64)
}

/// Epoch number from a JSON value (seconds or milliseconds by magnitude).
pub fn epoch_from_number(n: f64) -> Option<i64> {
    let ms = if n < 1e11 {
        n * 1000.0
    } else if n < 1e14 {
        n
    } else if n < 1e17 {
        n / 1000.0
    } else {
        n / 1e6
    };
    (946_684_800_000.0..4_102_444_800_000.0).contains(&ms).then_some(ms as i64)
}

fn expand_year(y: i32) -> i32 {
    if y < 100 {
        2000 + y
    } else {
        y
    }
}

fn month_number(m: &str) -> Option<u32> {
    let m = m.to_ascii_lowercase();
    let names = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    names.iter().position(|n| m.starts_with(n)).map(|i| i as u32 + 1)
}

/// Converts a .NET custom date format ("dd/MM/yyyy HH:mm:ss.fff zzz") to a
/// chrono strftime format. Strings containing '%' are assumed to be strftime
/// already.
pub fn to_strftime(fmt: &str) -> String {
    if fmt.contains('%') {
        return fmt.to_string();
    }
    let chars: Vec<char> = fmt.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let mut n = 1;
        while i + n < chars.len() && chars[i + n] == c {
            n += 1;
        }
        match c {
            '\'' | '"' => {
                let end = chars[i + 1..]
                    .iter()
                    .position(|&x| x == c)
                    .map(|p| i + 1 + p)
                    .unwrap_or(chars.len());
                out.extend(&chars[i + 1..end]);
                i = end + 1;
                continue;
            }
            '\\' if i + 1 < chars.len() => {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            'y' => out.push_str(if n <= 2 { "%y" } else { "%Y" }),
            'M' => out.push_str(match n {
                1 | 2 => "%m",
                3 => "%b",
                _ => "%B",
            }),
            'd' => out.push_str(match n {
                1 | 2 => "%d",
                3 => "%a",
                _ => "%A",
            }),
            'H' => out.push_str("%H"),
            'h' => out.push_str("%I"),
            'm' => out.push_str("%M"),
            's' => out.push_str("%S"),
            'f' | 'F' => {
                // ".fff" -> "%.f" (the dot is consumed by %.f)
                if out.ends_with('.') || out.ends_with(',') {
                    out.pop();
                }
                out.push_str("%.f");
            }
            't' => out.push_str("%p"),
            'z' => out.push_str(if n >= 3 { "%:z" } else { "%z" }),
            'K' => out.push_str("%:z"),
            _ => {
                for _ in 0..n {
                    out.push(c);
                }
            }
        }
        i += n;
    }
    out
}

pub fn utc_date_of(ms: i64) -> Option<NaiveDate> {
    Utc.timestamp_millis_opt(ms).single().map(|d| d.date_naive())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> TimestampParser {
        TimestampParser::default()
    }

    fn ms(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32, milli: u32) -> i64 {
        NaiveDate::from_ymd_opt(y, mo, d)
            .unwrap()
            .and_hms_milli_opt(h, mi, s, milli)
            .unwrap()
            .and_utc()
            .timestamp_millis()
    }

    #[test]
    fn iso_and_serilog_formats() {
        let want = ms(2026, 10, 14, 10, 41, 22, 318);
        assert_eq!(p().parse("2026-10-14T10:41:22.318Z"), Some(want));
        assert_eq!(p().parse("2026-10-14T10:41:22.3180000+00:00"), Some(want));
        assert_eq!(p().parse("2026-10-14 18:41:22.318 +08:00"), Some(want));
        assert_eq!(p().parse("2026-10-14 10:41:22,318"), Some(want));
        assert_eq!(p().parse("2026/10/14 10:41:22.318"), Some(want));
    }

    #[test]
    fn day_and_month_first() {
        let want = ms(2026, 4, 3, 13, 5, 0, 0);
        assert_eq!(p().with_day_first(Some(true)).parse("03/04/2026 13:05:00"), Some(want));
        assert_eq!(p().with_day_first(Some(false)).parse("04/03/2026 1:05:00 PM"), Some(want));
        assert_eq!(p().parse("23/10/2026 00:00:00"), Some(ms(2026, 10, 23, 0, 0, 0, 0)));
        assert_eq!(detect_day_first(["01/02/2026 10:00", "25/02/2026 10:00"]), Some(true));
        assert_eq!(detect_day_first(["02/25/2026 10:00"]), Some(false));
    }

    #[test]
    fn named_months_and_epoch() {
        assert_eq!(p().parse("14 Oct 2026 10:41:22"), Some(ms(2026, 10, 14, 10, 41, 22, 0)));
        assert_eq!(p().parse("Oct 14, 2026 10:41:22 AM"), Some(ms(2026, 10, 14, 10, 41, 22, 0)));
        assert_eq!(p().parse("1792000000"), Some(1_792_000_000_000));
        assert_eq!(p().parse("1792000000123"), Some(1_792_000_000_123));
        assert_eq!(p().parse("12345"), None);
    }

    #[test]
    fn custom_dotnet_format() {
        let parser = TimestampParser::new(&["dd.MM.yyyy HH:mm:ss.fff".to_string()], "UTC", None).unwrap();
        assert_eq!(parser.parse("14.10.2026 10:41:22.318"), Some(ms(2026, 10, 14, 10, 41, 22, 318)));
        assert_eq!(to_strftime("yyyy-MM-dd'T'HH:mm:ss.fffzzz"), "%Y-%m-%dT%H:%M:%S%.f%:z");
        assert_eq!(to_strftime("HH:mmK"), "%H:%M%:z");
        let k = TimestampParser::new(&["yyyy-MM-dd HH:mm:ssK".to_string()], "UTC", None).unwrap();
        assert_eq!(k.parse("2026-10-14 12:41:22+02:00"), Some(ms(2026, 10, 14, 10, 41, 22, 0)));
    }

    #[test]
    fn assumed_zone() {
        let parser = TimestampParser::new(&[], "+08:00", None).unwrap();
        assert_eq!(parser.parse("2026-10-14 18:41:22.318"), Some(ms(2026, 10, 14, 10, 41, 22, 318)));
    }

    #[test]
    fn zones_and_errors() {
        assert!(TimestampParser::new(&[], "Mars/Olympus", None).is_err());
        let local = TimestampParser::new(&[], "local", None).unwrap();
        assert!(local.parse("2026-10-14 10:00:00").is_some());
        let utc = TimestampParser::new(&[], "", None).unwrap();
        assert_eq!(utc.parse("2026-10-14 10:00:00"), Some(ms(2026, 10, 14, 10, 0, 0, 0)));
        assert_eq!(p().parse("2026-10-14 10:00:00 +0800"), Some(ms(2026, 10, 14, 2, 0, 0, 0)));
        assert_eq!(p().parse("2026-10-14 10:00:00 -05"), Some(ms(2026, 10, 14, 15, 0, 0, 0)));
        assert_eq!(p().parse("2026-10-14 10:00:00 GMT"), Some(ms(2026, 10, 14, 10, 0, 0, 0)));
        assert_eq!(parse_offset("x0800"), None);
        assert_eq!(parse_offset("+123"), None);
        assert_eq!(p().parse("   "), None);
        assert_eq!(p().parse("not a date"), None);
        assert_eq!(p().parse("2026-13-45 10:00:00"), None);
    }

    #[test]
    fn more_formats() {
        assert_eq!(p().parse("2026-10-14"), Some(ms(2026, 10, 14, 0, 0, 0, 0)));
        assert_eq!(p().parse("12/25/26 11:30 PM"), Some(ms(2026, 12, 25, 23, 30, 0, 0)));
        assert_eq!(p().parse("10/14/2026 12:05:00 AM"), Some(ms(2026, 10, 14, 0, 5, 0, 0)));
        assert_eq!(p().parse("Wed, 14 Oct 2026 10:41:22 +0000"), Some(ms(2026, 10, 14, 10, 41, 22, 0)));
        // RFC 2822 with a named zone or a two-digit year.
        assert_eq!(p().parse("Wed, 14 Oct 2026 10:41:22 EST"), Some(ms(2026, 10, 14, 15, 41, 22, 0)));
        assert_eq!(p().parse("Wed, 14 Oct 26 10:41:22 +0000"), Some(ms(2026, 10, 14, 10, 41, 22, 0)));
        assert_eq!(p().parse("[2026-10-14T10:41:22.318Z]"), Some(ms(2026, 10, 14, 10, 41, 22, 318)));
        assert_eq!(p().parse("1792000000.5"), Some(1_792_000_000_500));
        assert_eq!(p().parse("1792000000123456"), Some(1_792_000_000_123));
        assert_eq!(p().parse("1792000000123456789"), Some(1_792_000_000_123));
        assert_eq!(p().parse("17920000001"), None);
        assert_eq!(p().parse("1.2.3"), None);
        assert_eq!(epoch_from_number(1_792_000_000.0), Some(1_792_000_000_000));
        assert_eq!(epoch_from_number(1_792_000_000_123.0), Some(1_792_000_000_123));
        assert_eq!(epoch_from_number(1_792_000_000_123_000.0), Some(1_792_000_000_123));
        assert_eq!(epoch_from_number(1.792e18), Some(1_792_000_000_000));
        assert_eq!(epoch_from_number(5.0), None);
        let d = NaiveDate::from_ymd_opt(2026, 10, 14).unwrap();
        assert_eq!(p().parse_time_on("10:41:22,5", d), Some(ms(2026, 10, 14, 10, 41, 22, 500)));
        assert_eq!(p().parse_time_on("12:00:00 AM", d), Some(ms(2026, 10, 14, 0, 0, 0, 0)));
        assert_eq!(p().parse_time_on("not a time", d), None);
        assert_eq!(utc_date_of(ms(2026, 10, 14, 23, 0, 0, 0)), Some(d));
        assert_eq!(detect_day_first(["2026/01/02", "nope", "01/02/2026"]), None);
    }

    #[test]
    fn custom_formats() {
        let f = |fmt: &str, s: &str| TimestampParser::new(&[fmt.to_string()], "UTC", None).unwrap().parse(s);
        assert_eq!(
            f("%d/%m/%Y %H:%M:%S %z", "14/10/2026 18:00:00 +0800"),
            Some(ms(2026, 10, 14, 10, 0, 0, 0))
        );
        assert_eq!(f("yyyyMMdd", "20261014"), Some(ms(2026, 10, 14, 0, 0, 0, 0)));
        assert_eq!(
            f("dd-MMM-yy hh:mm:ss tt", "14-Oct-26 01:02:03 PM"),
            Some(ms(2026, 10, 14, 13, 2, 3, 0))
        );
        assert_eq!(
            f("yyyy-MM-dd'T'HH:mm:ssK", "2026-10-14T10:00:00+02:00"),
            Some(ms(2026, 10, 14, 8, 0, 0, 0))
        );
        assert_eq!(to_strftime("dddd, MMMM d yyyy 'at' h:mm zz"), "%A, %B %d %Y at %I:%M %z");
        assert_eq!(to_strftime("yyyy-MM-dd\\THH"), "%Y-%m-%dT%H");
        assert_eq!(to_strftime("ddd MMM HH:mm:ss,FFF"), "%a %b %H:%M:%S%.f");
        assert_eq!(to_strftime("'unterminated"), "unterminated");
    }
}
