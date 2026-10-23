//! Clock-skew correction: per-source time offsets so the timelines of
//! machines whose clocks disagree line up.
//!
//! An offset is set manually, by aligning one event of a source on the same
//! real-world event of another ([`align_offset`]), or from a suggestion that
//! pairs events sharing a correlation id across sources ([`suggest`]).
//!
//! Offsets are keyed by source path (stable across reloads, rotation and
//! projects) and applied by the workspace when records enter it (import, live
//! tail, reload, project restore): the stored timestamp is the shifted one.
//!
//! Why on load rather than at query time: records are kept in time order and
//! a record's id is its position, which every view, the match cache, paging,
//! the timeline, charts and all analyses rely on. Shifting at query time would
//! leave the store out of time order and force every one of them to re-sort or
//! translate; shifting on load keeps one invariant and one code path, so all
//! views reflect the shift for free. Changing an offset moves one source's
//! records with the merge live tail already uses (two sorted runs, O(n), never
//! a full sort); ids before the first moved record stay, later ones are
//! renumbered exactly as a fresh load with that offset would number them (so
//! ids saved in a project, e.g. bookmarks, match after reopening), and the
//! command returns the new ids of the records the caller tracks. The original
//! timestamp is `ts - offset`; page rows carry the offset as `skewMs`.

mod store;
mod types;

pub use store::{suggest_over, MAX_SCAN};
pub use types::*;

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// Largest offset accepted (either sign): a week covers time-zone mistakes
/// and wrong dates; anything larger is a different problem.
pub const MAX_OFFSET_MS: i64 = 7 * 24 * 3_600_000;

/// Correlation-id fields tried by [`suggest`] when the request names none.
pub const DEFAULT_ID_FIELDS: [&str; 3] = ["CorrelationId", "TraceId", "RequestId"];

/// Default plausibility bound of a paired delta: one hour.
pub const DEFAULT_MAX_DELTA_MS: i64 = 3_600_000;

/// Default tolerance around the median within which a pair counts as agreeing.
pub const DEFAULT_TOLERANCE_MS: i64 = 1_000;

/// Fewest agreeing pairs a suggestion needs.
pub const MIN_PAIRS: usize = 3;

/// Pairs at which the sample-size factor of the confidence reaches 1.
const FULL_CONFIDENCE_PAIRS: f64 = 10.0;

/// Most distinct correlation ids examined (bounded memory on huge inputs).
pub const MAX_IDS: usize = 500_000;

/// Why an offset was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkewError {
    /// Larger than [`MAX_OFFSET_MS`] in magnitude.
    TooLarge(i64),
    /// No source with this id.
    UnknownSource(u32),
    /// No record with this id.
    UnknownRecord(u32),
    /// The record has no timestamp to align on.
    Untimed(u32),
    /// Both records of an alignment belong to the same source.
    SameSource(u32),
}

impl std::fmt::Display for SkewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkewError::TooLarge(ms) => write!(
                f,
                "a clock offset of {} is too large; use at most ±{} (7 days)",
                format_offset(*ms),
                format_offset(MAX_OFFSET_MS)
            ),
            SkewError::UnknownSource(id) => write!(f, "source {id} is not open any more; reload the sources and try again"),
            SkewError::UnknownRecord(id) => write!(f, "event {id} no longer exists; select the events again"),
            SkewError::Untimed(id) => write!(f, "event {id} has no timestamp, so it cannot be used to align clocks"),
            SkewError::SameSource(id) => write!(
                f,
                "both events come from the same source ({id}); pick the matching event in another source"
            ),
        }
    }
}

impl std::error::Error for SkewError {}

/// An offset checked against [`MAX_OFFSET_MS`].
pub fn checked_offset(ms: i64) -> Result<i64, SkewError> {
    if ms.checked_abs().is_none_or(|a| a > MAX_OFFSET_MS) {
        return Err(SkewError::TooLarge(ms));
    }
    Ok(ms)
}

/// `+2.35 s`, `-1 h 2 min 3.5 s`, `0 s`: an offset for messages.
pub fn format_offset(ms: i64) -> String {
    let sign = if ms < 0 { "-" } else { "+" };
    let mut rest = ms.unsigned_abs();
    let mut parts = Vec::new();
    for (unit, size) in [("d", 86_400_000u64), ("h", 3_600_000), ("min", 60_000)] {
        if rest >= size {
            parts.push(format!("{} {unit}", rest / size));
            rest %= size;
        }
    }
    if rest > 0 || parts.is_empty() {
        let s = format!("{:.3}", rest as f64 / 1000.0);
        let s = s.trim_end_matches('0').trim_end_matches('.');
        parts.push(format!("{s} s"));
    }
    if ms == 0 {
        return "0 s".into();
    }
    format!("{sign}{}", parts.join(" "))
}

/// Per-source offsets in milliseconds, keyed by source path. Zero offsets are
/// not stored.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ClockOffsets(BTreeMap<String, i64>);

impl ClockOffsets {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The offset of a source path (0 when unset).
    pub fn get(&self, path: &str) -> i64 {
        self.0.get(path).copied().unwrap_or(0)
    }

    /// Sets (or with 0, clears) a source's offset; returns the previous one.
    pub fn set(&mut self, path: &str, ms: i64) -> Result<i64, SkewError> {
        let ms = checked_offset(ms)?;
        let old = if ms == 0 {
            self.0.remove(path)
        } else {
            self.0.insert(path.to_string(), ms)
        };
        Ok(old.unwrap_or(0))
    }

    /// Offsets parsed from untrusted input (a project file): out-of-range
    /// values are dropped rather than failing the whole project.
    pub fn sanitized(self) -> ClockOffsets {
        ClockOffsets(
            self.0
                .into_iter()
                .filter(|(_, ms)| *ms != 0 && checked_offset(*ms).is_ok())
                .collect(),
        )
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &i64)> {
        self.0.iter()
    }

    /// The same offsets with every key passed through `f` (e.g. to store
    /// project-relative paths).
    pub fn map_keys(&self, f: impl Fn(&str) -> String) -> ClockOffsets {
        ClockOffsets(self.0.iter().map(|(k, v)| (f(k), *v)).collect())
    }
}

/// Shifts a timestamp, saturating instead of overflowing.
pub fn shift(ts: Option<i64>, offset: i64) -> Option<i64> {
    ts.map(|t| t.saturating_add(offset))
}

/// The offset for the target's source that puts the target event at the
/// reference event's (already corrected) time: `tsA − tsB`, where `tsA` is
/// the reference as shown and `tsB` the target's original timestamp.
pub fn align_offset(reference_shown: i64, target_original: i64) -> Result<i64, SkewError> {
    checked_offset(reference_shown.saturating_sub(target_original))
}

/// One correlated event: its source, id value, and original timestamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdEvent<'a> {
    pub source: u32,
    pub id: &'a str,
    pub original_ts: i64,
}

/// Suggests offsets from events sharing correlation ids across sources.
///
/// For each id, the earliest occurrence per source counts. For every source
/// paired with the reference, the deltas `reference_shown − source_original`
/// within `±max_delta_ms` are collected; the median is the suggestion and
/// the share of deltas within `tolerance_ms` of it (scaled down below ten
/// pairs) is the confidence. Noisy ids (reused, unrelated) spread the deltas
/// and lower the confidence instead of moving the median much. Note that
/// causally related events (request sent, then received) are not exactly
/// simultaneous: the suggestion is accurate up to the typical latency.
pub fn suggest(events: &[IdEvent<'_>], offsets: &HashMap<u32, i64>, req: &SuggestRequest) -> SuggestResult {
    let off = |s: u32| offsets.get(&s).copied().unwrap_or(0);
    // id -> source -> earliest original ts
    let mut by_id: HashMap<&str, BTreeMap<u32, i64>> = HashMap::new();
    for e in events {
        if e.id.is_empty() {
            continue;
        }
        if !by_id.contains_key(e.id) && by_id.len() >= MAX_IDS {
            continue;
        }
        let slot = by_id.entry(e.id).or_default().entry(e.source).or_insert(e.original_ts);
        *slot = (*slot).min(e.original_ts);
    }
    by_id.retain(|_, m| m.len() > 1);

    // The reference: requested, else the source sharing ids with most others.
    let mut links: BTreeMap<u32, usize> = BTreeMap::new();
    for m in by_id.values() {
        for s in m.keys() {
            *links.entry(*s).or_default() += m.len() - 1;
        }
    }
    let reference = req
        .reference
        .or_else(|| links.iter().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0))).map(|(s, _)| *s));
    let Some(reference) = reference else {
        return SuggestResult::default();
    };

    let bound = req.max_delta_ms.clamp(1, MAX_OFFSET_MS);
    let tolerance = req.tolerance_ms.clamp(0, bound);
    let mut deltas: BTreeMap<u32, Vec<i64>> = BTreeMap::new();
    for m in by_id.values() {
        let Some(&r) = m.get(&reference) else { continue };
        let shown = r.saturating_add(off(reference));
        for (&s, &t) in m.iter().filter(|(s, _)| **s != reference) {
            let d = shown.saturating_sub(t);
            if d.abs() <= bound {
                deltas.entry(s).or_default().push(d);
            }
        }
    }
    let mut result = SuggestResult {
        reference: Some(reference),
        fields: req.id_fields(),
        ..Default::default()
    };
    let mut candidates: Vec<u32> = links.keys().copied().filter(|s| *s != reference).collect();
    candidates.sort_unstable();
    for s in candidates {
        let Some(mut d) = deltas.remove(&s).filter(|d| d.len() >= MIN_PAIRS) else {
            result.insufficient.push(s);
            continue;
        };
        let median = median(&mut d);
        let mut dev: Vec<i64> = d.iter().map(|x| (x - median).abs()).collect();
        let spread = median_of(&mut dev);
        let agreeing = d.iter().filter(|x| (*x - median).abs() <= tolerance).count();
        if agreeing < MIN_PAIRS {
            result.insufficient.push(s);
            continue;
        }
        let share = agreeing as f64 / d.len() as f64;
        let size = (d.len() as f64 / FULL_CONFIDENCE_PAIRS).min(1.0);
        result.suggestions.push(OffsetSuggestion {
            source: s,
            reference,
            current_ms: off(s),
            suggested_ms: median.clamp(-MAX_OFFSET_MS, MAX_OFFSET_MS),
            pairs: d.len(),
            agreeing,
            spread_ms: spread,
            confidence: (share * size * 1000.0).round() / 1000.0,
        });
    }
    result
}

/// Median of a non-empty list (lower median for even lengths, an observed delta).
fn median(v: &mut [i64]) -> i64 {
    median_of(v)
}

fn median_of(v: &mut [i64]) -> i64 {
    v.sort_unstable();
    v[(v.len() - 1) / 2]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(source: u32, id: &'static str, original_ts: i64) -> IdEvent<'static> {
        IdEvent { source, id, original_ts }
    }

    #[test]
    fn offsets_are_bounded_and_formatted() {
        assert_eq!(checked_offset(MAX_OFFSET_MS), Ok(MAX_OFFSET_MS));
        assert_eq!(checked_offset(-MAX_OFFSET_MS), Ok(-MAX_OFFSET_MS));
        assert_eq!(checked_offset(MAX_OFFSET_MS + 1), Err(SkewError::TooLarge(MAX_OFFSET_MS + 1)));
        assert!(checked_offset(i64::MIN).is_err());
        assert_eq!(format_offset(2_350), "+2.35 s");
        assert_eq!(format_offset(-2_000), "-2 s");
        assert_eq!(format_offset(0), "0 s");
        assert_eq!(format_offset(3_723_500), "+1 h 2 min 3.5 s");
        assert_eq!(format_offset(86_400_000), "+1 d");
        assert!(SkewError::TooLarge(MAX_OFFSET_MS + 1).to_string().contains("too large"));
        assert!(SkewError::UnknownSource(3).to_string().contains("source 3"));
        assert!(SkewError::UnknownRecord(4).to_string().contains("event 4"));
        assert!(SkewError::Untimed(5).to_string().contains("no timestamp"));
        assert!(SkewError::SameSource(6).to_string().contains("same source"));
        assert_eq!(shift(Some(i64::MAX), 5), Some(i64::MAX));
        assert_eq!(shift(None, 5), None);
        assert_eq!(align_offset(10_000, 7_650), Ok(2_350));
        assert!(align_offset(MAX_OFFSET_MS * 3, 0).is_err());
    }

    #[test]
    fn offsets_store_set_clear_and_sanitize() {
        let mut o = ClockOffsets::default();
        assert!(o.is_empty());
        assert_eq!(o.set("/a.log", 500), Ok(0));
        assert_eq!(o.set("/a.log", -200), Ok(500));
        assert_eq!(o.get("/a.log"), -200);
        assert_eq!(o.get("/b.log"), 0);
        assert!(o.set("/a.log", MAX_OFFSET_MS + 1).is_err());
        assert_eq!(o.set("/a.log", 0), Ok(-200));
        assert!(o.is_empty());
        let raw: ClockOffsets = serde_json::from_str(r#"{"/a": 5, "/b": 0, "/c": 999999999999}"#).unwrap();
        let clean = raw.sanitized();
        assert_eq!(clean.iter().collect::<Vec<_>>(), vec![(&"/a".to_string(), &5)]);
        assert_eq!(clean.map_keys(|k| format!("x{k}")).get("x/a"), 5);
        assert_eq!(serde_json::to_string(&clean).unwrap(), r#"{"/a":5}"#);
    }

    #[test]
    fn suggests_the_median_of_shared_ids() {
        // Source 1's clock is 2.5 s behind source 0; source 2 is 10 s ahead.
        let ids = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l"];
        let mut events = Vec::new();
        for (i, id) in ids.iter().enumerate() {
            let t = 1_000_000 + i as i64 * 7_000;
            events.push(ev(0, id, t));
            events.push(ev(1, id, t - 2_500 + (i as i64 % 3) * 10));
            events.push(ev(2, id, t + 10_000));
            // A later repeat of the id in the same source is ignored.
            events.push(ev(2, id, t + 50_000));
        }
        let r = suggest(&events, &HashMap::new(), &SuggestRequest::default());
        assert_eq!(r.reference, Some(0));
        assert_eq!(r.fields, DEFAULT_ID_FIELDS.map(String::from).to_vec());
        assert!(r.insufficient.is_empty());
        let s1 = r.suggestions.iter().find(|s| s.source == 1).unwrap();
        assert_eq!(s1.suggested_ms, 2_490);
        assert_eq!((s1.pairs, s1.agreeing, s1.reference), (12, 12, 0));
        assert_eq!(s1.confidence, 1.0);
        assert!(s1.spread_ms <= 10);
        let s2 = r.suggestions.iter().find(|s| s.source == 2).unwrap();
        assert_eq!(s2.suggested_ms, -10_000);
    }

    #[test]
    fn existing_offsets_and_an_explicit_reference() {
        let ids = ["a", "b", "c", "d"];
        let events: Vec<IdEvent> = ids
            .iter()
            .enumerate()
            .flat_map(|(i, id)| [ev(0, id, i as i64 * 1000), ev(1, id, i as i64 * 1000 + 300)])
            .collect();
        // The reference itself is shifted by +1 s: the suggestion follows it.
        let offsets = HashMap::from([(0, 1_000), (1, 50)]);
        let req = SuggestRequest {
            reference: Some(0),
            ..Default::default()
        };
        let r = suggest(&events, &offsets, &req);
        let s = &r.suggestions[0];
        assert_eq!((s.source, s.current_ms, s.suggested_ms), (1, 50, 700));
        // Four pairs: confidence scaled for the small sample.
        assert_eq!(s.confidence, 0.4);
        // Reversed reference.
        let r = suggest(
            &events,
            &HashMap::new(),
            &SuggestRequest {
                reference: Some(1),
                ..Default::default()
            },
        );
        assert_eq!(r.suggestions[0].suggested_ms, 300);
    }

    #[test]
    fn noisy_ids_lower_the_confidence() {
        let mut events = Vec::new();
        let names: Vec<String> = (0..40).map(|i| format!("id{i}")).collect();
        for (i, id) in names.iter().enumerate() {
            let t = i as i64 * 60_000;
            events.push(IdEvent {
                source: 0,
                id,
                original_ts: t,
            });
            // Half the ids are reused by unrelated events at random-ish distances.
            let noise = if i % 2 == 0 { 0 } else { ((i as i64 * 7919) % 1_800_000) - 900_000 };
            events.push(IdEvent {
                source: 1,
                id,
                original_ts: t - 5_000 + noise,
            });
        }
        let r = suggest(&events, &HashMap::new(), &SuggestRequest::default());
        let s = &r.suggestions[0];
        assert_eq!(s.pairs, 40);
        assert!((18..=22).contains(&s.agreeing), "{}", s.agreeing);
        assert!(s.confidence > 0.4 && s.confidence < 0.6, "{}", s.confidence);
        assert!((s.suggested_ms - 5_000).abs() < 900_000);
    }

    #[test]
    fn implausible_and_insufficient_pairs() {
        // Deltas beyond the bound are ignored; too few pairs give no suggestion.
        let events = vec![
            ev(0, "a", 0),
            ev(1, "a", 10_000_000),
            ev(0, "b", 0),
            ev(1, "b", 100),
            ev(0, "c", 0),
            ev(2, "c", 50),
            ev(0, "", 0),
            ev(3, "", 0),
            ev(4, "lonely", 0),
        ];
        let r = suggest(&events, &HashMap::new(), &SuggestRequest::default());
        assert_eq!(r.reference, Some(0));
        assert!(r.suggestions.is_empty());
        assert_eq!(r.insufficient, [1, 2]);
        // Pairs that exist but disagree: nothing suggested either.
        let events = vec![
            ev(0, "a", 0),
            ev(1, "a", 100_000),
            ev(0, "b", 0),
            ev(1, "b", 200_000),
            ev(0, "c", 0),
            ev(1, "c", 300_000),
        ];
        let r = suggest(&events, &HashMap::new(), &SuggestRequest::default());
        assert_eq!(r.insufficient, [1]);
        // Nothing shared at all.
        assert_eq!(suggest(&[], &HashMap::new(), &SuggestRequest::default()), SuggestResult::default());
    }

    #[test]
    fn request_defaults_and_fields() {
        let r: SuggestRequest = serde_json::from_str("{}").unwrap();
        assert_eq!(
            (r.max_delta_ms, r.tolerance_ms, r.reference),
            (DEFAULT_MAX_DELTA_MS, DEFAULT_TOLERANCE_MS, None)
        );
        assert_eq!(r.id_fields().len(), 3);
        let r: SuggestRequest = serde_json::from_str(r#"{"fields": ["X", " "], "reference": 2}"#).unwrap();
        assert_eq!(r.id_fields(), ["X"]);
    }

    #[test]
    fn id_cap_bounds_memory() {
        let names: Vec<String> = (0..MAX_IDS + 5).map(|i| i.to_string()).collect();
        let events: Vec<IdEvent> = names
            .iter()
            .map(|id| IdEvent {
                source: 0,
                id,
                original_ts: 0,
            })
            .collect();
        assert_eq!(suggest(&events, &HashMap::new(), &SuggestRequest::default()).reference, None);
    }
}
