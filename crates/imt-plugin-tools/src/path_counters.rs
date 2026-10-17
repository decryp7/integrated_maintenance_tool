//! Live counters for any Performance Monitor path (`\Object(instance)\Counter`),
//! read through one reusable query: on Windows the PDH binding of
//! `imt-monitor`. The query is reopened with the union of the paths asked for
//! so far, so rate counters keep their baseline between calls; a path the
//! query cannot add is an error naming it and leaves the query as it was.
//!
//! The query itself is a port ([`Opener`]), so the caching and mapping here
//! are tested on every platform with a fake.

use anyhow::{bail, Context, Result};
use imt_api_perf::{is_counter_path, CounterSample, CounterSource};
use imt_monitor::counters::{CounterPath, CounterSource as Query};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Most counters held in one query; beyond it the query restarts with only the requested ones.
pub const MAX_QUERY_COUNTERS: usize = 256;

/// Opens a query for counter paths (`imt_monitor::counters::open` on Windows).
pub type Opener = Box<dyn Fn(&[CounterPath]) -> Result<Box<dyn Query>> + Send + Sync>;

/// The open query and the paths it holds.
#[derive(Default)]
struct Open {
    paths: Vec<CounterPath>,
    query: Option<Box<dyn Query>>,
}

/// A [`CounterSource`] over arbitrary counter paths.
pub struct PathCounters {
    opener: Opener,
    /// Paths offered by [`CounterSource::counters`]; any other valid path can be sampled too.
    listed: Vec<String>,
    open: Mutex<Open>,
}

impl PathCounters {
    pub fn new(opener: Opener, listed: &[&str]) -> PathCounters {
        PathCounters {
            opener,
            listed: listed.iter().map(|c| c.to_string()).collect(),
            open: Mutex::new(Open::default()),
        }
    }

    /// Parses a requested path; wildcard instances are refused (one value per requested path).
    fn parse(path: &str) -> Result<CounterPath> {
        if !is_counter_path(path) {
            bail!("unknown counter \"{path}\": use \\Object(instance)\\Counter or \\Object\\Counter");
        }
        let parsed = CounterPath::parse(path).with_context(|| format!("unknown counter \"{path}\""))?;
        if parsed.is_wildcard() {
            bail!("counter \"{path}\" has a wildcard instance; name one instance");
        }
        Ok(parsed)
    }
}

impl Open {
    /// Makes sure the query holds every path in `wanted`, reopening it with the union when needed.
    fn ensure(&mut self, wanted: &[CounterPath], opener: &Opener) -> Result<&mut Box<dyn Query>> {
        let missing: Vec<&CounterPath> = wanted.iter().filter(|p| !self.paths.contains(p)).collect();
        let query = match self.query.take() {
            Some(q) if missing.is_empty() => q,
            old => {
                let mut paths = self.paths.clone();
                paths.extend(missing.into_iter().cloned());
                if paths.len() > MAX_QUERY_COUNTERS {
                    paths = wanted.to_vec();
                }
                match opener(&paths) {
                    Ok(q) => {
                        self.paths = paths;
                        q
                    }
                    Err(e) => {
                        self.query = old;
                        return Err(e);
                    }
                }
            }
        };
        Ok(self.query.insert(query))
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(1)
        .max(1)
}

impl CounterSource for PathCounters {
    fn counters(&self) -> Result<Vec<String>> {
        Ok(self.listed.clone())
    }

    fn sample(&self, paths: &[String]) -> Result<Vec<CounterSample>> {
        let wanted = paths.iter().map(|p| PathCounters::parse(p)).collect::<Result<Vec<_>>>()?;
        if wanted.is_empty() {
            return Ok(Vec::new());
        }
        if wanted.len() > MAX_QUERY_COUNTERS {
            bail!("at most {MAX_QUERY_COUNTERS} counters can be sampled at once");
        }
        let mut open = self.open.lock().unwrap_or_else(|p| p.into_inner());
        let values = match open.ensure(&wanted, &self.opener)?.collect() {
            Ok(v) => v,
            Err(e) => {
                // A broken query is reopened by the next call.
                *open = Open::default();
                return Err(e.context("cannot read the performance counters"));
            }
        };
        drop(open);
        let at_ms = now_ms();
        Ok(paths
            .iter()
            .zip(&wanted)
            .filter_map(|(text, path)| {
                let v = values.iter().find(|v| &v.path == path)?;
                v.value.is_finite().then(|| CounterSample {
                    path: text.clone(),
                    value: v.value,
                    at_ms,
                })
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_monitor::counters::CounterValue;
    use std::sync::{Arc, Mutex};

    /// A fake PDH: knows `\Memory\*` and `\Processor(_Total)\*`; the processor counter is a
    /// rate (no value on the first collect of a query); `\Memory\Broken` yields NaN.
    struct FakeQuery {
        paths: Vec<CounterPath>,
        collects: usize,
        fail: Arc<Mutex<bool>>,
    }

    impl Query for FakeQuery {
        fn collect(&mut self) -> Result<Vec<CounterValue>> {
            if *self.fail.lock().unwrap() {
                bail!("PdhCollectQueryData failed");
            }
            self.collects += 1;
            Ok(self
                .paths
                .iter()
                .filter(|p| p.object != "Processor" || self.collects > 1)
                .map(|p| CounterValue {
                    path: p.clone(),
                    value: if p.counter == "Broken" { f64::NAN } else { 42.0 },
                })
                .collect())
        }
    }

    struct Fake {
        opened: Arc<Mutex<Vec<usize>>>,
        fail: Arc<Mutex<bool>>,
    }

    fn fake() -> (PathCounters, Fake) {
        let f = Fake {
            opened: Arc::default(),
            fail: Arc::default(),
        };
        let (opened, fail) = (f.opened.clone(), f.fail.clone());
        let opener: Opener = Box::new(move |paths| {
            if let Some(bad) = paths.iter().find(|p| p.object != "Memory" && p.object != "Processor") {
                bail!("PdhAddEnglishCounter failed for {bad} (PDH status 0xC0000BB8)");
            }
            opened.lock().unwrap().push(paths.len());
            Ok(Box::new(FakeQuery {
                paths: paths.to_vec(),
                collects: 0,
                fail: fail.clone(),
            }))
        });
        let source = PathCounters::new(opener, &["\\Memory\\Available Bytes", "\\Processor(_Total)\\% Processor Time"]);
        (source, f)
    }

    fn s(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn passes_the_counter_source_contract() {
        let (source, _) = fake();
        imt_api_perf::contract_tests(&source).unwrap();
    }

    #[test]
    fn any_path_is_sampled_through_one_growing_query() {
        let (source, f) = fake();
        let got = source.sample(&s(&["\\Memory\\Pool Paged Bytes"])).unwrap();
        assert_eq!((got[0].path.as_str(), got[0].value), ("\\Memory\\Pool Paged Bytes", 42.0));
        assert!(got[0].at_ms > 0);
        // A new path reopens the query with the union; known paths reuse it.
        let cpu = "\\Processor(_Total)\\% Processor Time";
        let first = source.sample(&s(&[cpu, "\\Memory\\Pool Paged Bytes"])).unwrap();
        assert_eq!(first.len(), 1, "a rate counter has no value on the first collect");
        let second = source.sample(&s(&[cpu])).unwrap();
        assert_eq!(second[0].path, cpu);
        assert_eq!(*f.opened.lock().unwrap(), [1, 2]);
        // The requested spelling comes back, the internal one is matched.
        let spaced = source.sample(&s(&["\\Memory\\Pool Paged Bytes "])).unwrap();
        assert_eq!(spaced[0].path, "\\Memory\\Pool Paged Bytes ");
        // Non-finite values are omitted.
        assert!(source.sample(&s(&["\\Memory\\Broken"])).unwrap().is_empty());
        assert_eq!(source.counters().unwrap().len(), 2);
    }

    #[test]
    fn bad_paths_are_errors_and_keep_the_query() {
        let (source, f) = fake();
        source.sample(&s(&["\\Memory\\A"])).unwrap();
        let e = source.sample(&s(&["\\Memory\\A", "\\Disk(C:)\\Reads"])).unwrap_err().to_string();
        assert!(e.contains("\\Disk(C:)\\Reads"), "{e}");
        assert_eq!(*f.opened.lock().unwrap(), [1], "the query was not replaced");
        assert_eq!(source.sample(&s(&["\\Memory\\A"])).unwrap().len(), 1);
        let e = source.sample(&s(&["\\Processor(*)\\% Processor Time"])).unwrap_err().to_string();
        assert!(e.contains("wildcard"), "{e}");
        let e = source.sample(&s(&["Memory"])).unwrap_err().to_string();
        assert!(e.starts_with("unknown counter"), "{e}");
        for bad in ["\\Memory(x\\y", "\\Memory(x)y\\C"] {
            let e = source.sample(&s(&[bad])).unwrap_err().to_string();
            assert!(e.starts_with("unknown counter"), "{bad}: {e}");
        }
        let many: Vec<String> = (0..=MAX_QUERY_COUNTERS).map(|i| format!("\\Memory\\C{i}")).collect();
        assert!(source.sample(&many).unwrap_err().to_string().contains("at most"));
    }

    #[test]
    fn a_full_query_restarts_with_the_requested_paths() {
        let (source, f) = fake();
        let half: Vec<String> = (0..MAX_QUERY_COUNTERS / 2 + 1).map(|i| format!("\\Memory\\A{i}")).collect();
        let other: Vec<String> = (0..MAX_QUERY_COUNTERS / 2 + 1).map(|i| format!("\\Memory\\B{i}")).collect();
        assert_eq!(source.sample(&half).unwrap().len(), half.len());
        assert_eq!(source.sample(&other).unwrap().len(), other.len());
        assert_eq!(*f.opened.lock().unwrap(), [half.len(), other.len()]);
    }

    #[test]
    fn a_failing_query_is_reported_and_reopened() {
        let (source, f) = fake();
        source.sample(&s(&["\\Memory\\A"])).unwrap();
        *f.fail.lock().unwrap() = true;
        let e = format!("{:#}", source.sample(&s(&["\\Memory\\A"])).unwrap_err());
        assert!(
            e.contains("cannot read the performance counters") && e.contains("PdhCollectQueryData"),
            "{e}"
        );
        *f.fail.lock().unwrap() = false;
        assert_eq!(source.sample(&s(&["\\Memory\\A"])).unwrap().len(), 1);
        assert_eq!(*f.opened.lock().unwrap(), [1, 1]);
    }

    #[test]
    fn a_poisoned_lock_still_samples() {
        let (source, _) = fake();
        let source = Arc::new(source);
        let s2 = source.clone();
        let _ = std::thread::spawn(move || {
            let _guard = s2.open.lock().unwrap();
            panic!("poison");
        })
        .join();
        assert_eq!(source.sample(&s(&["\\Memory\\A"])).unwrap().len(), 1);
    }
}
