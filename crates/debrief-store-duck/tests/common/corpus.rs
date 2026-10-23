//! The benchmark corpus (from the store spike's `corpus.rs`): the timed events of `/samples`, imported
//! once into a store, written back as CLEF and repeated with shifted timestamps and renamed instances
//! (`MachineName-NN`, [`INSTANCE_GROUPS`] of them) until the requested number of events is reached. A
//! marker file records what a folder holds, so a corpus is generated once and reused.

use super::{log_samples, TempDir};
use debrief_core::batch::FieldValue;
use debrief_core::ports::{Ingest, Query};
use debrief_core::{PluginRegistry, Profile};
use debrief_store_duck::DuckStore;
use serde_json::{Map, Value};
use std::io::{BufWriter, Write};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

pub const INSTANCE_GROUPS: usize = 20;

/// One timed sample event as a CLEF object without its time.
struct Sample {
    source: String,
    ts: i64,
    clef: Map<String, Value>,
}

fn level_name(code: u8) -> &'static str {
    ["Verbose", "Debug", "Information", "Warning", "Error", "Fatal"][code.min(5) as usize]
}

fn samples() -> Vec<Sample> {
    let dir = TempDir::new("corpus-src");
    let store = DuckStore::create(&dir.0, Profile::default_profile(), PluginRegistry::new()).unwrap();
    store.open(&log_samples()).unwrap().wait().unwrap();
    let names: std::collections::HashMap<u32, String> = store
        .summary()
        .unwrap()
        .sources
        .into_iter()
        .map(|s| (s.id, s.name.replace(['/', '\\', ':'], "_")))
        .collect();
    let mut out = Vec::new();
    store
        .scan(&Default::default(), &mut |b| {
            for r in b.rows() {
                let Some(ts) = r.ts() else { continue };
                let mut o = Map::new();
                o.insert("@l".into(), Value::String(level_name(r.level().get()).into()));
                o.insert("@m".into(), Value::String(r.message().into()));
                if let Some(t) = r.template() {
                    o.insert("@mt".into(), Value::String(t.into()));
                }
                if let Some(x) = r.exception() {
                    o.insert("@x".into(), Value::String(x.into()));
                }
                for (k, v) in r.fields() {
                    let v = match v {
                        FieldValue::Text(s) => Value::String(s.into()),
                        FieldValue::Int(i) => Value::from(i),
                        FieldValue::Float(f) => Value::from(f),
                        FieldValue::Bool(b) => Value::Bool(b),
                    };
                    o.insert(k.to_string(), v);
                }
                out.push(Sample {
                    source: names.get(&r.source().0).cloned().unwrap_or_default(),
                    ts,
                    clef: o,
                });
            }
            ControlFlow::Continue(())
        })
        .unwrap();
    out
}

/// A corpus of at least `events` events in `dir` (generated unless the marker says it is there).
pub fn corpus(dir: &Path, events: u64) -> PathBuf {
    let marker = dir.join("corpus.txt");
    if std::fs::read_to_string(&marker).ok().and_then(|s| s.trim().parse::<u64>().ok()) == Some(events) {
        return dir.to_path_buf();
    }
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    let samples = samples();
    let from = samples.iter().map(|s| s.ts).min().unwrap();
    let to = samples.iter().map(|s| s.ts).max().unwrap();
    let period = ((to - from) / 60_000 + 2) * 60_000;
    let mut by_source: std::collections::BTreeMap<&str, Vec<&Sample>> = Default::default();
    for s in &samples {
        by_source.entry(&s.source).or_default().push(s);
    }
    let mut written = 0u64;
    'copies: for k in 0.. {
        let copy = dir.join(format!("copy-{k:04}"));
        std::fs::create_dir_all(&copy).unwrap();
        for (name, rows) in &by_source {
            let mut w = BufWriter::new(std::fs::File::create(copy.join(format!("{name}.clef"))).unwrap());
            for s in rows {
                if written >= events {
                    break 'copies;
                }
                let mut o = Map::new();
                let t = chrono::DateTime::from_timestamp_millis(s.ts + k as i64 * period).unwrap_or_default();
                o.insert("@t".into(), Value::String(t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()));
                for (key, v) in &s.clef {
                    let v = match (key.as_str(), v) {
                        ("MachineName", Value::String(m)) => Value::String(format!("{m}-{:02}", k % INSTANCE_GROUPS)),
                        _ => v.clone(),
                    };
                    o.insert(key.clone(), v);
                }
                serde_json::to_writer(&mut w, &Value::Object(o)).unwrap();
                w.write_all(b"\n").unwrap();
                written += 1;
            }
        }
    }
    std::fs::write(&marker, events.to_string()).unwrap();
    dir.to_path_buf()
}
