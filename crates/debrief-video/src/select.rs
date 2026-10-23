//! Which frames of a video are kept (ADR 0033 §3). Pure: the candidates are the frames FFmpeg's `mpdecimate` left
//! (near-duplicates such as a blinking cursor or a ticking clock are gone) with their scene scores; the selection keeps
//! the first and the last always, then the changes (score at least the threshold) or one frame per interval, within
//! the cap: the threshold rises while there are too many, and what is still too many is spread evenly, with a note.

use serde::{Deserialize, Serialize};

/// Default number of frames kept per video.
pub const DEFAULT_CAP: usize = 30;
/// Most frames a video may keep.
pub const MAX_CAP: usize = 120;
/// Default scene-change threshold (FFmpeg's scene score, 0–1).
pub const DEFAULT_THRESHOLD: f32 = 0.10;
/// The adaptive threshold stops here.
pub const MAX_THRESHOLD: f32 = 0.60;
/// Default interval of the interval mode.
pub const DEFAULT_INTERVAL_MS: u64 = 10_000;
/// Shortest interval.
pub const MIN_INTERVAL_MS: u64 = 500;

/// A frame that survived `mpdecimate`: its time in the video and how much it differs from the one before.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate {
    pub ms: u64,
    pub score: f32,
}

/// How frames are chosen.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "mode", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SelectMode {
    /// Every change at least `threshold` (adaptive, see the module docs).
    Changes { threshold: f32 },
    /// One frame per interval.
    Interval {
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        every_ms: u64,
    },
}

impl Default for SelectMode {
    fn default() -> Self {
        SelectMode::Changes {
            threshold: DEFAULT_THRESHOLD,
        }
    }
}

/// The selection settings of one video.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", default)]
pub struct SelectOptions {
    pub mode: SelectMode,
    pub cap: usize,
}

impl Default for SelectOptions {
    fn default() -> Self {
        SelectOptions {
            mode: SelectMode::default(),
            cap: DEFAULT_CAP,
        }
    }
}

impl SelectOptions {
    /// The same options within their bounds (they come from the webview): cap 2–120, threshold 0.01–0.60,
    /// interval at least 0.5 s.
    pub fn bounded(self) -> SelectOptions {
        let mode = match self.mode {
            SelectMode::Changes { threshold } => SelectMode::Changes {
                threshold: if threshold.is_finite() {
                    threshold.clamp(0.01, MAX_THRESHOLD)
                } else {
                    DEFAULT_THRESHOLD
                },
            },
            SelectMode::Interval { every_ms } => SelectMode::Interval {
                every_ms: every_ms.max(MIN_INTERVAL_MS),
            },
        };
        SelectOptions {
            mode,
            cap: self.cap.clamp(2, MAX_CAP),
        }
    }
}

/// What the user should know about a selection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SelectNote {
    /// More changes than the cap: the threshold was raised.
    ThresholdRaised { from: f32, to: f32 },
    /// Still more than the cap at the highest threshold (or in interval mode): the kept frames are spread evenly;
    /// `left_out` frames that would have been kept are not.
    SpreadEvenly { left_out: usize },
    /// Nothing changed on screen after the first frame.
    NoChanges,
}

/// The kept frames (ascending times) and the notes.
#[derive(Debug, Clone, PartialEq)]
pub struct Selection {
    pub kept: Vec<Candidate>,
    /// The threshold that was used (changes mode).
    pub threshold: Option<f32>,
    pub notes: Vec<SelectNote>,
}

/// The candidates sorted by time, one per time, scores within 0–1 (FFmpeg's output is parsed, not trusted).
pub fn normalize(mut c: Vec<Candidate>) -> Vec<Candidate> {
    for x in &mut c {
        x.score = if x.score.is_finite() { x.score.clamp(0.0, 1.0) } else { 0.0 };
    }
    c.sort_by_key(|x| x.ms);
    c.dedup_by_key(|x| x.ms);
    c
}

/// Picks `n` of `inner` spread evenly over `[from, to]`: for each of `n` evenly spaced points the nearest one not
/// taken yet. Returns them in time order.
fn spread(inner: &[Candidate], n: usize, from: u64, to: u64) -> Vec<Candidate> {
    if n == 0 || inner.is_empty() {
        return Vec::new();
    }
    if inner.len() <= n {
        return inner.to_vec();
    }
    let mut taken = vec![false; inner.len()];
    let span = to.saturating_sub(from) as f64;
    for k in 1..=n {
        let target = from as f64 + span * k as f64 / (n + 1) as f64;
        let best = (0..inner.len()).filter(|i| !taken[*i]).min_by(|a, b| {
            let da = (inner[*a].ms as f64 - target).abs();
            let db = (inner[*b].ms as f64 - target).abs();
            da.total_cmp(&db).then(a.cmp(b))
        });
        if let Some(i) = best {
            taken[i] = true;
        }
    }
    inner.iter().zip(taken).filter(|(_, t)| *t).map(|(c, _)| *c).collect()
}

/// The first and the last candidate plus `inner`, capped by spreading `inner` evenly.
fn with_ends(c: &[Candidate], inner: Vec<Candidate>, cap: usize, notes: &mut Vec<SelectNote>) -> Vec<Candidate> {
    let (first, last) = (c[0], c[c.len() - 1]);
    if c.len() == 1 {
        return vec![first];
    }
    let room = cap.saturating_sub(2);
    let inner = if inner.len() > room {
        notes.push(SelectNote::SpreadEvenly {
            left_out: inner.len() - room,
        });
        spread(&inner, room, first.ms, last.ms)
    } else {
        inner
    };
    let mut out = Vec::with_capacity(inner.len() + 2);
    out.push(first);
    out.extend(inner);
    out.push(last);
    out
}

/// Chooses the frames to keep from the candidates (any order; see [`normalize`]).
pub fn select(candidates: Vec<Candidate>, opts: SelectOptions) -> Selection {
    let opts = opts.bounded();
    let c = normalize(candidates);
    let mut notes = Vec::new();
    if c.is_empty() {
        return Selection {
            kept: Vec::new(),
            threshold: None,
            notes,
        };
    }
    let interior = if c.len() > 2 { &c[1..c.len() - 1] } else { &[][..] };
    match opts.mode {
        SelectMode::Changes { threshold } => {
            let above = |t: f32| interior.iter().filter(|x| x.score >= t).copied().collect::<Vec<_>>();
            let mut t = threshold;
            let mut inner = above(t);
            while inner.len() + 2 > opts.cap && t < MAX_THRESHOLD {
                t = (t * 1.25).max(t + 0.01).min(MAX_THRESHOLD);
                inner = above(t);
            }
            if t > threshold {
                notes.push(SelectNote::ThresholdRaised { from: threshold, to: t });
            }
            if inner.is_empty() && c.len() > 1 && interior.iter().all(|x| x.score < threshold) && c[c.len() - 1].score < threshold {
                notes.push(SelectNote::NoChanges);
            }
            let kept = with_ends(&c, inner, opts.cap, &mut notes);
            Selection {
                kept,
                threshold: Some(t),
                notes,
            }
        }
        SelectMode::Interval { every_ms } => {
            let last_ms = c[c.len() - 1].ms;
            let mut inner: Vec<Candidate> = Vec::new();
            let mut at = every_ms;
            while at < last_ms {
                let nearest = interior.iter().min_by_key(|x| (x.ms.abs_diff(at), x.ms)).copied();
                if let Some(n) = nearest {
                    if !inner.iter().any(|k| k.ms == n.ms) {
                        inner.push(n);
                    }
                }
                at = at.saturating_add(every_ms);
            }
            inner.sort_by_key(|x| x.ms);
            let kept = with_ends(&c, inner, opts.cap, &mut notes);
            Selection {
                kept,
                threshold: None,
                notes,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn c(ms: u64, score: f32) -> Candidate {
        Candidate { ms, score }
    }

    fn times(s: &Selection) -> Vec<u64> {
        s.kept.iter().map(|k| k.ms).collect()
    }

    fn changes(threshold: f32, cap: usize) -> SelectOptions {
        SelectOptions {
            mode: SelectMode::Changes { threshold },
            cap,
        }
    }

    #[test]
    fn keeps_the_first_the_last_and_the_changes() {
        let s = select(
            vec![c(4000, 0.02), c(0, 0.0), c(2000, 0.4), c(1000, 0.05), c(3000, 0.12)],
            SelectOptions::default(),
        );
        assert_eq!(times(&s), [0, 2000, 3000, 4000]);
        assert_eq!(s.threshold, Some(DEFAULT_THRESHOLD));
        assert!(s.notes.is_empty());
    }

    #[test]
    fn empty_one_and_two_candidates() {
        assert!(select(vec![], SelectOptions::default()).kept.is_empty());
        assert_eq!(times(&select(vec![c(5, 0.0)], SelectOptions::default())), [5]);
        let two = select(vec![c(0, 0.0), c(9, 0.0)], SelectOptions::default());
        assert_eq!(times(&two), [0, 9]);
        assert_eq!(two.notes, [SelectNote::NoChanges]);
    }

    #[test]
    fn a_static_screen_says_nothing_changed() {
        let s = select((0..50).map(|i| c(i * 100, 0.01)).collect(), SelectOptions::default());
        assert_eq!(times(&s), [0, 4900]);
        assert_eq!(s.notes, [SelectNote::NoChanges]);
    }

    #[test]
    fn too_many_changes_raise_the_threshold() {
        // 10 strong changes and 40 weak ones: a cap of 12 keeps exactly the strong ones.
        let mut v: Vec<Candidate> = (0..40).map(|i| c(1000 + i * 10, 0.15)).collect();
        v.extend((0..10).map(|i| c(5000 + i * 1000, 0.5)));
        v.push(c(0, 0.0));
        v.push(c(20_000, 0.0));
        let s = select(v, changes(0.10, 12));
        assert_eq!(s.kept.len(), 12);
        assert!(s.kept[1..11].iter().all(|k| k.score == 0.5));
        let to = s.threshold.unwrap();
        assert!(to > 0.15 && to <= 0.5, "{to}");
        assert_eq!(s.notes, [SelectNote::ThresholdRaised { from: 0.10, to }]);
    }

    #[test]
    fn still_too_many_at_the_highest_threshold_are_spread_evenly() {
        let v: Vec<Candidate> = (0..=100).map(|i| c(i * 1000, 0.9)).collect();
        let s = select(v, changes(0.10, 6));
        assert_eq!(times(&s), [0, 20_000, 40_000, 60_000, 80_000, 100_000]);
        assert_eq!(s.threshold, Some(MAX_THRESHOLD));
        assert!(matches!(s.notes[0], SelectNote::ThresholdRaised { .. }));
        assert_eq!(s.notes[1], SelectNote::SpreadEvenly { left_out: 95 });
    }

    #[test]
    fn interval_mode_takes_the_nearest_candidate_once() {
        // The screen did not change between 3 s and 30 s: the points 10 s, 20 s and 30 s find the 3 s and 30 s frames,
        // the 30 s frame only once.
        let v = vec![c(0, 0.0), c(2000, 0.3), c(3000, 0.3), c(30_000, 0.3), c(31_000, 0.0)];
        let s = select(
            v,
            SelectOptions {
                mode: SelectMode::Interval { every_ms: 10_000 },
                cap: 30,
            },
        );
        assert_eq!(times(&s), [0, 3000, 30_000, 31_000]);
        assert_eq!(s.threshold, None);
        let many: Vec<Candidate> = (0..=600).map(|i| c(i * 1000, 0.0)).collect();
        let s = select(
            many,
            SelectOptions {
                mode: SelectMode::Interval { every_ms: 1000 },
                cap: 10,
            },
        );
        assert_eq!(s.kept.len(), 10);
        assert!(matches!(s.notes[..], [SelectNote::SpreadEvenly { .. }]));
    }

    #[test]
    fn options_from_the_webview_are_bounded() {
        let o = SelectOptions {
            mode: SelectMode::Changes { threshold: f32::NAN },
            cap: 100_000,
        }
        .bounded();
        assert_eq!(o, changes(DEFAULT_THRESHOLD, MAX_CAP));
        assert_eq!(changes(5.0, 0).bounded(), changes(MAX_THRESHOLD, 2));
        assert_eq!(
            SelectOptions {
                mode: SelectMode::Interval { every_ms: 1 },
                cap: 30
            }
            .bounded()
            .mode,
            SelectMode::Interval { every_ms: MIN_INTERVAL_MS }
        );
        let n = normalize(vec![c(2, f32::INFINITY), c(1, -3.0), c(2, 0.5)]);
        assert_eq!(n, [c(1, 0.0), c(2, 0.0)]);
    }

    proptest! {
        #[test]
        fn never_more_than_the_cap_always_the_ends_in_order(
            raw in proptest::collection::vec((0u64..1_000_000, any::<f32>()), 0..400),
            cap in 0usize..200,
            threshold in any::<f32>(),
            interval in any::<bool>(),
            every in 0u64..100_000,
        ) {
            let v: Vec<Candidate> = raw.iter().map(|(ms, s)| c(*ms, *s)).collect();
            let mode = if interval { SelectMode::Interval { every_ms: every } } else { SelectMode::Changes { threshold } };
            let opts = SelectOptions { mode, cap };
            let n = normalize(v.clone());
            let s = select(v, opts);
            prop_assert!(s.kept.len() <= opts.bounded().cap);
            prop_assert!(s.kept.windows(2).all(|w| w[0].ms < w[1].ms));
            if let (Some(f), Some(l)) = (n.first(), n.last()) {
                prop_assert_eq!(s.kept.first().map(|k| k.ms), Some(f.ms));
                prop_assert_eq!(s.kept.last().map(|k| k.ms), Some(l.ms));
            } else {
                prop_assert!(s.kept.is_empty());
            }
        }
    }
}
