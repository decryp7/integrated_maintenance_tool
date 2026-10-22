//! [`AnomalyInputs`] answered from events in memory: the events grouped into instances (only what the
//! detectors read of each, see [`Slim`]) and sorted by time.

use super::anomaly::{
    is_crash_trigger, sample_id, says_stopped, AnomalyConfig, AnomalyInputs, BucketCount, Ending, Gaps, Instance, InstanceRange, Layout,
    OutlierBucket, Trigger,
};
use super::trend::{growth_series, GrowthSeries, TrendConfig};
use super::util::{median_sorted, sorted};
use crate::model::Level;
use crate::row::RowRef;
use std::collections::BTreeMap;
use std::convert::Infallible;

/// What the detectors read of an event, kept per event instead of the event (a few dozen bytes): its
/// id, level, whether it has an exception, the message only where a detector reads it (errors, and
/// messages that say the instance stopped) and the few fields the latency and growth detectors use, only
/// those the event has (texts shared between events: machine and process names repeat millions of times).
pub(crate) struct Slim<'n> {
    id: u64,
    ts: i64,
    level: Level,
    exception: bool,
    message: Box<str>,
    /// `(index in SlimNames, value)` of the fields the event has.
    fields: Box<[(u8, SlimValue)]>,
    names: &'n SlimNames,
}

/// A kept field value.
enum SlimValue {
    Number(f64),
    Text(std::sync::Arc<str>),
}

/// The fields a [`Slim`] keeps (numbers first, then texts), and the texts seen so far (shared).
pub(crate) struct SlimNames {
    numbers: Vec<String>,
    texts: Vec<String>,
    interned: std::cell::RefCell<std::collections::HashSet<std::sync::Arc<str>>>,
}

/// Distinct texts shared at most; beyond, texts are kept per event (a hostile input cannot grow the set).
const MAX_INTERNED: usize = 100_000;

impl SlimNames {
    pub(crate) fn of(cfg: &AnomalyConfig) -> SlimNames {
        let mut numbers: Vec<String> = cfg.latency_fields.iter().chain(&cfg.growth.fields).cloned().collect();
        numbers.extend(["Value", "InstanceIndex"].map(String::from));
        numbers.dedup();
        SlimNames {
            numbers,
            texts: ["Counter", "MachineName", "ProcessName", "Instance"].map(String::from).to_vec(),
            interned: Default::default(),
        }
    }

    fn intern(&self, t: &str) -> std::sync::Arc<str> {
        let mut set = self.interned.borrow_mut();
        if let Some(a) = set.get(t) {
            return a.clone();
        }
        let a: std::sync::Arc<str> = t.into();
        if set.len() < MAX_INTERNED {
            set.insert(a.clone());
        }
        a
    }

    fn slim<'n>(&'n self, r: &dyn RowRef, ts: i64) -> Slim<'n> {
        let m = r.message();
        let keep = r.level() >= Level::Error || says_stopped(m);
        Slim {
            id: r.id(),
            ts,
            level: r.level(),
            exception: r.exception().is_some(),
            message: if keep { m.into() } else { "".into() },
            fields: {
                let numbers = self
                    .numbers
                    .iter()
                    .enumerate()
                    .filter_map(|(i, n)| Some((u8::try_from(i).ok()?, SlimValue::Number(r.number_of(n)?))));
                let texts = self.texts.iter().enumerate().filter_map(|(i, n)| {
                    Some((
                        u8::try_from(self.numbers.len() + i).ok()?,
                        SlimValue::Text(self.intern(&r.text_of(n)?)),
                    ))
                });
                numbers.chain(texts).collect()
            },
            names: self,
        }
    }
}

impl RowRef for Slim<'_> {
    fn id(&self) -> u64 {
        self.id
    }
    fn ts(&self) -> Option<i64> {
        Some(self.ts)
    }
    fn level(&self) -> Level {
        self.level
    }
    fn source(&self) -> u32 {
        0
    }
    fn source_name(&self) -> &str {
        ""
    }
    fn line(&self) -> u32 {
        0
    }
    fn markers(&self) -> crate::model::MarkerMask {
        0
    }
    fn message(&self) -> &str {
        &self.message
    }
    fn template(&self) -> Option<&str> {
        None
    }
    fn exception(&self) -> Option<&str> {
        self.exception.then_some("")
    }
    fn named_text(&self, _sym: crate::row::Sym, name: &str) -> Option<std::borrow::Cow<'_, str>> {
        let i = self.names.numbers.len() + self.names.texts.iter().position(|n| n == name)?;
        self.fields.iter().find_map(|(j, v)| match v {
            SlimValue::Text(t) if usize::from(*j) == i => Some(std::borrow::Cow::Borrowed(&**t)),
            _ => None,
        })
    }
    fn named_number(&self, _sym: crate::row::Sym, name: &str) -> Option<f64> {
        let i = self.names.numbers.iter().position(|n| n == name)?;
        self.fields.iter().find_map(|(j, v)| match v {
            SlimValue::Number(x) if usize::from(*j) == i => Some(*x),
            _ => None,
        })
    }
    fn named_value(&self, _sym: crate::row::Sym, _name: &str) -> Option<serde_json::Value> {
        None
    }
    fn for_each_field(&self, _f: &mut dyn FnMut(&str, std::borrow::Cow<'_, serde_json::Value>)) {}
}

type Events<'n> = Vec<(i64, Slim<'n>)>;

/// The events of each instance in time order (instances sorted by key).
pub(crate) struct EventInputs<'n> {
    instances: Vec<(String, Events<'n>)>,
    /// The last field whose values were sorted ([`AnomalyInputs::value_count`] then `values_at`).
    values: Option<(String, Vec<f64>)>,
    /// The first ids of each bucket of a layout, in instance then time order.
    bucket_ids: Option<(Layout, usize, Vec<Vec<u32>>)>,
}

impl<'n> EventInputs<'n> {
    /// Groups `(event, instance key)` pairs (events without a timestamp are left out).
    pub(crate) fn new<R: RowRef, K: AsRef<str>>(records: impl IntoIterator<Item = (R, K)>, names: &'n SlimNames) -> EventInputs<'n> {
        let mut by_key: BTreeMap<String, Events<'n>> = BTreeMap::new();
        for (rec, key) in records {
            let key = key.as_ref();
            let Some(ts) = rec.ts() else { continue };
            let rec = names.slim(&rec, ts);
            match by_key.get_mut(key) {
                Some(v) => v.push((ts, rec)),
                None => {
                    by_key.insert(key.to_string(), vec![(ts, rec)]);
                }
            }
        }
        let instances = by_key
            .into_iter()
            .map(|(k, mut v)| {
                v.sort_by_key(|e| e.0);
                (k, v)
            })
            .collect();
        EventInputs {
            instances,
            values: None,
            bucket_ids: None,
        }
    }

    fn events(&self, instance: usize) -> &[(i64, Slim<'n>)] {
        self.instances.get(instance).map_or(&[], |(_, v)| v.as_slice())
    }

    fn sorted_values(&mut self, field: &str) -> &[f64] {
        if self.values.as_ref().is_none_or(|(f, _)| f != field) {
            let values: Vec<f64> = self
                .instances
                .iter()
                .flat_map(|(_, v)| v.iter().filter_map(|(_, r)| r.number_of(field)))
                .collect();
            self.values = Some((field.to_string(), sorted(&values)));
        }
        self.values.as_ref().map_or(&[], |(_, v)| v.as_slice())
    }
}

fn is_error(r: &dyn RowRef) -> bool {
    r.level() >= Level::Error
}

impl AnomalyInputs for EventInputs<'_> {
    type Error = Infallible;

    fn instances(&mut self) -> Result<Vec<Instance>, Infallible> {
        Ok(self
            .instances
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(key, v)| Instance {
                key: key.clone(),
                events: v.len() as u64,
                first: v[0].0,
                last: v[v.len() - 1].0,
            })
            .collect())
    }

    fn bucket_counts(&mut self, layout: &Layout) -> Result<Vec<BucketCount>, Infallible> {
        let mut out = Vec::new();
        for (i, (_, events)) in self.instances.iter().enumerate() {
            let mut counts: BTreeMap<usize, (u64, u64)> = BTreeMap::new();
            for (ts, r) in events {
                let c = counts.entry(layout.index(*ts)).or_default();
                c.0 += 1;
                c.1 += u64::from(is_error(r));
            }
            out.extend(counts.into_iter().map(|(bucket, (events, errors))| BucketCount {
                instance: i,
                bucket,
                events,
                errors,
            }));
        }
        Ok(out)
    }

    fn error_ids(&mut self, ranges: &[InstanceRange], n: usize) -> Result<Vec<Vec<u32>>, Infallible> {
        Ok(ranges
            .iter()
            .map(|r| {
                self.events(r.instance)
                    .iter()
                    .filter(|(ts, e)| is_error(e) && *ts >= r.from && *ts <= r.to)
                    .map(|(_, e)| sample_id(e.id))
                    .take(n)
                    .collect()
            })
            .collect())
    }

    fn gaps(&mut self, instances: &[usize], at_least: f64) -> Result<Vec<Gaps>, Infallible> {
        Ok(instances
            .iter()
            .map(|&i| {
                let events = self.events(i);
                let gaps: Vec<f64> = events.windows(2).map(|w| (w[1].0 - w[0].0) as f64).collect();
                Gaps {
                    median: median_sorted(&sorted(&gaps)),
                    long: events
                        .windows(2)
                        .filter(|w| (w[1].0 - w[0].0) as f64 >= at_least)
                        .map(|w| (w[0].0, w[1].0))
                        .collect(),
                }
            })
            .collect())
    }

    fn endings(&mut self, crash_window: i64) -> Result<Vec<Ending>, Infallible> {
        Ok(self
            .instances
            .iter()
            .map(|(_, events)| {
                let Some((last_ts, last)) = events.last() else {
                    return Ending {
                        last_id: 0,
                        stopped: false,
                        trigger: None,
                    };
                };
                let trigger = events
                    .iter()
                    .rev()
                    .take_while(|(ts, _)| *ts >= last_ts - crash_window)
                    .find(|(_, r)| is_crash_trigger(r.level, r.exception))
                    .map(|(ts, r)| Trigger {
                        ts: *ts,
                        id: sample_id(r.id),
                        level: r.level,
                        message: r.message.to_string(),
                    });
                Ending {
                    last_id: sample_id(last.id),
                    stopped: says_stopped(last.message()),
                    trigger,
                }
            })
            .collect())
    }

    fn latest_error_ids(&mut self, instances: &[usize], n: usize) -> Result<Vec<Vec<u32>>, Infallible> {
        Ok(instances
            .iter()
            .map(|&i| {
                self.events(i)
                    .iter()
                    .rev()
                    .filter(|e| is_error(&e.1))
                    .map(|e| sample_id(e.1.id))
                    .take(n)
                    .collect()
            })
            .collect())
    }

    fn value_count(&mut self, field: &str) -> Result<usize, Infallible> {
        Ok(self.sorted_values(field).len())
    }

    fn values_at(&mut self, field: &str, ranks: &[usize]) -> Result<Vec<f64>, Infallible> {
        let values = self.sorted_values(field);
        Ok(ranks.iter().filter_map(|r| values.get(*r).copied()).collect())
    }

    fn outliers(&mut self, field: &str, above: f64, at_least: f64, layout: &Layout) -> Result<Vec<OutlierBucket>, Infallible> {
        let mut by_bucket: BTreeMap<usize, OutlierBucket> = BTreeMap::new();
        for (i, (_, events)) in self.instances.iter().enumerate() {
            for (ts, r) in events {
                let Some(v) = r.number_of(field) else { continue };
                if !(v > above && v >= at_least) {
                    continue;
                }
                let bucket = layout.index(*ts);
                let g = by_bucket.entry(bucket).or_insert_with(|| OutlierBucket {
                    bucket,
                    count: 0,
                    peak: f64::MIN,
                    first_instance: i,
                    single: true,
                    ids: Vec::new(),
                });
                g.count += 1;
                g.peak = g.peak.max(v);
                g.single &= g.first_instance == i;
                if g.ids.len() < super::anomaly::MAX_SAMPLES {
                    g.ids.push(sample_id(r.id));
                }
            }
        }
        Ok(by_bucket.into_values().collect())
    }

    fn bucket_ids(&mut self, layout: &Layout, runs: &[(usize, usize)], n: usize) -> Result<Vec<Vec<u32>>, Infallible> {
        if self.bucket_ids.as_ref().is_none_or(|(l, m, _)| l != layout || *m != n) {
            let mut ids: Vec<Vec<u32>> = vec![Vec::new(); layout.buckets];
            for (ts, r) in self.instances.iter().flat_map(|(_, v)| v) {
                let b = &mut ids[layout.index(*ts)];
                if b.len() < n {
                    b.push(sample_id(r.id));
                }
            }
            self.bucket_ids = Some((*layout, n, ids));
        }
        let ids = self.bucket_ids.as_ref().map_or(&[][..], |(_, _, v)| v.as_slice());
        Ok(runs
            .iter()
            .map(|&(a, b)| ids.get(a..=b).unwrap_or(&[]).iter().flatten().copied().take(n).collect())
            .collect())
    }

    fn growth_series(&mut self, cfg: &TrendConfig, f: &mut dyn FnMut(GrowthSeries)) -> Result<(), Infallible> {
        for s in growth_series(self.instances.iter().map(|(k, v)| (k.as_str(), v.as_slice())), cfg) {
            f(s);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Record;
    use crate::row::RecordRow;

    #[test]
    fn slim_events_keep_only_the_fields_they_have_and_share_texts() {
        let names = SlimNames::of(&AnomalyConfig::default());
        let mut a = Record::new(1);
        a.message = "request handled".into();
        a.fields.insert("MachineName".into(), "WEB01".into());
        a.fields.insert("ElapsedMs".into(), serde_json::json!(12.5));
        let mut b = a.clone();
        b.id = 2;
        b.level = Level::Error;
        b.message = "service stopped".into();
        let (sa, sb) = (
            names.slim(&RecordRow { rec: &a, source: "s" }, 10),
            names.slim(&RecordRow { rec: &b, source: "s" }, 20),
        );
        assert_eq!(sa.fields.len(), 2, "only the fields the event has");
        assert_eq!(sa.number_of("ElapsedMs"), Some(12.5));
        assert_eq!(sa.number_of("Duration"), None);
        assert_eq!(sa.text_of("MachineName").as_deref(), Some("WEB01"));
        assert_eq!(sa.text_of("ProcessName"), None);
        assert_eq!(sa.message(), "", "a message no detector reads is not kept");
        assert_eq!(sb.message(), "service stopped");
        let (SlimValue::Text(x), SlimValue::Text(y)) = (&sa.fields[1].1, &sb.fields[1].1) else {
            panic!("texts")
        };
        assert!(std::sync::Arc::ptr_eq(x, y), "the machine name is shared");
    }

    #[test]
    fn a_slim_event_reads_as_a_row_with_only_what_detectors_keep() {
        let names = SlimNames::of(&AnomalyConfig::default());
        let mut r = Record::new(99);
        r.id = 7;
        r.level = Level::Error;
        r.message = "request failed".into();
        r.exception = Some("boom".into());
        r.template = Some("request {Verb}".into());
        r.source = 3;
        r.markers = 0b11;
        r.fields.insert("MachineName".into(), "WEB01".into());
        r.fields.insert("Value".into(), serde_json::json!(4.5));
        r.fields.insert("Other".into(), serde_json::json!("dropped"));
        let slim = names.slim(
            &RecordRow {
                rec: &r,
                source: "app.log",
            },
            1_234,
        );
        assert_eq!((slim.id(), slim.ts(), slim.level()), (7, Some(1_234), Level::Error));
        assert_eq!(slim.message(), "request failed", "errors keep their message");
        assert_eq!(slim.exception(), Some(""), "only whether there is an exception is kept");
        // What no detector reads is gone, not guessed.
        assert_eq!(
            (slim.source(), slim.source_name(), slim.line(), slim.markers(), slim.template()),
            (0, "", 0, 0, None)
        );
        let sym = crate::row::Sym(0);
        assert_eq!(slim.named_number(sym, "Value"), Some(4.5));
        assert_eq!(slim.named_number(sym, "MachineName"), None, "a text is not a number");
        assert_eq!(slim.named_text(sym, "MachineName").as_deref(), Some("WEB01"));
        assert_eq!(slim.named_text(sym, "Value"), None, "a number is not a text");
        assert_eq!(slim.named_text(sym, "Other"), None);
        assert_eq!(slim.named_value(sym, "Value"), None);
        let mut seen = 0;
        slim.for_each_field(&mut |_, _| seen += 1);
        assert_eq!(seen, 0, "fields are only reachable by name");
        let quiet = names.slim(
            &RecordRow {
                rec: &Record::new(1),
                source: "s",
            },
            5,
        );
        assert_eq!(quiet.exception(), None);
    }

    #[test]
    fn answers_for_unknown_instances_and_ranks_are_empty() {
        let names = SlimNames::of(&AnomalyConfig::default());
        let mut inputs = EventInputs::new(std::iter::empty::<(RecordRow<'_>, &str)>(), &names);
        assert!(inputs.instances().unwrap().is_empty());
        assert_eq!(
            inputs
                .error_ids(
                    &[InstanceRange {
                        instance: 3,
                        from: 0,
                        to: 1
                    }],
                    5
                )
                .unwrap(),
            vec![Vec::<u32>::new()]
        );
        assert_eq!(inputs.values_at("ElapsedMs", &[0, 9]).unwrap(), Vec::<f64>::new());
        let layout = Layout {
            start: 0,
            end: 10,
            bucket: 10,
            buckets: 1,
        };
        assert_eq!(inputs.bucket_ids(&layout, &[(0, 3)], 5).unwrap(), vec![Vec::<u32>::new()]);
    }
}
