//! When a page can be read: the load finished, the region the caller cares about has content, and the DOM has been
//! quiet for a while. Not "network idle". Pure: the caller polls and passes the observations.

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observation {
    /// `document.readyState === "complete"`.
    pub load_finished: bool,
    /// Items found in the region of interest (links in a results list, say); `None` when there is no such region.
    pub region_items: Option<usize>,
    /// How long no node was added or removed.
    pub quiet: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    pub quiet: Duration,
    /// An empty region is accepted after this much quiet (a page with no results).
    pub quiet_empty: Duration,
}

impl Default for Rule {
    fn default() -> Self {
        Rule {
            quiet: Duration::from_millis(500),
            quiet_empty: Duration::from_secs(2),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    Ready,
    Loading,
    Changing,
    RegionEmpty,
}

pub fn readiness(o: &Observation, rule: &Rule) -> Readiness {
    if !o.load_finished {
        return Readiness::Loading;
    }
    match o.region_items {
        Some(0) if o.quiet >= rule.quiet_empty => Readiness::Ready,
        Some(0) => Readiness::RegionEmpty,
        _ if o.quiet >= rule.quiet => Readiness::Ready,
        _ => Readiness::Changing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn o(load: bool, items: Option<usize>, ms: u64) -> Observation {
        Observation {
            load_finished: load,
            region_items: items,
            quiet: Duration::from_millis(ms),
        }
    }

    #[test]
    fn rule() {
        let r = Rule::default();
        assert_eq!(readiness(&o(false, Some(9), 9999), &r), Readiness::Loading);
        assert_eq!(readiness(&o(true, Some(9), 100), &r), Readiness::Changing);
        assert_eq!(readiness(&o(true, Some(9), 600), &r), Readiness::Ready);
        assert_eq!(readiness(&o(true, Some(0), 600), &r), Readiness::RegionEmpty);
        assert_eq!(readiness(&o(true, Some(0), 2000), &r), Readiness::Ready);
        assert_eq!(readiness(&o(true, None, 600), &r), Readiness::Ready);
    }
}
