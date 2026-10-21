//! Human-readable quantities for text composed in Rust (AI summaries, exports, reports, log lines).
//!
//! These follow the same rules as the UI's `app/src/app/shared/format/units.ts`, in the source locale (en-US):
//! binary steps with the labels KB / MB / GB / TB / PB, `12 bytes`, one decimal below 10 and none above (trailing
//! zeros trimmed), `4.2 s` / `3 min 10 s` / `2 h 5 min` durations, grouped counts. Anything shown in the UI should
//! travel as a typed value and be formatted there; use this only where the text itself is the product.
//! `units-cases.json` is the table both implementations are tested against.

/// Shown for a value that is not a usable number.
pub const NONE: &str = "—";

/// Rounds half away from zero, as `Intl.NumberFormat` does.
fn round_to(v: f64, decimals: i32) -> f64 {
    let f = 10f64.powi(decimals);
    (v * f).round() / f
}

fn group(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `12345` → `"12,345"`.
pub fn count(n: u64) -> String {
    group(n)
}

/// A rounded value with at most `decimals` decimals, grouped, without trailing zeros.
fn decimal(v: f64, decimals: i32) -> String {
    let r = round_to(v, decimals);
    if r.fract() == 0.0 {
        return group(r as u64);
    }
    let text = format!("{:.*}", decimals as usize, r);
    let (whole, frac) = text.split_once('.').unwrap_or((&text, ""));
    let whole: u64 = whole.parse().unwrap_or(0);
    format!("{}.{}", group(whole), frac.trim_end_matches('0'))
}

/// `0` → `"0 bytes"`, `1` → `"1 byte"`, `1536` → `"1.5 KB"`, `1_288_490_189` → `"1.2 GB"`. 1023.9 KB rolls over to `"1 MB"`.
pub fn bytes(n: u64) -> String {
    const LABELS: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];
    if n < 1024 {
        return if n == 1 {
            "1 byte".to_string()
        } else {
            format!("{} bytes", group(n))
        };
    }
    let rounded = |v: f64| if v < 10.0 { round_to(v, 1) } else { v.round() };
    let mut value = n as f64 / 1024.0;
    let mut i = 0;
    while i < LABELS.len() - 1 && rounded(value) >= 1024.0 {
        value /= 1024.0;
        i += 1;
    }
    format!("{} {}", decimal(value, if value < 10.0 { 1 } else { 0 }), LABELS[i])
}

/// The exact size, for a tooltip or a log: `1234567` → `"1,234,567 bytes"`.
pub fn exact_bytes(n: u64) -> String {
    if n == 1 {
        "1 byte".to_string()
    } else {
        format!("{} bytes", group(n))
    }
}

/// `350` → `"350 ms"`, `4200` → `"4.2 s"`, `190_000` → `"3 min 10 s"`, `7_500_000` → `"2 h 5 min"`, `97_200_000` → `"1 d 3 h"`.
pub fn duration_ms(ms: u64) -> String {
    if ms < 1000 {
        return format!("{ms} ms");
    }
    if ms < 9950 {
        return format!("{} s", decimal(ms as f64 / 1000.0, 1));
    }
    let total = (ms as f64 / 1000.0).round() as u64;
    let (s, min, h, d) = (total % 60, total / 60 % 60, total / 3600 % 24, total / 86_400);
    let part = |v: u64, unit: &str| format!("{v} {unit}");
    let mut parts = Vec::new();
    if d > 0 {
        parts.push(part(d, "d"));
        if h > 0 {
            parts.push(part(h, "h"));
        }
    } else if h > 0 {
        parts.push(part(h, "h"));
        if min > 0 {
            parts.push(part(min, "min"));
        }
    } else if min > 0 {
        parts.push(part(min, "min"));
        if s > 0 {
            parts.push(part(s, "s"));
        }
    } else {
        parts.push(part(s, "s"));
    }
    parts.join(" ")
}

/// A percentage given as 0..100: `42.46` with 1 digit → `"42.5%"`; `—` for NaN or infinity.
pub fn percent(pct: f64, digits: u8) -> String {
    if !pct.is_finite() {
        return NONE.to_string();
    }
    let sign = if pct < 0.0 { "-" } else { "" };
    format!("{sign}{}%", decimal(pct.abs(), i32::from(digits)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    /// The table shared with the UI's `units.spec.ts`: both sides must produce exactly these texts.
    #[test]
    fn agrees_with_the_shared_case_table() {
        let table: Value = serde_json::from_str(include_str!("../units-cases.json")).expect("units-cases.json is JSON");
        let run = |key: &str, f: &dyn Fn(&Value) -> String| {
            let cases = table[key].as_array().unwrap_or_else(|| panic!("{key} cases"));
            assert!(!cases.is_empty());
            for c in cases {
                assert_eq!(f(&c["in"]), c["out"].as_str().unwrap(), "{key}({})", c["in"]);
            }
        };
        run("bytes", &|v| bytes(v.as_u64().unwrap()));
        run("durationMs", &|v| duration_ms(v.as_u64().unwrap()));
        run("count", &|v| count(v.as_u64().unwrap()));
        run("percent", &|v| percent(v[0].as_f64().unwrap(), v[1].as_u64().unwrap() as u8));
    }

    #[test]
    fn edge_cases() {
        assert_eq!(exact_bytes(1), "1 byte");
        assert_eq!(exact_bytes(1_234_567), "1,234,567 bytes");
        assert_eq!(percent(f64::NAN, 0), NONE);
        assert_eq!(percent(f64::INFINITY, 1), NONE);
        assert_eq!(percent(-12.5, 1), "-12.5%");
        assert!(bytes(u64::MAX).ends_with(" PB"));
    }
}
