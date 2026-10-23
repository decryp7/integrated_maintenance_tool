//! Per-pattern trend (ADR 0046): how often a template occurred over the time range of the analysed events, as
//! [`TREND_BUCKETS`] equal buckets shared by every pattern of one run so sparklines compare.
//!
//! The range is not known before the events are read, so the scale **grows with them**: when a timestamp falls
//! outside it, the bucket width doubles until the range fits and every pattern's buckets are folded onto the new scale
//! ([`Remap`]). Pure and allocation-free per event.

/// Buckets per trend as returned.
pub const TREND_BUCKETS: usize = 24;
/// Buckets counted while reading (four per returned bucket), folded onto a scale fitted to the events seen at the end,
/// so the doubling steps of the working scale never leave empty space at the right of a sparkline.
pub const FINE_BUCKETS: usize = 96;
/// Narrowest bucket: one second.
const MIN_WIDTH_MS: i64 = 1_000;
/// Widest bucket, so `width * TREND_BUCKETS` cannot overflow.
const MAX_WIDTH_MS: i64 = i64::MAX / (4 * FINE_BUCKETS as i64);

/// One pattern's counts per bucket.
pub type Buckets = [u32; FINE_BUCKETS];

/// The scale of a run: bucket `i` covers `[from + i * width, from + (i + 1) * width)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TrendScale {
    from: i64,
    width: i64,
    seen_from: i64,
    seen_to: i64,
}

/// How to fold buckets of the previous scale onto the new one after the scale grew.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Remap {
    old: TrendScale,
}

impl TrendScale {
    /// Whether any timed event was seen.
    pub fn is_set(&self) -> bool {
        self.width > 0
    }

    /// Start of the first bucket (epoch ms); only meaningful when [`is_set`](Self::is_set).
    pub fn from(&self) -> i64 {
        self.from
    }

    /// Width of a bucket in ms; 0 until the first timed event.
    pub fn width_ms(&self) -> i64 {
        self.width
    }

    fn end(&self) -> i64 {
        self.from.saturating_add(self.width.saturating_mul(FINE_BUCKETS as i64))
    }

    /// Start of the returned trend: the earliest event seen.
    pub fn display_from(&self) -> i64 {
        self.seen_from
    }

    /// Width of a returned bucket (ms): the events seen span exactly [`TREND_BUCKETS`] of them.
    pub fn display_width_ms(&self) -> i64 {
        let span = i128::from(self.seen_to) - i128::from(self.seen_from) + 1;
        let w = (span + TREND_BUCKETS as i128 - 1) / TREND_BUCKETS as i128;
        w.clamp(1, i128::from(i64::MAX)) as i64
    }

    /// The counts of `fine` on the returned scale.
    pub fn display(&self, fine: &Buckets) -> Vec<u32> {
        let mut out = vec![0u32; TREND_BUCKETS];
        if !self.is_set() {
            return out;
        }
        let dw = i128::from(self.display_width_ms());
        for (j, &c) in fine.iter().enumerate() {
            if c == 0 {
                continue;
            }
            let mid = i128::from(self.from) + i128::from(self.width) * j as i128 + i128::from(self.width / 2);
            let mid = mid.clamp(i128::from(self.seen_from), i128::from(self.seen_to));
            let k = ((mid - i128::from(self.seen_from)) / dw).clamp(0, TREND_BUCKETS as i128 - 1) as usize;
            out[k] = out[k].saturating_add(c);
        }
        out
    }

    /// Makes room for `ts`. `Some` when the scale changed (the caller folds its buckets with [`Remap::fold`]).
    pub fn fit(&mut self, ts: i64) -> Option<Remap> {
        if !self.is_set() {
            *self = TrendScale {
                from: ts,
                width: MIN_WIDTH_MS,
                seen_from: ts,
                seen_to: ts,
            };
            return None;
        }
        self.seen_from = self.seen_from.min(ts);
        self.seen_to = self.seen_to.max(ts);
        if ts >= self.from && ts < self.end() {
            return None;
        }
        let old = *self;
        let from = self.from.min(ts);
        let last = (self.end() - 1).max(ts);
        let span = (i128::from(last) - i128::from(from) + 1).max(1);
        let mut width = self.width;
        while i128::from(width) * (FINE_BUCKETS as i128) < span && width < MAX_WIDTH_MS {
            width = width.saturating_mul(2);
        }
        self.from = from;
        self.width = width.min(MAX_WIDTH_MS);
        Some(Remap { old })
    }

    /// The bucket of `ts` (clamped into range, so a caller that skipped [`fit`](Self::fit) cannot index out of bounds).
    pub fn index(&self, ts: i64) -> usize {
        if !self.is_set() {
            return 0;
        }
        let i = (i128::from(ts) - i128::from(self.from)) / i128::from(self.width);
        i.clamp(0, FINE_BUCKETS as i128 - 1) as usize
    }
}

impl Remap {
    /// `buckets` (counted on the previous scale) folded onto `scale`: each old bucket lands in the new bucket holding its
    /// midpoint.
    pub fn fold(&self, buckets: &Buckets, scale: &TrendScale) -> Buckets {
        let mut out = [0u32; FINE_BUCKETS];
        for (j, &c) in buckets.iter().enumerate() {
            if c == 0 {
                continue;
            }
            let mid = self
                .old
                .from
                .saturating_add(self.old.width.saturating_mul(j as i64))
                .saturating_add(self.old.width / 2);
            let k = scale.index(mid);
            out[k] = out[k].saturating_add(c);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill(scale: &mut TrendScale, buckets: &mut Buckets, ts: i64) {
        if let Some(r) = scale.fit(ts) {
            *buckets = r.fold(buckets, scale);
        }
        let i = scale.index(ts);
        buckets[i] += 1;
    }

    #[test]
    fn the_first_event_sets_a_one_second_scale() {
        let mut s = TrendScale::default();
        assert!(!s.is_set() && s.index(5) == 0);
        assert_eq!(
            s.display(&[0u32; FINE_BUCKETS]),
            vec![0u32; TREND_BUCKETS],
            "nothing to show before the first event"
        );
        assert!(s.fit(10_000).is_none());
        assert_eq!((s.from(), s.width_ms()), (10_000, 1_000));
        assert_eq!(s.index(10_999), 0);
        assert_eq!(s.index(13_000), 3);
        assert_eq!(s.index(i64::MAX), FINE_BUCKETS - 1);
        assert_eq!(s.index(i64::MIN), 0);
    }

    #[test]
    fn the_scale_grows_until_the_range_fits_and_keeps_every_count() {
        let mut s = TrendScale::default();
        let mut b = [0u32; FINE_BUCKETS];
        for ts in (0..24).map(|i| i * 1_000).chain([100_000, 500_000, 3_600_000]) {
            fill(&mut s, &mut b, ts);
        }
        assert_eq!(b.iter().sum::<u32>(), 27, "no event is lost when the scale grows");
        assert!(s.width_ms() * FINE_BUCKETS as i64 > 3_600_000);
        let shown = s.display(&b);
        assert_eq!(shown.iter().sum::<u32>(), 27);
        assert_eq!((s.display_from(), s.display_width_ms()), (0, 3_600_001 / 24 + 1));
        assert_eq!(shown[TREND_BUCKETS - 1], 1, "the last event is in the last bucket");
        assert!(shown[0] >= 24, "the early events share the first buckets: {shown:?}");
    }

    #[test]
    fn earlier_events_move_the_origin_back() {
        let mut s = TrendScale::default();
        let mut b = [0u32; FINE_BUCKETS];
        fill(&mut s, &mut b, 50_000);
        fill(&mut s, &mut b, 10_000);
        assert_eq!(s.from(), 10_000);
        assert_eq!(b.iter().sum::<u32>(), 2);
        assert_eq!(s.index(10_000), 0);
        assert!(s.index(50_000) > 0);
    }

    #[test]
    fn extreme_timestamps_do_not_overflow() {
        let mut s = TrendScale::default();
        let mut b = [0u32; FINE_BUCKETS];
        fill(&mut s, &mut b, i64::MIN / 2);
        fill(&mut s, &mut b, i64::MAX / 2);
        assert_eq!(b.iter().sum::<u32>(), 2);
        assert!(s.width_ms() <= MAX_WIDTH_MS);
    }
}
