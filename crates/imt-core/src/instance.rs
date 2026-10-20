//! The automatic instance key (ADR 0020): which fields tell the processes of the opened logs apart, so the
//! timeline gets one lane per process.
//!
//! Candidates are the profile's `[instance] candidates` followed by the identity fields the sources'
//! importers declare (syslog `Host`/`App`/`ProcId`, EVTX `Computer`/`ProcessId`...); they match fields
//! ignoring case. The key is chosen in two steps:
//!
//! 1. the candidates present on at least half of **all** events (the usual case: one product, or products
//!    sharing field names);
//! 2. then, for every source none of those fields covers (a field covers a source when it has two or more
//!    values and is present on at least half of the source's events), the candidates present on at least
//!    half of **that source's** events, so a product that writes other fields, or only a small part of the
//!    events, still gets a lane per process instead of one for the whole source (routes make products
//!    sources of their own).
//!
//! Only fields with two or more values over all events are kept. Without any, a single source is keyed by
//! its first present candidate, and several sources by `@source`. Fields of step 2 come last, and
//! [`crate::query::group_key`] leaves out missing values at either end (putting the source name first when
//! the leading ones are missing), so each product's lanes read like its own key.

use crate::api::FieldInfo;
use crate::profile::InstanceCfg;

/// How often one field occurs in one source.
pub trait FieldPresence {
    /// Events of source `source` (an index into the sources given to [`instance_key`]) that carry `field`.
    fn count_in(&self, field: &str, source: usize) -> usize;
}

/// The profile's candidates followed by `identity` (importer-declared fields), without duplicates (ignoring
/// case).
pub fn candidates(cfg: &InstanceCfg, identity: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in cfg.candidates.iter().chain(identity) {
        if !out.iter().any(|o| o.eq_ignore_ascii_case(c)) {
            out.push(c.clone());
        }
    }
    out
}

/// The field a candidate names: the one with exactly that name, else the most frequent one whose name
/// differs only in case.
pub fn resolve<'f>(candidate: &str, fields: &'f [FieldInfo]) -> Option<&'f FieldInfo> {
    fields.iter().find(|f| f.name == candidate).or_else(|| {
        fields
            .iter()
            .filter(|f| f.name.eq_ignore_ascii_case(candidate))
            .max_by(|a, b| a.count.cmp(&b.count).then_with(|| b.name.cmp(&a.name)))
    })
}

/// The instance key: the profile's `[instance] key` when set, else chosen from the candidates (see the
/// module documentation). `fields` are the statistics over all events, `sources` the event count of each
/// source, `presence` how often a field occurs in each of them.
pub fn instance_key(
    cfg: &InstanceCfg,
    identity: &[String],
    fields: &[FieldInfo],
    sources: &[usize],
    presence: &dyn FieldPresence,
) -> Vec<String> {
    pick(cfg, identity, fields, sources, presence, sources.len())
}

/// [`instance_key`] with the number of sources the fallback looks at given separately.
fn pick(
    cfg: &InstanceCfg,
    identity: &[String],
    fields: &[FieldInfo],
    sources: &[usize],
    presence: &dyn FieldPresence,
    source_count: usize,
) -> Vec<String> {
    if !cfg.key.is_empty() {
        return cfg.key.clone();
    }
    let candidates = candidates(cfg, identity);
    let total = sources.iter().sum::<usize>().max(1);
    let mut present: Vec<&FieldInfo> = Vec::new();
    fn add<'f>(f: &'f FieldInfo, present: &mut Vec<&'f FieldInfo>) {
        if !present.iter().any(|p| p.name == f.name) {
            present.push(f);
        }
    }
    // 1. On at least half of all events.
    for c in &candidates {
        if let Some(f) = resolve(c, fields).filter(|f| f.count * 2 >= total) {
            add(f, &mut present);
        }
    }
    // 2. Sources the fields so far do not tell apart: their own candidates (any case variant), listed in
    // candidate order whatever the order of the sources.
    let covered: Vec<&FieldInfo> = present.clone();
    let mut extra: Vec<&str> = Vec::new();
    for (i, &events) in sources.iter().enumerate() {
        let dominant = |f: &FieldInfo| events > 0 && presence.count_in(&f.name, i) * 2 >= events;
        if events == 0 || covered.iter().any(|f| f.distinct > 1 && dominant(f)) {
            continue;
        }
        for f in fields
            .iter()
            .filter(|f| candidates.iter().any(|c| f.name.eq_ignore_ascii_case(c)) && dominant(f))
        {
            extra.push(&f.name);
        }
    }
    for c in &candidates {
        for f in fields
            .iter()
            .filter(|f| f.name.eq_ignore_ascii_case(c) && extra.contains(&f.name.as_str()))
        {
            add(f, &mut present);
        }
    }
    // Constant fields (same value everywhere) do not tell instances apart.
    let varying: Vec<String> = present.iter().filter(|f| f.distinct > 1).map(|f| f.name.clone()).collect();
    if !varying.is_empty() {
        return varying;
    }
    if source_count <= 1 {
        if let Some(f) = present.first() {
            return vec![f.name.clone()];
        }
    }
    vec!["@source".into()]
}

/// Presence when only totals over all events are known: every source is treated as one.
struct Totals<'f>(&'f [FieldInfo]);

impl FieldPresence for Totals<'_> {
    fn count_in(&self, field: &str, _source: usize) -> usize {
        self.0.iter().find(|f| f.name == field).map_or(0, |f| f.count)
    }
}

/// [`instance_key`] from totals only (stores that keep no per-source field counts): a candidate counts when
/// present on at least half of all `total` events; `sources` only decides the fallback.
pub fn instance_key_from_totals(cfg: &InstanceCfg, fields: &[FieldInfo], total: usize, sources: usize) -> Vec<String> {
    pick(cfg, &[], fields, &[total.max(1)], &Totals(fields), sources)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn info(name: &str, count: usize, distinct: usize) -> FieldInfo {
        FieldInfo {
            name: name.into(),
            count,
            distinct,
            numeric: false,
            derived: false,
        }
    }

    fn cfg(candidates: &[&str]) -> InstanceCfg {
        InstanceCfg {
            key: Vec::new(),
            candidates: candidates.iter().map(|c| c.to_string()).collect(),
        }
    }

    struct Counts(HashMap<(&'static str, usize), usize>);

    impl FieldPresence for Counts {
        fn count_in(&self, field: &str, source: usize) -> usize {
            self.0.iter().find(|((f, s), _)| *f == field && *s == source).map_or(0, |(_, n)| *n)
        }
    }

    #[test]
    fn a_field_one_small_product_writes_still_counts() {
        // Source 0: 1,000 events of product A with ProcessId; source 1: 10 events of B with ProcId.
        let fields = [info("ProcessId", 1000, 3), info("ProcId", 10, 2)];
        let counts = Counts(HashMap::from([(("ProcessId", 0), 1000), (("ProcId", 1), 10)]));
        let c = cfg(&["ProcessId"]);
        assert_eq!(
            instance_key(&c, &["ProcId".into()], &fields, &[1000, 10], &counts),
            ["ProcessId", "ProcId"]
        );
        // Measured over all events (the old rule), ProcId would be dropped.
        assert_eq!(
            instance_key_from_totals(&cfg(&["ProcessId", "ProcId"]), &fields, 1010, 2),
            ["ProcessId"]
        );
    }

    #[test]
    fn the_key_does_not_depend_on_the_order_of_the_sources() {
        // Two sources the common field does not cover, each with its own variant of a process id.
        let fields = [info("Application", 100, 2), info("Pid", 10, 3), info("pid", 4, 2)];
        let forward = Counts(HashMap::from([(("Application", 0), 100), (("Pid", 1), 10), (("pid", 2), 4)]));
        let backward = Counts(HashMap::from([(("Application", 0), 100), (("pid", 1), 4), (("Pid", 2), 10)]));
        let c = cfg(&["Application", "Pid"]);
        let a = instance_key(&c, &[], &fields, &[100, 10, 4], &forward);
        let b = instance_key(&c, &[], &fields, &[100, 4, 10], &backward);
        assert_eq!(a, ["Application", "Pid", "pid"]);
        assert_eq!(a, b);
    }

    #[test]
    fn candidates_match_ignoring_case_and_dedupe() {
        let c = cfg(&["ProcessId", "Pid"]);
        assert_eq!(candidates(&c, &["processid".into(), "Host".into()]), ["ProcessId", "Pid", "Host"]);
        let fields = [info("pid", 5, 2), info("PID", 9, 3), info("Pid2", 9, 3)];
        assert_eq!(resolve("Pid", &fields).unwrap().name, "PID", "the most frequent");
        assert_eq!(resolve("pid", &fields).unwrap().name, "pid", "exact first");
        assert!(resolve("Host", &fields).is_none());
        let counts = Counts(HashMap::from([(("PID", 0), 9)]));
        assert_eq!(instance_key(&cfg(&["Pid", "pid"]), &[], &fields[1..2], &[9], &counts), ["PID"]);
    }

    #[test]
    fn fallbacks_and_explicit_keys() {
        let fields = [info("MachineName", 10, 1)];
        let counts = Counts(HashMap::from([(("MachineName", 0), 10), (("MachineName", 1), 10)]));
        let c = cfg(&["MachineName"]);
        assert_eq!(instance_key(&c, &[], &fields, &[10], &counts), ["MachineName"], "one source");
        assert_eq!(
            instance_key(&c, &[], &fields, &[10, 10], &counts),
            ["@source"],
            "constant across sources"
        );
        assert_eq!(instance_key(&cfg(&["Nope"]), &[], &fields, &[10], &counts), ["@source"]);
        let explicit = InstanceCfg {
            key: vec!["Thread".into()],
            candidates: Vec::new(),
        };
        assert_eq!(instance_key(&explicit, &[], &fields, &[10], &counts), ["Thread"]);
        assert_eq!(instance_key_from_totals(&explicit, &fields, 10, 2), ["Thread"]);
        // Totals: present on fewer than half of all events does not count.
        assert_eq!(instance_key_from_totals(&c, &[info("MachineName", 4, 2)], 10, 1), ["@source"]);
        assert_eq!(instance_key_from_totals(&c, &fields, 10, 1), ["MachineName"]);
        assert_eq!(instance_key_from_totals(&c, &fields, 10, 3), ["@source"]);
        assert_eq!(
            crate::query::default_group_by(&c, &fields, 10, 1),
            ["MachineName"],
            "the totals-only entry point"
        );
    }
}
