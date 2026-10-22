#![no_main]
#[path = "../common/mod.rs"]
mod common;
use libfuzzer_sys::fuzz_target;

use debrief_core::time::{self, TimestampParser};

fuzz_target!(|data: &[u8]| {
    let s = String::from_utf8_lossy(data);
    let formats = vec!["yyyy-MM-dd HH:mm:ss,fff".to_string(), "dd/MMM/yyyy:HH:mm:ss zzz".to_string()];
    for (fmts, zone, day_first) in [
        (&[][..], "UTC", None),
        (&formats[..], "+08:00", Some(true)),
        (&formats[..], "local", Some(false)),
    ] {
        let p = TimestampParser::new(fmts, zone, day_first).unwrap();
        let _ = p.parse(&s);
        let _ = p.parse_time_on(&s, chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap());
        // The input's lines as lines whose timestamps were not read.
        let lines: Vec<&str> = s.lines().take(32).collect();
        let _ = debrief_core::timestamp_issue::suggest::propose(&lines, &p);
    }
    // Also treat the input as a user-supplied format.
    let _ = TimestampParser::new(&[s.to_string()], "UTC", None).map(|p| p.parse("2026-10-14 10:00:00,123"));
    let _ = time::to_strftime(&s);
    let _ = time::detect_day_first(s.lines());
});
