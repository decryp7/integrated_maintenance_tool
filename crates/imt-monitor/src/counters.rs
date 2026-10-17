//! Performance counters (Windows PDH). Path parsing and instance mapping are pure and
//! portable; the PDH binding lives in `pdh` and only compiles on Windows.

use crate::tracker::{base_fields, stamp};
use anyhow::{anyhow, bail, Result};
use imt_core::model::Record;
use std::collections::HashMap;

/// A parsed counter path such as `\Process(chrome#1)\% Processor Time`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CounterPath {
    pub object: String,
    pub instance: Option<String>,
    pub counter: String,
}

impl CounterPath {
    /// Parses `\Object(instance)\Counter` or `\Object\Counter`.
    pub fn parse(path: &str) -> Result<CounterPath> {
        let rest = path
            .trim()
            .strip_prefix('\\')
            .ok_or_else(|| anyhow!("counter path \"{path}\" must start with \\"))?;
        let mut depth = 0i32;
        let mut split = None;
        for (i, c) in rest.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                '\\' if depth == 0 => {
                    split = Some(i);
                    break;
                }
                _ => {}
            }
        }
        let split = split.ok_or_else(|| anyhow!("counter path \"{path}\" must look like \\Object(instance)\\Counter"))?;
        let (object_part, counter) = (&rest[..split], rest[split + 1..].trim());
        if counter.is_empty() {
            bail!("counter path \"{path}\" has no counter name");
        }
        let (object, instance) = match object_part.find('(') {
            Some(open) => {
                let inner = object_part[open + 1..]
                    .strip_suffix(')')
                    .ok_or_else(|| anyhow!("counter path \"{path}\" has an unbalanced instance"))?;
                (&object_part[..open], Some(inner.to_string()))
            }
            None => (object_part, None),
        };
        if object.trim().is_empty() {
            bail!("counter path \"{path}\" has no object name");
        }
        Ok(CounterPath {
            object: object.trim().to_string(),
            instance,
            counter: counter.to_string(),
        })
    }

    /// The same counter for a concrete instance (used to expand wildcards).
    pub fn with_instance(&self, instance: &str) -> CounterPath {
        CounterPath {
            instance: Some(instance.to_string()),
            ..self.clone()
        }
    }

    pub fn is_wildcard(&self) -> bool {
        self.instance.as_deref().is_some_and(|i| i.contains('*'))
    }

    pub fn is_process(&self) -> bool {
        self.object.eq_ignore_ascii_case("Process")
    }
}

impl std::fmt::Display for CounterPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.instance {
            Some(i) => write!(f, "\\{}({})\\{}", self.object, i, self.counter),
            None => write!(f, "\\{}\\{}", self.object, self.counter),
        }
    }
}

/// Splits a PDH instance name `chrome#2` into (`chrome`, 2); no suffix means index 0.
pub fn split_instance(name: &str) -> (&str, u32) {
    match name.rsplit_once('#') {
        Some((base, n)) if !base.is_empty() => match n.parse() {
            Ok(i) => (base, i),
            Err(_) => (name, 0),
        },
        _ => (name, 0),
    }
}

/// One sampled counter value.
#[derive(Debug, Clone, PartialEq)]
pub struct CounterValue {
    pub path: CounterPath,
    pub value: f64,
}

/// Where counter values come from (PDH on Windows; fakes in tests).
pub trait CounterSource {
    /// Collects every configured counter once; counters that cannot be computed yet are omitted.
    fn collect(&mut self) -> Result<Vec<CounterValue>>;
}

/// Opens the platform's counter source for `paths`.
#[cfg(windows)]
pub fn open(paths: &[CounterPath]) -> Result<Box<dyn CounterSource>> {
    Ok(Box::new(crate::pdh::Pdh::open(paths)?))
}

/// Opens the platform's counter source for `paths`.
#[cfg(not(windows))]
pub fn open(paths: &[CounterPath]) -> Result<Box<dyn CounterSource>> {
    bail!(
        "performance counters are only available on Windows; ignoring {} configured counter(s)",
        paths.len()
    )
}

/// Maps `Process` instance names to PIDs using the `ID Process` counter values.
pub fn instance_pids(values: &[CounterValue]) -> HashMap<String, u32> {
    values
        .iter()
        .filter(|v| v.path.is_process() && v.path.counter.eq_ignore_ascii_case("ID Process"))
        .filter_map(|v| Some((v.path.instance.clone()?, v.value as u32)))
        .collect()
}

/// Turns counter values into records. `Process` counters are kept only for tracked PIDs
/// (when `filter_processes`) and carry a `ProcessId`; the internal `ID Process` helper
/// counter is dropped unless the user asked for it (`wanted`).
pub fn counter_records(
    values: &[CounterValue],
    wanted: &[CounterPath],
    filter_processes: bool,
    tracked: &dyn Fn(u32) -> bool,
    ts: i64,
    host: &str,
) -> Vec<Record> {
    let pids = instance_pids(values);
    let mut out = Vec::new();
    for v in values {
        let pid = if v.path.is_process() {
            v.path.instance.as_ref().and_then(|i| pids.get(i)).copied()
        } else {
            None
        };
        if v.path.is_process() && filter_processes && !pid.is_some_and(tracked) {
            continue;
        }
        let is_helper = v.path.is_process() && v.path.counter.eq_ignore_ascii_case("ID Process");
        if is_helper
            && !wanted
                .iter()
                .any(|w| w.is_process() && w.counter.eq_ignore_ascii_case("ID Process"))
        {
            continue;
        }
        let text = v.path.to_string();
        let mut r = stamp(Record::new(0), ts);
        r.message = format!("Counter {text} = {}", fmt_value(v.value));
        r.template = Some("Counter {CounterPath} = {CounterValue}".into());
        r.fields = base_fields(host);
        r.fields.insert("CounterPath".into(), text.into());
        r.fields.insert(
            "CounterValue".into(),
            serde_json::Number::from_f64(v.value).map(Into::into).unwrap_or(0.into()),
        );
        if let Some(pid) = pid {
            r.fields.insert("ProcessId".into(), pid.into());
        }
        out.push(r);
    }
    out
}

fn fmt_value(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{v:.0}")
    } else {
        format!("{v:.3}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cv(p: &str, v: f64) -> CounterValue {
        CounterValue {
            path: CounterPath::parse(p).unwrap(),
            value: v,
        }
    }

    #[test]
    fn parses_paths() {
        let p = CounterPath::parse("\\Processor(_Total)\\% Processor Time").unwrap();
        assert_eq!(
            (p.object.as_str(), p.instance.as_deref(), p.counter.as_str()),
            ("Processor", Some("_Total"), "% Processor Time")
        );
        assert_eq!(p.to_string(), "\\Processor(_Total)\\% Processor Time");
        let p = CounterPath::parse("\\Memory\\Available MBytes").unwrap();
        assert_eq!(p.instance, None);
        assert_eq!(p.to_string(), "\\Memory\\Available MBytes");
        let p = CounterPath::parse("\\Process(a(b)#1)\\ID Process").unwrap();
        assert_eq!(p.instance.as_deref(), Some("a(b)#1"));
        assert!(CounterPath::parse("\\Process(*)\\ID Process").unwrap().is_wildcard());
        assert!(!p.is_wildcard());
        assert_eq!(p.with_instance("x").to_string(), "\\Process(x)\\ID Process");
    }

    #[test]
    fn rejects_bad_paths() {
        for bad in [
            "Processor\\x",
            "\\Processor",
            "\\Processor\\",
            "\\(x)\\c",
            "\\Proc(x\\c",
            "\\Proc(x)y\\c",
        ] {
            assert!(CounterPath::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn splits_instances() {
        assert_eq!(split_instance("chrome#2"), ("chrome", 2));
        assert_eq!(split_instance("chrome"), ("chrome", 0));
        assert_eq!(split_instance("chrome#x"), ("chrome#x", 0));
        assert_eq!(split_instance("#3"), ("#3", 0));
    }

    #[test]
    fn maps_instances_to_pids_and_filters() {
        let values = vec![
            cv("\\Process(app)\\ID Process", 10.0),
            cv("\\Process(app#1)\\ID Process", 11.0),
            cv("\\Process(app)\\% Processor Time", 5.5),
            cv("\\Process(app#1)\\% Processor Time", 7.0),
            cv("\\Process(other)\\% Processor Time", 1.0),
            cv("\\Processor(_Total)\\% Processor Time", 42.0),
        ];
        let map = instance_pids(&values);
        assert_eq!(map["app"], 10);
        assert_eq!(map["app#1"], 11);
        let wanted = vec![CounterPath::parse("\\Process(*)\\% Processor Time").unwrap()];
        let recs = counter_records(&values, &wanted, true, &|p| p == 11, 1000, "H");
        let msgs: Vec<_> = recs.iter().map(|r| r.message.as_str()).collect();
        assert_eq!(
            msgs,
            [
                "Counter \\Process(app#1)\\% Processor Time = 7",
                "Counter \\Processor(_Total)\\% Processor Time = 42"
            ]
        );
        assert_eq!(recs[0].fields["ProcessId"], 11);
        assert_eq!(recs[0].fields["CounterValue"], 7.0);
        assert!(recs[1].fields.get("ProcessId").is_none());
        // Without a process filter every instance is kept, even unmapped ones.
        let all = counter_records(&values, &wanted, false, &|_| false, 1000, "H");
        assert_eq!(all.len(), 4);
        // The helper counter is emitted when the user asked for it.
        let ask = vec![CounterPath::parse("\\Process(*)\\ID Process").unwrap()];
        assert_eq!(counter_records(&values[..2], &ask, false, &|_| true, 0, "H").len(), 2);
        assert_eq!(counter_records(&values[..2], &wanted, false, &|_| true, 0, "H").len(), 0);
    }

    #[test]
    fn formats_values() {
        assert_eq!(fmt_value(3.0), "3");
        assert_eq!(fmt_value(1.23456), "1.235");
    }

    #[cfg(not(windows))]
    #[test]
    fn unsupported_off_windows() {
        let err = open(&[CounterPath::parse("\\Memory\\Available MBytes").unwrap()]).err().unwrap();
        assert!(err.to_string().contains("only available on Windows"));
    }
}
