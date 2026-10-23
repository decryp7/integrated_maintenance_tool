//! UTC date formatting for epoch milliseconds, without a date library.

/// "2026-10-14 10:42:01" (UTC).
pub fn format_utc(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // civil_from_days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// "10:42" (UTC).
pub fn format_hm(ms: i64) -> String {
    let rem = ms.div_euclid(1000).rem_euclid(86_400);
    format!("{:02}:{:02}", rem / 3600, rem % 3600 / 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_known_instants() {
        assert_eq!(format_utc(0), "1970-01-01 00:00:00");
        assert_eq!(format_utc(1_792_000_921_000), "2026-10-14 18:02:01");
        assert_eq!(format_utc(-1000), "1969-12-31 23:59:59");
        assert_eq!(format_utc(951_782_400_000), "2000-02-29 00:00:00");
        assert_eq!(format_hm(1_792_000_921_000), "18:02");
        assert_eq!(format_hm(-60_000), "23:59");
    }
}
