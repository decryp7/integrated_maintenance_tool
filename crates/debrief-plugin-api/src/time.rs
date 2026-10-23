//! Timestamp parsing that copes with the many formats found in log files:
//! ISO 8601 / RFC 3339, RFC 2822, Serilog defaults, slash/dot separated dates
//! in either day-first or month-first order, month names, 12-hour clocks,
//! epoch seconds/milliseconds, plus user-supplied .NET or strftime formats.
//! Also Japanese dates (`2026年10月14日 10時00分03秒`) and, by name in the formats, C `asctime`
//! (`Tue Oct 14 10:00:03 2026`, format `asctime`) and numeric clocks: `epoch_s`, `epoch_ms`, `epoch_us`, `epoch_ns`, `dotnet_ticks`, `filetime` and `uptime`
//! (seconds since boot, `[ 12.345]`, anchored at a boot time).

use anyhow::{bail, Result};
use chrono::{DateTime, Datelike, FixedOffset, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use regex::Regex;
use std::sync::OnceLock;

/// The time zone assumed for timestamps that carry no offset.
#[derive(Debug, Clone, Copy)]
pub enum AssumedZone {
    /// UTC.
    Utc,
    /// The machine's local time zone.
    Local,
    /// A fixed offset from UTC.
    Fixed(FixedOffset),
}

/// Parses timestamp text in many log formats to milliseconds since the Unix epoch (UTC).
#[derive(Debug, Clone)]
pub struct TimestampParser {
    custom: Vec<String>,
    /// Numeric clocks named in the formats, tried before them.
    clocks: Vec<NumberClock>,
    /// The formats name `asctime`.
    asctime: bool,
    zone: AssumedZone,
    /// None = decide from the value itself (falls back to month-first).
    day_first: Option<bool>,
    /// The boot time (ms since the epoch) `uptime` values count from.
    boot_ms: i64,
    /// The time formats without a year are read near (the file's modification time); `None` = now.
    reference_ms: Option<i64>,
}

/// A timestamp written as one number, named in `timestamp_formats`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberClock {
    /// Seconds since 1970-01-01 UTC (with an optional fraction).
    EpochSeconds,
    /// Milliseconds since the Unix epoch.
    EpochMillis,
    /// Microseconds since the Unix epoch.
    EpochMicros,
    /// Nanoseconds since the Unix epoch.
    EpochNanos,
    /// .NET `DateTime.Ticks`: 100 ns since 0001-01-01.
    DotnetTicks,
    /// Windows FILETIME: 100 ns since 1601-01-01.
    FileTime,
    /// Seconds since boot (Linux `dmesg` `[   12.345678]`).
    Uptime,
}

impl NumberClock {
    /// The clock a format names (`epoch_ms`, `dotnet_ticks`, `filetime`, `uptime`...), ignoring case.
    pub fn named(name: &str) -> Option<NumberClock> {
        Some(match name.trim().to_ascii_lowercase().as_str() {
            "epoch" | "epoch_s" | "unix" | "unix_s" => NumberClock::EpochSeconds,
            "epoch_ms" | "unix_ms" => NumberClock::EpochMillis,
            "epoch_us" | "unix_us" => NumberClock::EpochMicros,
            "epoch_ns" | "unix_ns" => NumberClock::EpochNanos,
            "ticks" | "dotnet_ticks" => NumberClock::DotnetTicks,
            "filetime" => NumberClock::FileTime,
            "uptime" => NumberClock::Uptime,
            _ => return None,
        })
    }

    /// Milliseconds since the Unix epoch of `s` (an uptime counts from `boot_ms`).
    pub fn read(self, s: &str, boot_ms: i64) -> Option<i64> {
        let s = s.trim().trim_matches(|c: char| c == '[' || c == ']' || c.is_whitespace());
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit() || b == b'.') || s.bytes().filter(|&b| b == b'.').count() > 1 {
            return None;
        }
        // 100 ns units between 0001-01-01 / 1601-01-01 and the Unix epoch.
        const TICKS_TO_UNIX: i128 = 621_355_968_000_000_000;
        const FILETIME_TO_UNIX: i128 = 116_444_736_000_000_000;
        let whole = |s: &str| s.split('.').next().unwrap_or("").parse::<i128>().ok();
        let ms: i128 = match self {
            NumberClock::EpochSeconds => (s.parse::<f64>().ok()? * 1000.0) as i128,
            NumberClock::EpochMillis => s.parse::<f64>().ok()? as i128,
            NumberClock::EpochMicros => whole(s)? / 1000,
            NumberClock::EpochNanos => whole(s)? / 1_000_000,
            NumberClock::DotnetTicks => (whole(s)? - TICKS_TO_UNIX) / 10_000,
            NumberClock::FileTime => (whole(s)? - FILETIME_TO_UNIX) / 10_000,
            NumberClock::Uptime => boot_ms as i128 + (s.parse::<f64>().ok()? * 1000.0) as i128,
        };
        // Years 1970..2200 only: anything else is not this clock.
        (0..7_258_118_400_000).contains(&ms).then_some(ms as i64)
    }
}

impl Default for TimestampParser {
    fn default() -> Self {
        TimestampParser {
            custom: Vec::new(),
            clocks: Vec::new(),
            asctime: false,
            zone: AssumedZone::Utc,
            day_first: None,
            boot_ms: 0,
            reference_ms: None,
        }
    }
}

fn numeric_date_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)^
            (?P<a>\d{4}|\d{1,2})[-/.](?P<b>\d{1,2})[-/.](?P<c>\d{1,4})
            (?:(?:[\sT]+|[:,]\s*)(?P<time>\d{1,2}:\d{2}(?::\d{2})?(?:[.,]\d{1,9})?)\s*(?P<ampm>[AaPp][Mm])?)?
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

/// `2026年10月14日 10時00分03秒`, `2026年10月14日(火) 午後 3:04:05.120`, `2026/10/14 10時00分`.
fn japanese_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)^(?P<y>\d{4})\s*(?:年|/|-)\s*(?P<m>\d{1,2})\s*(?:月|/|-)\s*(?P<d>\d{1,2})\s*日?
            \s*(?:[(（][^)）]{1,3}[)）])?\s*(?P<ampm>午前|午後)?\s*
            (?P<h>\d{1,2})\s*(?:時|:)\s*(?P<mi>\d{1,2})\s*(?:分|:)?\s*(?:(?P<s>\d{1,2})(?:[.,](?P<f>\d{1,9}))?\s*秒?)?
            \s*(?P<zone>Z|UTC|GMT|[+-]\d{2}:?\d{2})?\s*$",
        )
        .unwrap()
    })
}

/// C `asctime` / `ctime`: `Tue Oct 14 10:00:03 2026` (optionally with a fraction, AM/PM or an offset before the year).
fn asctime_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)^(?:[A-Za-z]{3,9},?\s+)?(?P<m>[A-Za-z]{3,9})\s+(?P<d>\d{1,2})\s+
            (?P<time>\d{1,2}:\d{2}:\d{2}(?:[.,]\d{1,9})?)\s*(?P<ampm>[AaPp][Mm])?\s+
            (?:(?P<zone>Z|UTC|GMT|[+-]\d{2}:?\d{2})\s+)?(?P<y>\d{4})\s*$",
        )
        .unwrap()
    })
}

fn time_only_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(\d{1,2}:\d{2}:\d{2}(?:[.,]\d{1,9})?)\s*([AaPp][Mm])?$").unwrap())
}

impl TimestampParser {
    /// Builds a parser from user formats (.NET style or strftime), the zone assumed for values without an offset (`"UTC"`, `"local"` or an offset like `"+08:00"`) and the date order (`None` = decide from the value). Fails on an unknown zone.
    pub fn new(formats: &[String], zone: &str, day_first: Option<bool>) -> Result<Self> {
        let zone = match zone.trim().to_ascii_lowercase().as_str() {
            "" | "utc" | "z" => AssumedZone::Utc,
            "local" => AssumedZone::Local,
            other => match parse_offset(other) {
                Some(o) => AssumedZone::Fixed(o),
                None => bail!("assume_timezone must be \"UTC\", \"local\" or an offset like \"+08:00\", got \"{zone}\""),
            },
        };
        let is_asctime = |f: &String| f.trim().eq_ignore_ascii_case("asctime");
        let clocks = formats.iter().filter_map(|f| NumberClock::named(f)).collect();
        let custom = formats
            .iter()
            .filter(|f| NumberClock::named(f).is_none() && !is_asctime(f))
            .map(|f| to_strftime(f))
            .collect();
        Ok(TimestampParser {
            custom,
            clocks,
            asctime: formats.iter().any(is_asctime),
            zone,
            day_first,
            boot_ms: 0,
            reference_ms: None,
        })
    }

    /// This parser with `uptime` values counted from `boot_ms`.
    pub fn with_boot(&self, boot_ms: i64) -> Self {
        TimestampParser { boot_ms, ..self.clone() }
    }

    /// This parser reading formats without a year near `reference_ms` (a file's modification time) instead of now.
    pub fn with_reference(&self, reference_ms: Option<i64>) -> Self {
        TimestampParser {
            reference_ms,
            ..self.clone()
        }
    }

    /// C `asctime` text (`Tue Oct 14 10:00:03 2026`, also with a fraction, AM/PM or an offset before the year).
    fn parse_asctime(&self, s: &str) -> Option<i64> {
        let c = asctime_re().captures(s)?;
        let date = NaiveDate::from_ymd_opt(c["y"].parse().ok()?, month_number(&c["m"])?, c["d"].parse().ok()?)?;
        self.finish(
            date,
            Some(&c["time"]),
            c.name("ampm").map(|m| m.as_str()),
            c.name("zone").map(|m| m.as_str()),
        )
    }

    /// Whether the formats name the `uptime` clock.
    pub fn uses_uptime(&self) -> bool {
        self.clocks.contains(&NumberClock::Uptime)
    }

    /// The configured date order: `Some(true)` day first, `Some(false)` month first, `None` decided per value.
    pub fn day_first(&self) -> Option<bool> {
        self.day_first
    }

    /// This parser with another date order.
    pub fn with_day_first(&self, day_first: Option<bool>) -> Self {
        TimestampParser { day_first, ..self.clone() }
    }

    /// This parser with `format` (.NET style or strftime) tried before its own formats.
    pub fn with_format(&self, format: &str) -> Self {
        if format.trim().eq_ignore_ascii_case("asctime") {
            return TimestampParser {
                asctime: true,
                ..self.clone()
            };
        }
        if let Some(clock) = NumberClock::named(format) {
            let mut clocks = vec![clock];
            clocks.extend(self.clocks.iter().copied());
            return TimestampParser { clocks, ..self.clone() };
        }
        let mut custom = vec![to_strftime(format)];
        custom.extend(self.custom.iter().cloned());
        TimestampParser { custom, ..self.clone() }
    }

    /// Parses a full timestamp into epoch milliseconds.
    pub fn parse(&self, s: &str) -> Option<i64> {
        let s = s.trim().trim_matches(|c| c == '[' || c == ']' || c == '"');
        if s.is_empty() {
            return None;
        }
        for clock in &self.clocks {
            if let Some(ms) = clock.read(s, self.boot_ms) {
                return Some(ms);
            }
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
        if self.asctime {
            if let Some(ms) = self.parse_asctime(s) {
                return Some(ms);
            }
        }
        if let Some(c) = japanese_re().captures(s) {
            let date = NaiveDate::from_ymd_opt(c["y"].parse().ok()?, c["m"].parse().ok()?, c["d"].parse().ok()?)?;
            let mut time = format!("{}:{}:{}", &c["h"], &c["mi"], c.name("s").map_or("0", |m| m.as_str()));
            if let Some(f) = c.name("f") {
                time.push('.');
                time.push_str(f.as_str());
            }
            let ampm = c.name("ampm").map(|m| if m.as_str() == "午後" { "PM" } else { "AM" });
            return self.finish(date, Some(&time), ampm, c.name("zone").map(|m| m.as_str()));
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
        if !fmt.contains("%Y") && !fmt.contains("%y") {
            return self.parse_without_year(s, fmt, self.reference_ms.unwrap_or_else(|| Utc::now().timestamp_millis()));
        }
        None
    }

    /// A format without a year (BSD syslog "Oct 14 10:41:00"): the current year, or the
    /// previous one when that would put the event more than a day in the future.
    fn parse_without_year(&self, s: &str, fmt: &str, now_ms: i64) -> Option<i64> {
        let year = DateTime::from_timestamp_millis(now_ms)?.year();
        let at = |y: i32| {
            let text = format!("{y} {s}");
            let with_year = format!("%Y {fmt}");
            DateTime::parse_from_str(&text, &with_year)
                .map(|d| d.timestamp_millis())
                .ok()
                .or_else(|| NaiveDateTime::parse_from_str(&text, &with_year).ok().and_then(|n| self.localize(n)))
        };
        let ms = at(year)?;
        if ms > now_ms + 86_400_000 {
            return at(year - 1);
        }
        Some(ms)
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
                if out.ends_with(',') {
                    // ",fff": chrono's "%.f" only reads a dot, so the comma stays literal before fixed digits.
                    out.push_str(match n {
                        1..=3 => "%3f",
                        4..=6 => "%6f",
                        _ => "%9f",
                    });
                } else {
                    // ".fff" -> "%.f" (the dot is consumed by %.f)
                    if out.ends_with('.') {
                        out.pop();
                    }
                    out.push_str("%.f");
                }
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

/// The UTC calendar date of a time in milliseconds since the Unix epoch; `None` when out of range.
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
    fn formats_without_a_year_use_the_current_or_previous_year() {
        let p = TimestampParser::new(&["%b %e %H:%M:%S".to_string()], "UTC", None).unwrap();
        let utc = |y, mo, d, h| Utc.with_ymd_and_hms(y, mo, d, h, 0, 0).unwrap().timestamp_millis();
        let fmt = "%b %e %H:%M:%S";
        assert_eq!(
            p.parse_without_year("Oct 14 10:00:00", fmt, utc(2026, 10, 17, 0)),
            Some(utc(2026, 10, 14, 10))
        );
        // Just after new year, December is last year.
        assert_eq!(
            p.parse_without_year("Dec 31 23:00:00", fmt, utc(2027, 1, 2, 0)),
            Some(utc(2026, 12, 31, 23))
        );
        assert_eq!(p.parse_without_year("not a date", fmt, utc(2027, 1, 2, 0)), None);
        assert_eq!(p.parse_without_year("Oct 14 10:00:00", fmt, i64::MAX), None);
        // Through the public parser (the current year).
        assert!(p.parse("Oct  4 09:00:00").is_some());
        let zoned = TimestampParser::new(&["%b %e %H:%M:%S %z".to_string()], "UTC", None).unwrap();
        assert_eq!(
            zoned.parse_without_year("Oct 14 18:00:00 +0800", "%b %e %H:%M:%S %z", utc(2026, 10, 17, 0)),
            Some(utc(2026, 10, 14, 10))
        );
    }

    #[test]
    fn japanese_asctime_and_number_clocks() {
        let t = |s: &str| p().parse(s);
        assert_eq!(t("2026年10月14日 10時00分03秒"), Some(ms(2026, 10, 14, 10, 0, 3, 0)));
        assert_eq!(t("2026年10月14日(火) 午後 3:04:05.120"), Some(ms(2026, 10, 14, 15, 4, 5, 120)));
        assert_eq!(t("2026年1月4日 午前12時05分"), Some(ms(2026, 1, 4, 0, 5, 0, 0)));
        assert_eq!(t("2026年10月14日 10時00分03秒 +09:00"), Some(ms(2026, 10, 14, 1, 0, 3, 0)));
        assert_eq!(t("2026年13月14日 10時00分03秒"), None);
        assert_eq!(t("Tue Oct 14 10:00:03 2026"), None, "asctime is named in the formats");
        let a = TimestampParser::new(&["asctime".to_string()], "UTC", None).unwrap();
        assert_eq!(a.parse("Tue Oct 14 10:00:03 2026"), Some(ms(2026, 10, 14, 10, 0, 3, 0)));
        assert_eq!(a.parse("Tue Oct  4 10:00:03.250 PM 2026"), Some(ms(2026, 10, 4, 22, 0, 3, 250)));
        assert_eq!(a.parse("Oct 14 10:00:03 +02:00 2026"), Some(ms(2026, 10, 14, 8, 0, 3, 0)));
        assert_eq!(
            p().with_format("asctime").parse("Tue Oct 14 10:00:03 2026"),
            Some(ms(2026, 10, 14, 10, 0, 3, 0))
        );
        let f = |fmt: &str, s: &str| TimestampParser::new(&[fmt.to_string()], "UTC", None).unwrap().parse(s);
        let at = ms(2026, 10, 14, 10, 0, 3, 0);
        assert_eq!(f("epoch_s", "1791972003"), Some(at));
        assert_eq!(f("epoch_ms", "1791972003000"), Some(at));
        assert_eq!(f("epoch_us", "1791972003000123"), Some(at));
        assert_eq!(f("epoch_ns", "1791972003000123456"), Some(at));
        assert_eq!(
            f("dotnet_ticks", &(at as i128 * 10_000 + 621_355_968_000_000_000).to_string()),
            Some(at)
        );
        assert_eq!(
            f("FILETIME", &(at as i128 * 10_000 + 116_444_736_000_000_000).to_string()),
            Some(at)
        );
        assert_eq!(f("filetime", "12"), None, "before 1970");
        assert_eq!(f("epoch_ms", "12a"), None);
        let up = TimestampParser::new(&["uptime".to_string()], "UTC", None).unwrap();
        assert!(up.uses_uptime() && !p().uses_uptime());
        assert_eq!(up.with_boot(at).parse("[   12.345678]"), Some(at + 12_345));
        assert_eq!(up.with_format("epoch_ms").parse("1791972003000"), Some(at));
        assert_eq!(NumberClock::named("nope"), None);
        // Formats without a year are read near the reference time (the file's), with the year rolling back.
        let yearless = TimestampParser::new(&["%b %e %H:%M:%S".to_string()], "UTC", None).unwrap();
        let jan = ms(2027, 1, 2, 0, 0, 0, 0);
        assert_eq!(
            yearless.with_reference(Some(jan)).parse("Dec 31 23:00:00"),
            Some(ms(2026, 12, 31, 23, 0, 0, 0))
        );
        assert_eq!(
            yearless.with_reference(Some(jan)).parse("Jan  1 01:00:00"),
            Some(ms(2027, 1, 1, 1, 0, 0, 0))
        );
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
        assert_eq!(to_strftime("ddd MMM HH:mm:ss,FFF"), "%a %b %H:%M:%S,%3f");
        assert_eq!(to_strftime("HH:mm:ss,ffffff"), "%H:%M:%S,%6f");
        assert_eq!(to_strftime("HH:mm:ss,fffffffff"), "%H:%M:%S,%9f");
        // A comma before the fraction stays a comma (chrono's "%.f" reads only a dot).
        assert_eq!(
            f("dd.MM.yyyy HH:mm:ss,fff", "14.10.2026 10:00:00,250"),
            Some(ms(2026, 10, 14, 10, 0, 0, 250))
        );
        assert_eq!(to_strftime("'unterminated"), "unterminated");
    }

    #[test]
    fn comma_between_date_and_time() {
        // Slashes, a comma between date and time, milliseconds and an offset (reported from a real log).
        let want = ms(2025, 11, 7, 23, 4, 37, 759);
        assert_eq!(p().parse("2025/11/08,00:04:37.759+01:00"), Some(want));
        assert_eq!(p().parse("2025-11-08,00:04:37.759+0100"), Some(want));
        assert_eq!(p().parse("2025/11/08,00:04:37"), Some(ms(2025, 11, 8, 0, 4, 37, 0)));
        assert_eq!(p().parse("2025/11/08,00:04:37.759Z"), Some(ms(2025, 11, 8, 0, 4, 37, 759)));
        assert_eq!(p().parse("2025/11/08, 00:04:37"), Some(ms(2025, 11, 8, 0, 4, 37, 0)));
        assert_eq!(p().parse("[2025/11/08,00:04:37.759+01:00]"), Some(want));
        // Midnight with an offset: the instant falls on the previous UTC day, milliseconds with a leading zero.
        assert_eq!(p().parse("2024/03/02,00:00:00.013+01:00"), Some(ms(2024, 3, 1, 23, 0, 0, 13)));
        assert_eq!(p().parse("08/11/2025,00:04:37"), Some(ms(2025, 8, 11, 0, 4, 37, 0)));
        // The comma as the decimal separator (log4j) still reads as before.
        assert_eq!(p().parse("2025-11-08 00:04:37,759"), Some(ms(2025, 11, 8, 0, 4, 37, 759)));
        assert_eq!(p().parse("2025/11/08,00:04:37,759"), Some(ms(2025, 11, 8, 0, 4, 37, 759)));
        // Two commas or a comma with nothing after it are not a timestamp.
        assert_eq!(p().parse("2025/11/08,,00:04:37"), None);
        assert_eq!(p().parse("2025/11/08,"), None);
        assert_eq!(detect_day_first(["25/11/2025,00:04:37"]), Some(true));
    }

    #[test]
    fn with_format_is_tried_first() {
        let base = TimestampParser::new(&[], "+01:00", None).unwrap();
        let extended = base.with_format("%a %b %d %H:%M:%S %Y");
        assert_eq!(base.parse("Sat Nov 08 00:04:37 2025"), None);
        assert_eq!(extended.parse("Sat Nov 08 00:04:37 2025"), Some(ms(2025, 11, 7, 23, 4, 37, 0)));
        // The rest of the parser is kept.
        assert_eq!(extended.parse("2025-11-08 00:04:37"), Some(ms(2025, 11, 7, 23, 4, 37, 0)));
        assert_eq!(
            TimestampParser::default().with_format("yyyy|MM|dd").parse("2025|11|08"),
            Some(ms(2025, 11, 8, 0, 0, 0, 0))
        );
    }
}
