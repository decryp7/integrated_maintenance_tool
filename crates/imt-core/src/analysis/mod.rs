//! Analysis over already-loaded events: anomaly detection, window comparison,
//! per-field statistics and trace/request correlation.
//!
//! Everything here is a pure function over iterators of [`crate::model::Record`]
//! (plus an instance key where the caller has one); nothing depends on the
//! workspace, so the same code serves the UI, exports and tests.

pub mod anomaly;
pub mod compare;
pub mod field_stats;
pub mod trace;

pub use anomaly::{detect, Anomaly, AnomalyConfig, AnomalyKind};
pub use compare::{compare_windows, CompareConfig, Comparison};
pub use field_stats::{field_stats, FieldStats, HistogramBin, NumericStats, ValueCount};
pub use trace::{correlate, request_durations, spans, waterfall, RequestSpan, Span, WaterfallRow};

/// Small numeric and formatting helpers shared by the analysis modules.
pub(crate) mod util {
    pub fn median_sorted(v: &[f64]) -> f64 {
        let n = v.len();
        if n == 0 {
            0.0
        } else if n % 2 == 1 {
            v[n / 2]
        } else {
            (v[n / 2 - 1] + v[n / 2]) / 2.0
        }
    }

    pub fn sorted(v: &[f64]) -> Vec<f64> {
        let mut s = v.to_vec();
        s.sort_by(|a, b| a.total_cmp(b));
        s
    }

    pub fn median(v: &[f64]) -> f64 {
        median_sorted(&sorted(v))
    }

    /// Median absolute deviation around `med`.
    pub fn mad(v: &[f64], med: f64) -> f64 {
        let dev: Vec<f64> = v.iter().map(|x| (x - med).abs()).collect();
        median(&dev)
    }

    /// Linear-interpolated percentile (`p` in 0..=100) of a sorted slice.
    pub fn percentile_sorted(v: &[f64], p: f64) -> f64 {
        if v.is_empty() {
            return 0.0;
        }
        let rank = p / 100.0 * (v.len() - 1) as f64;
        let (lo, hi) = (rank.floor() as usize, rank.ceil() as usize);
        v[lo] + (v[hi] - v[lo]) * (rank - lo as f64)
    }

    /// UTC wall clock "HH:MM:SS" of an epoch-ms timestamp.
    pub fn fmt_clock(ms: i64) -> String {
        let s = ms.div_euclid(1000).rem_euclid(86_400);
        format!("{:02}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60)
    }

    /// "850 ms", "46 s", "2 min 5 s", "1 h 5 min".
    pub fn fmt_duration(ms: i64) -> String {
        let s = ms / 1000;
        if ms < 1000 {
            format!("{ms} ms")
        } else if s < 60 {
            format!("{s} s")
        } else if s < 3600 {
            format!("{} min {} s", s / 60, s % 60)
        } else {
            format!("{} h {} min", s / 3600, s % 3600 / 60)
        }
    }

    /// Short number: whole when large, otherwise up to two decimals.
    pub fn fmt_num(v: f64) -> String {
        if v.abs() >= 100.0 {
            return format!("{v:.0}");
        }
        let s = format!("{v:.2}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }

    pub fn truncate(s: &str, max: usize) -> String {
        let line = s.lines().next().unwrap_or("");
        if line.chars().count() <= max {
            line.to_string()
        } else {
            let cut: String = line.chars().take(max).collect();
            format!("{cut}…")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::util::*;

    #[test]
    fn helpers() {
        assert_eq!(median(&[]), 0.0);
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), 2.5);
        assert_eq!(mad(&[1.0, 2.0, 9.0], 2.0), 1.0);
        assert_eq!(percentile_sorted(&[], 50.0), 0.0);
        assert_eq!(percentile_sorted(&[1.0, 3.0], 50.0), 2.0);
        assert_eq!(fmt_clock(-1000), "23:59:59");
        assert_eq!(fmt_duration(850), "850 ms");
        assert_eq!(fmt_duration(125_000), "2 min 5 s");
        assert_eq!(fmt_duration(3_900_000), "1 h 5 min");
        assert_eq!(fmt_num(0.4), "0.4");
        assert_eq!(fmt_num(38.0), "38");
        assert_eq!(fmt_num(1234.6), "1235");
        assert_eq!(truncate("short\nsecond", 10), "short");
        assert_eq!(truncate("abcdef", 3), "abc…");
    }
}
