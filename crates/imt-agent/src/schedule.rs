//! Timers: cron expressions and fixed intervals, and when they are next due.
//!
//! Cron uses the `croner` crate (MIT, maintained): POSIX/Vixie five-field
//! patterns with an optional seconds field, `@daily`-style aliases, `L`, `W`
//! and `#`, evaluated in local time or UTC. Everything here is pure.

use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::time::Duration;

/// Shortest interval (a run every few seconds would only fill the disk).
pub const MIN_EVERY: Duration = Duration::from_secs(60);
/// Longest interval.
pub const MAX_EVERY: Duration = Duration::from_secs(31 * 86_400);
/// Longest accepted cron expression.
const MAX_CRON_LEN: usize = 120;

/// Which clock cron expressions are read in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimeZoneChoice {
    #[default]
    Local,
    Utc,
}

/// A parsed timer.
#[derive(Debug, Clone, PartialEq)]
pub enum Timer {
    Cron(Box<croner::Cron>),
    Every(Duration),
}

/// Parses `500ms`, `30s`, `15m`, `2h`, `1d` (a bare number is seconds).
pub fn parse_duration(text: &str) -> Result<Duration, String> {
    let s = text.trim();
    let (num, unit_ms) = if let Some(n) = s.strip_suffix("ms") {
        (n, 1)
    } else if let Some(n) = s.strip_suffix('s') {
        (n, 1000)
    } else if let Some(n) = s.strip_suffix('m') {
        (n, 60_000)
    } else if let Some(n) = s.strip_suffix('h') {
        (n, 3_600_000)
    } else if let Some(n) = s.strip_suffix('d') {
        (n, 86_400_000)
    } else {
        (s, 1000)
    };
    let n: u64 = num
        .trim()
        .parse()
        .map_err(|_| format!("\"{text}\" is not a duration; use e.g. 30s, 15m, 2h or 1d"))?;
    Ok(Duration::from_millis(n.saturating_mul(unit_ms)))
}

impl Timer {
    /// A cron expression (`0 2 * * *`, `@hourly`, `0 */15 * * * *`).
    pub fn cron(expr: &str) -> Result<Timer, String> {
        let expr = expr.trim();
        if expr.is_empty() {
            return Err("the cron expression is empty".into());
        }
        if expr.len() > MAX_CRON_LEN {
            return Err(format!("the cron expression is longer than {MAX_CRON_LEN} characters"));
        }
        croner::Cron::from_str(expr)
            .map(|c| Timer::Cron(Box::new(c)))
            .map_err(|e| format!("invalid cron expression \"{expr}\": {e}"))
    }

    /// A fixed interval between runs (`15m`, `6h`).
    pub fn every(text: &str) -> Result<Timer, String> {
        let d = parse_duration(text)?;
        if d < MIN_EVERY || d > MAX_EVERY {
            return Err(format!("the interval must be between 1m and 31d (got \"{}\")", text.trim()));
        }
        Ok(Timer::Every(d))
    }

    /// The first time strictly after `after` when the timer fires, or `None`
    /// when a cron pattern never matches again.
    pub fn next_after(&self, after: DateTime<Utc>, tz: TimeZoneChoice) -> Option<DateTime<Utc>> {
        match self {
            Timer::Every(d) => Some(after + chrono::Duration::from_std(*d).ok()?),
            Timer::Cron(c) => match tz {
                TimeZoneChoice::Utc => c.find_next_occurrence(&after, false).ok(),
                TimeZoneChoice::Local => c
                    .find_next_occurrence(&after.with_timezone(&Local), false)
                    .ok()
                    .map(|t| t.with_timezone(&Utc)),
            },
        }
    }

    /// Whether a timer anchored at `anchor` (the last run, or when the agent
    /// started) is due at `now`. Runs missed while the agent was down are
    /// caught up once, not once per missed slot.
    pub fn is_due(&self, anchor: DateTime<Utc>, now: DateTime<Utc>, tz: TimeZoneChoice) -> bool {
        self.next_after(anchor, tz).is_some_and(|next| next <= now)
    }

    /// Plain-English description for logs and the UI.
    pub fn describe(&self) -> String {
        match self {
            Timer::Every(d) => format!("every {}", human(*d)),
            Timer::Cron(c) => c.describe(),
        }
    }
}

/// `90s` → "1m 30s".
pub fn human(d: Duration) -> String {
    let s = d.as_secs();
    let parts = [(s / 86_400, "d"), (s % 86_400 / 3600, "h"), (s % 3600 / 60, "m"), (s % 60, "s")];
    let text: Vec<String> = parts.iter().filter(|(n, _)| *n > 0).map(|(n, u)| format!("{n}{u}")).collect();
    if text.is_empty() {
        format!("{}ms", d.as_millis())
    } else {
        text.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, s).unwrap()
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("500ms").unwrap(), Duration::from_millis(500));
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("15m").unwrap(), Duration::from_secs(900));
        assert_eq!(parse_duration(" 2h ").unwrap(), Duration::from_secs(7200));
        assert_eq!(parse_duration("1d").unwrap(), Duration::from_secs(86_400));
        assert_eq!(parse_duration("45").unwrap(), Duration::from_secs(45));
        assert!(parse_duration("soon").unwrap_err().contains("not a duration"));
        assert!(parse_duration("-5m").is_err());
    }

    #[test]
    fn intervals_and_bounds() {
        let t = Timer::every("15m").unwrap();
        assert_eq!(
            t.next_after(at(2026, 1, 1, 10, 0, 0), TimeZoneChoice::Utc),
            Some(at(2026, 1, 1, 10, 15, 0))
        );
        assert_eq!(t.describe(), "every 15m");
        assert!(Timer::every("30s").unwrap_err().contains("between 1m and 31d"));
        assert!(Timer::every("32d").is_err());
        assert!(Timer::every("x").is_err());
    }

    #[test]
    fn cron_next_run_with_a_fixed_clock() {
        let t = Timer::cron("0 2 * * *").unwrap();
        let utc = TimeZoneChoice::Utc;
        assert_eq!(t.next_after(at(2026, 3, 1, 1, 59, 59), utc), Some(at(2026, 3, 1, 2, 0, 0)));
        // Strictly after: the slot itself is not repeated.
        assert_eq!(t.next_after(at(2026, 3, 1, 2, 0, 0), utc), Some(at(2026, 3, 2, 2, 0, 0)));
        let q = Timer::cron("*/15 * * * *").unwrap();
        assert_eq!(q.next_after(at(2026, 3, 1, 10, 7, 0), utc), Some(at(2026, 3, 1, 10, 15, 0)));
        let monthly = Timer::cron("@monthly").unwrap();
        assert_eq!(monthly.next_after(at(2026, 2, 10, 0, 0, 0), utc), Some(at(2026, 3, 1, 0, 0, 0)));
        let secs = Timer::cron("30 * * * * *").unwrap();
        assert_eq!(secs.next_after(at(2026, 3, 1, 10, 0, 0), utc), Some(at(2026, 3, 1, 10, 0, 30)));
        assert!(!t.describe().is_empty());
        // Local time is converted back to UTC.
        let local = t.next_after(at(2026, 3, 1, 0, 0, 0), TimeZoneChoice::Local).unwrap();
        assert_eq!(local.with_timezone(&Local).format("%H:%M").to_string(), "02:00");
    }

    #[test]
    fn invalid_cron_is_explained() {
        assert!(Timer::cron("").unwrap_err().contains("empty"));
        assert!(Timer::cron("61 * * * *").unwrap_err().contains("invalid cron"));
        assert!(Timer::cron("nonsense").is_err());
        assert!(Timer::cron(&"1 ".repeat(100)).unwrap_err().contains("longer than"));
    }

    #[test]
    fn due_and_catch_up_once() {
        let t = Timer::cron("0 * * * *").unwrap();
        let utc = TimeZoneChoice::Utc;
        let anchor = at(2026, 1, 1, 10, 30, 0);
        assert!(!t.is_due(anchor, at(2026, 1, 1, 10, 59, 0), utc));
        assert!(t.is_due(anchor, at(2026, 1, 1, 11, 0, 0), utc));
        // Down for a day: due now, and after running (anchor = now) not again until the next slot.
        let now = at(2026, 1, 2, 12, 10, 0);
        assert!(t.is_due(anchor, now, utc));
        assert!(!t.is_due(now, at(2026, 1, 2, 12, 59, 0), utc));
        // A pattern that can never match again is never due.
        let never = Timer::cron("0 0 30 2 *").unwrap();
        assert!(!never.is_due(anchor, at(2030, 1, 1, 0, 0, 0), utc));
    }

    #[test]
    fn humanises_durations() {
        assert_eq!(human(Duration::from_secs(90)), "1m 30s");
        assert_eq!(human(Duration::from_secs(90_061)), "1d 1h 1m 1s");
        assert_eq!(human(Duration::from_millis(250)), "250ms");
    }
}
