//! Times on the command line (`--since`, `--until`, `--from`, `--to`): RFC 3339, `YYYY-MM-DD HH:MM[:SS]` or a
//! date, all read as UTC unless they carry an offset. Parse, don't validate: the result is epoch milliseconds.

use chrono::{DateTime, NaiveDate, NaiveDateTime};

/// Epoch milliseconds of `text`, or a message saying what to write.
pub fn parse_time(text: &str) -> Result<i64, String> {
    let t = text.trim();
    if let Ok(d) = DateTime::parse_from_rfc3339(t) {
        return Ok(d.timestamp_millis());
    }
    for fmt in ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M"] {
        if let Ok(d) = NaiveDateTime::parse_from_str(t, fmt) {
            return Ok(d.and_utc().timestamp_millis());
        }
    }
    if let Ok(d) = NaiveDate::parse_from_str(t, "%Y-%m-%d") {
        if let Some(dt) = d.and_hms_opt(0, 0, 0) {
            return Ok(dt.and_utc().timestamp_millis());
        }
    }
    Err(format!(
        "\"{text}\" is not a time: write 2026-10-01, 2026-10-01 14:30 or 2026-10-01T14:30:00Z (UTC unless it has an offset)"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_documented_forms_as_utc() {
        assert_eq!(parse_time("1970-01-02").unwrap(), 86_400_000);
        assert_eq!(parse_time("1970-01-01 00:01").unwrap(), 60_000);
        assert_eq!(parse_time("1970-01-01T00:00:01Z").unwrap(), 1_000);
        assert_eq!(parse_time("1970-01-01T01:00:00+01:00").unwrap(), 0);
        assert_eq!(parse_time(" 1970-01-01 00:00:00.5 ").unwrap(), 500);
    }

    #[test]
    fn refuses_other_text_with_a_hint() {
        let e = parse_time("yesterday").unwrap_err();
        assert!(e.contains("yesterday") && e.contains("2026-10-01"), "{e}");
        assert!(parse_time("").is_err());
    }
}
