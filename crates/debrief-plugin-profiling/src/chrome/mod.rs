//! Chrome / V8 CPU profiles: `.cpuprofile` files and trace event JSON.
//!
//! - [`CpuProfileImporter`]: the DevTools protocol `Profiler.Profile` object
//!   (<https://chromedevtools.github.io/devtools-protocol/tot/Profiler/#type-Profile>),
//!   as saved by DevTools and `node --cpu-prof`.
//! - [`TraceEventImporter`]: the Trace Event Format
//!   (<https://docs.google.com/document/d/1CvAClvFfyA5R-PhYUmn5OOQtYMH4h6I0nSsKchNAySU>,
//!   see also <https://perfetto.dev/docs/getting-started/other-formats>):
//!   V8 CPU profile chunks (`Profile` / `ProfileChunk` events) or, without
//!   them, duration events (`X`, `B`/`E`).
//!
//! Both share the V8 profile node tree ([`nodes`]).

mod cpuprofile;
pub(crate) mod nodes;
mod trace;

pub use cpuprofile::CpuProfileImporter;
pub use trace::TraceEventImporter;

use debrief_profile::{Limits, ProfileError, Result};

/// Refuses input above the size limit (importers can be called directly, not only through `import_with`).
pub(crate) fn check_size(bytes: &[u8], limits: &Limits) -> Result<()> {
    if bytes.len() as u64 > limits.max_input_bytes {
        return Err(ProfileError::LimitExceeded {
            what: "file size",
            limit: limits.max_input_bytes,
            hint: "open a smaller capture",
        });
    }
    Ok(())
}

/// Microseconds (possibly fractional or negative) to signed nanoseconds, saturating; non-finite is 0.
pub(crate) fn us_to_ns(us: f64) -> i64 {
    let x = (us * 1000.0).round();
    if x.is_finite() {
        // `as` saturates.
        x as i64
    } else {
        0
    }
}

/// Time attributed to each sample, like DevTools: the gap to the next
/// sample in time order; the last sample gets the gap to `end` when it is
/// later, else the average gap. Times may be unsorted (deltas can be negative).
pub(crate) fn sample_weights(times: &[i64], end: Option<i64>) -> Vec<u64> {
    let mut order: Vec<usize> = (0..times.len()).collect();
    order.sort_by_key(|&i| times[i]);
    let mut out = vec![0u64; times.len()];
    for w in order.windows(2) {
        out[w[0]] = gap(times[w[0]], times[w[1]]);
    }
    if let (Some(&first), Some(&last)) = (order.first(), order.last()) {
        out[last] = match end {
            Some(e) if e > times[last] => gap(times[last], e),
            _ if order.len() > 1 => gap(times[first], times[last]) / (order.len() as u64 - 1),
            _ => 0,
        };
    }
    out
}

fn gap(a: i64, b: i64) -> u64 {
    u64::try_from(i128::from(b) - i128::from(a)).unwrap_or(0)
}

/// `t - origin` as an unsigned offset (0 before the origin).
pub(crate) fn offset(t: i64, origin: i64) -> u64 {
    gap(origin, t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_follow_time_order() {
        assert_eq!(sample_weights(&[0, 10, 30], Some(35)), vec![10, 20, 5]);
        assert_eq!(sample_weights(&[0, 10, 30], None), vec![10, 20, 15]);
        assert_eq!(sample_weights(&[10, 0, 30], Some(5)), vec![20, 10, 15], "unsorted, end before last");
        assert_eq!(sample_weights(&[7], None), vec![0]);
        assert_eq!(sample_weights(&[7], Some(9)), vec![2]);
        assert!(sample_weights(&[], None).is_empty());
        assert_eq!(sample_weights(&[i64::MIN, i64::MAX], None)[0], u64::MAX);
    }

    #[test]
    fn conversions_saturate() {
        assert_eq!(us_to_ns(1.5), 1500);
        assert_eq!(us_to_ns(-2.0), -2000);
        assert_eq!(us_to_ns(f64::INFINITY), 0);
        assert_eq!(us_to_ns(1e300), i64::MAX);
        assert_eq!(offset(5, 10), 0);
        assert_eq!(offset(15, 10), 5);
        assert!(check_size(b"abc", &Limits::default()).is_ok());
        let small = Limits {
            max_input_bytes: 2,
            ..Limits::default()
        };
        assert!(check_size(b"abc", &small).is_err());
    }
}
