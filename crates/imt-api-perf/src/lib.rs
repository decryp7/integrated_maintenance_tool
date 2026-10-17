//! IMT plugin API: performance counter sources.
//!
//! A plugin that can read performance counters (on Windows, PDH) provides a
//! [`CounterSource`] under [`SERVICE_ID`] and [`VERSION`]; consumers (for
//! example the AI assistant) look it up from their `activate` hook:
//!
//! ```ignore
//! let counters = ctx.service::<dyn imt_api_perf::CounterSource>(imt_api_perf::SERVICE_ID, "^1")?;
//! let paths = counters.try_call(|c| c.counters())?;
//! ```
//!
//! Every provider must pass [`contract_tests`] (call it from a `#[test]`).
//! This crate is a public contract: changes follow semver, and additions to
//! the trait come with default methods.
//!
//! Counters that IMT imports from files (PerfMon `.csv`/`.blg`) are records,
//! queried through the workspace; this service is for live sampling.

use anyhow::Result;

/// Service id providers register under.
pub const SERVICE_ID: &str = "net.decryptology.perf.counters";
/// Version of this contract (the crate version).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// One sampled value.
#[derive(Debug, Clone, PartialEq)]
pub struct CounterSample {
    /// The counter path as requested, e.g. `\Processor(_Total)\% Processor Time`.
    pub path: String,
    pub value: f64,
    /// Sample time, Unix milliseconds.
    pub at_ms: i64,
}

/// Live performance counters.
pub trait CounterSource: Send + Sync {
    /// Counter paths this source can sample (`\Object(instance)\Counter` or
    /// `\Object\Counter`), without duplicates.
    fn counters(&self) -> Result<Vec<String>>;

    /// One sample of each requested counter, only for requested paths and
    /// with finite values. Counters that cannot be computed yet (rate
    /// counters need two samples) may be omitted. A path the source does not
    /// know is an error naming it. Must not panic on any input and must be
    /// callable from several threads.
    fn sample(&self, paths: &[String]) -> Result<Vec<CounterSample>>;
}

/// Whether `path` looks like `\Object\Counter` or `\Object(instance)\Counter`.
pub fn is_counter_path(path: &str) -> bool {
    let Some(rest) = path.strip_prefix('\\') else {
        return false;
    };
    let mut depth = 0i32;
    for (i, c) in rest.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            '\\' if depth == 0 => return i > 0 && !rest[i + 1..].trim().is_empty() && !rest[i + 1..].contains('\\'),
            _ => {}
        }
        if depth < 0 {
            return false;
        }
    }
    false
}

/// The behaviour every [`CounterSource`] must have. Returns every violation
/// found (an empty source passes trivially except for the input checks).
pub fn contract_tests<S: CounterSource + ?Sized>(source: &S) -> std::result::Result<(), Vec<String>> {
    let mut bad = Vec::new();
    let counters = match source.counters() {
        Ok(c) => c,
        Err(e) => return Err(vec![format!("counters() failed: {e:#}")]),
    };
    let mut seen = std::collections::HashSet::new();
    for c in &counters {
        if !is_counter_path(c) {
            bad.push(format!("counters(): \"{c}\" is not a counter path (\\Object(instance)\\Counter)"));
        }
        if !seen.insert(c) {
            bad.push(format!("counters(): \"{c}\" is listed twice"));
        }
    }
    match source.sample(&[]) {
        Ok(v) if v.is_empty() => {}
        Ok(v) => bad.push(format!("sample([]) returned {} values instead of none", v.len())),
        Err(e) => bad.push(format!("sample([]) failed: {e:#}")),
    }
    let wanted: Vec<String> = counters.iter().take(8).cloned().collect();
    for round in 0..2 {
        match source.sample(&wanted) {
            Ok(values) => {
                for v in &values {
                    if !wanted.contains(&v.path) {
                        bad.push(format!("sample(): returned unrequested counter \"{}\"", v.path));
                    }
                    if !v.value.is_finite() {
                        bad.push(format!("sample(): \"{}\" has a non-finite value", v.path));
                    }
                    if v.at_ms <= 0 {
                        bad.push(format!("sample(): \"{}\" has no sample time", v.path));
                    }
                }
                if round == 1 && !wanted.is_empty() && values.is_empty() {
                    bad.push("sample(): the second sample returned no values at all".into());
                }
            }
            Err(e) => bad.push(format!("sample(known counters) failed: {e:#}")),
        }
    }
    for garbage in ["", "not a counter", "\\", "\\NoSuchObject-imt-contract\\NoSuchCounter", "\\(\\)"] {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| source.sample(&[garbage.to_string()]))) {
            Ok(Ok(_)) => bad.push(format!("sample([\"{garbage}\"]) succeeded; unknown paths must be an error")),
            Ok(Err(_)) => {}
            Err(_) => bad.push(format!("sample([\"{garbage}\"]) panicked")),
        }
    }
    let threads = std::thread::scope(|s| {
        let handles: Vec<_> = (0..4).map(|_| s.spawn(|| source.sample(&wanted).is_ok())).collect();
        handles.into_iter().all(|h| h.join().unwrap_or(false))
    });
    if !threads {
        bad.push("sample() failed or panicked when called from several threads".into());
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(bad)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_paths() {
        for ok in [
            "\\Processor(_Total)\\% Processor Time",
            "\\Memory\\Available MBytes",
            "\\Process(a\\b)\\ID Process",
        ] {
            assert!(is_counter_path(ok), "{ok}");
        }
        for bad in ["", "Memory\\x", "\\Memory", "\\Memory\\", "\\\\x", "\\a)(\\b", "\\a\\b\\c"] {
            assert!(!is_counter_path(bad), "{bad}");
        }
    }
}
