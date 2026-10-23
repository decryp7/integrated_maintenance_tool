//! What a segment's writer learns while writing it, stored beside it in the catalog so the read side does
//! not scan the events for it:
//!
//! - `segment_fields(file, key, rows, numeric_rows, distinct_est, top_values)`: per field (declared and
//!   `extra`), the rows that have it, how many of those are numbers, an estimate of its distinct values
//!   (exact up to [`MAX_TRACKED`] values, else a HyperLogLog estimate) and its ten most frequent values
//!   (JSON `[[value, count], ...]`, among the tracked ones);
//! - `histogram(file, minute, level, source, n)`: events per minute (`ts` div 60,000; NULL for untimed
//!   events), level and source.
//!
//! Both are exact for the rows of the segment; segments with rows of removed sources are rewritten by
//! compaction, which recomputes them.

use debrief_core::batch::{EventBatch, FieldValue};
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// Distinct values counted exactly per field.
pub const MAX_TRACKED: usize = 256;
/// Fields tracked per segment.
pub const MAX_FIELDS: usize = 512;
/// Longest value tracked for top values (longer ones only count towards the estimate).
const MAX_VALUE_LEN: usize = 200;
/// Most frequent values kept per field.
pub const TOP: usize = 10;

const HLL_BITS: u32 = 10;

/// FxHash (rustc's): fast, not DoS-resistant (keys are field names and values of the user's own files).
#[derive(Default, Clone, Copy)]
struct Fx(u64);

impl Hasher for Fx {
    fn write(&mut self, bytes: &[u8]) {
        for c in bytes.chunks(8) {
            let mut w = [0u8; 8];
            w[..c.len()].copy_from_slice(c);
            self.0 = (self.0.rotate_left(5) ^ u64::from_le_bytes(w)).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
        }
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

type FxMap<K, V> = HashMap<K, V, BuildHasherDefault<Fx>>;

/// A value's hash, mixed (murmur3's finaliser) so its top bits suit the sketch.
fn value_hash(v: &str) -> u64 {
    let mut h = Fx::default();
    h.write(v.as_bytes());
    h.write(&[0xff]);
    let mut x = h.finish();
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    x ^ (x >> 33)
}

/// A HyperLogLog sketch with 1,024 registers (about 3 % error).
#[derive(Clone)]
struct Hll([u8; 1 << HLL_BITS]);

impl Hll {
    fn add(&mut self, x: u64) {
        let i = (x >> (64 - HLL_BITS)) as usize;
        let rank = ((x << HLL_BITS) | (1 << (HLL_BITS - 1))).leading_zeros() as u8 + 1;
        self.0[i] = self.0[i].max(rank);
    }

    fn estimate(&self) -> u64 {
        let m = self.0.len() as f64;
        let sum: f64 = self.0.iter().map(|&r| 2f64.powi(-(r as i32))).sum();
        let raw = 0.7213 / (1.0 + 1.079 / m) * m * m / sum;
        let zeros = self.0.iter().filter(|&&r| r == 0).count();
        if raw <= 2.5 * m && zeros > 0 {
            (m * (m / zeros as f64).ln()).round() as u64
        } else {
            raw.round() as u64
        }
    }
}

struct FieldAcc {
    rows: u64,
    numeric: u64,
    /// Value hash -> (value, count), at most [`MAX_TRACKED`].
    values: FxMap<u64, (String, u64)>,
    /// More distinct values than tracked were seen: from then on every value also feeds the sketch.
    hll: Option<Box<Hll>>,
}

impl FieldAcc {
    fn new() -> FieldAcc {
        FieldAcc {
            rows: 0,
            numeric: 0,
            values: FxMap::default(),
            hll: None,
        }
    }

    fn add(&mut self, text: &str) {
        let h = value_hash(text);
        if let Some(hll) = &mut self.hll {
            hll.add(h);
        }
        if let Some(e) = self.values.get_mut(&h) {
            e.1 += 1;
        } else if self.values.len() < MAX_TRACKED && text.len() <= MAX_VALUE_LEN {
            self.values.insert(h, (text.to_string(), 1));
        } else if self.hll.is_none() {
            // Overflow: the sketch starts with what was tracked so far and this value.
            let mut hll = Box::new(Hll([0; 1 << HLL_BITS]));
            for k in self.values.keys() {
                hll.add(*k);
            }
            hll.add(h);
            self.hll = Some(hll);
        }
    }
}

/// One row of `segment_fields`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldStat {
    pub key: String,
    pub rows: u64,
    pub numeric_rows: u64,
    pub distinct: u64,
    /// JSON `[[value, count], ...]`, most frequent first.
    pub top: String,
}

/// One row of `histogram`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Bucket {
    pub minute: Option<i64>,
    pub level: u8,
    pub source: u32,
}

/// Accumulates the statistics of one segment.
#[derive(Default)]
pub struct SegmentStats {
    fields: FxMap<String, FieldAcc>,
    histogram: FxMap<Bucket, u64>,
}

fn is_number(v: &FieldValue<'_>) -> bool {
    match v {
        FieldValue::Int(_) | FieldValue::Float(_) => true,
        FieldValue::Text(t) => {
            let t = t.trim();
            t.as_bytes()
                .first()
                .is_some_and(|b| b.is_ascii_digit() || matches!(b, b'-' | b'+' | b'.'))
                && t.parse::<f64>().is_ok_and(f64::is_finite)
        }
        FieldValue::Bool(_) => false,
    }
}

impl SegmentStats {
    pub fn add(&mut self, batch: &EventBatch) {
        for r in batch.rows() {
            *self
                .histogram
                .entry(Bucket {
                    minute: r.ts().map(|t| t.div_euclid(60_000)),
                    level: r.level().get(),
                    source: r.source().0,
                })
                .or_default() += 1;
            for (k, v) in r.fields() {
                if !self.fields.contains_key(k) {
                    if self.fields.len() >= MAX_FIELDS {
                        continue;
                    }
                    self.fields.insert(k.to_string(), FieldAcc::new());
                }
                let Some(acc) = self.fields.get_mut(k) else { continue };
                acc.rows += 1;
                acc.numeric += u64::from(is_number(&v));
                let rendered;
                let text: &str = match v {
                    FieldValue::Text(t) => t,
                    other => {
                        rendered = other.render();
                        &rendered
                    }
                };
                acc.add(text);
            }
        }
    }

    /// The `segment_fields` rows, by key.
    pub fn fields(&self) -> Vec<FieldStat> {
        let mut out: Vec<FieldStat> = self
            .fields
            .iter()
            .map(|(k, a)| {
                let mut top: Vec<(&String, &u64)> = a.values.values().map(|(v, n)| (v, n)).collect();
                top.sort_by(|x, y| y.1.cmp(x.1).then(x.0.cmp(y.0)));
                top.truncate(TOP);
                FieldStat {
                    key: k.clone(),
                    rows: a.rows,
                    numeric_rows: a.numeric,
                    distinct: match &a.hll {
                        Some(hll) => hll.estimate().max(a.values.len() as u64 + 1),
                        None => a.values.len() as u64,
                    },
                    top: serde_json::to_string(&top).unwrap_or_else(|_| "[]".into()),
                }
            })
            .collect();
        out.sort_by(|a, b| a.key.cmp(&b.key));
        out
    }

    /// The `histogram` rows, in bucket order.
    pub fn histogram(&self) -> Vec<(Bucket, u64)> {
        let mut v: Vec<(Bucket, u64)> = self.histogram.iter().map(|(b, n)| (*b, *n)).collect();
        v.sort();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use debrief_core::batch::{EventBatchBuilder, EventId, EventSchema, EventTime, LevelCode, NewEvent, SourceId};
    use std::sync::Arc;

    #[test]
    fn counts_fields_values_and_minutes() {
        let mut b = EventBatchBuilder::new(Arc::new(EventSchema::plain()));
        for i in 0..1000u32 {
            let ev = NewEvent {
                time: if i == 0 {
                    EventTime::Untimed
                } else {
                    EventTime::Parsed(i as i64 * 1000)
                },
                level: LevelCode::new((i % 2) as u8).unwrap(),
                source: SourceId(7),
                line: i + 1,
                byte_offset: 0,
                message: "m",
                template: None,
                exception: None,
                markers: 0,
                raw: "r",
            };
            let id = i.to_string();
            let host = format!("h{}", i % 3);
            b.push(
                &ev,
                [
                    ("Id", FieldValue::Text(&id)),
                    ("Host", FieldValue::Text(&host)),
                    ("Word", FieldValue::Text("x")),
                ],
            )
            .unwrap();
        }
        let batch = b.finish().assign_ids(EventId(0)).unwrap();
        let mut s = SegmentStats::default();
        s.add(&batch);
        let f = s.fields();
        let get = |k: &str| f.iter().find(|x| x.key == k).unwrap().clone();
        let host = get("Host");
        assert_eq!((host.rows, host.numeric_rows, host.distinct), (1000, 0, 3));
        assert!(host.top.starts_with("[[\"h0\",334]"), "{}", host.top);
        let id = get("Id");
        assert_eq!(id.numeric_rows, 1000);
        assert!((900..=1100).contains(&id.distinct), "estimated {}", id.distinct);
        let h = s.histogram();
        assert_eq!(h[0].0.minute, None, "the untimed event comes first");
        assert_eq!(h.iter().map(|(_, n)| n).sum::<u64>(), 1000);
        assert_eq!(h.iter().filter(|(b, _)| b.minute == Some(16)).map(|(_, n)| n).sum::<u64>(), 40);
        // An empty sketch estimates nothing.
        assert_eq!(Hll([0; 1 << HLL_BITS]).estimate(), 0);
        assert_ne!(value_hash("a"), value_hash("b"));
    }
}
