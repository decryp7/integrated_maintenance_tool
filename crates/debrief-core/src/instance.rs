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
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Where the identity fields of the instance key apply.
///
/// An identity field an importer or a parser declares (e.g. `Category` of the Microsoft.Extensions.Logging console
/// parser, `Logger` of the Hadoop/Spark one) tells apart the processes of the sources that importer or parser reads,
/// not of every file: another product may write a field of the same name with another meaning (an EVTX `Category`).
/// Such a field is part of an event's instance key only when the event's source declares it. Fields of the profile's
/// `[instance] candidates` (and an explicit `[instance] key`) apply everywhere.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyScopes(BTreeMap<String, BTreeSet<String>>);

impl KeyScopes {
    /// The scopes of the identity fields `sources` (name, declared identity fields) declare: a field not among the
    /// profile's candidates that some source does not declare applies only to the sources that do.
    pub fn new<'a>(cfg: &InstanceCfg, sources: impl IntoIterator<Item = (&'a str, &'a [String])>) -> KeyScopes {
        let sources: Vec<(&str, &[String])> = sources.into_iter().collect();
        let mut scopes: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        if !cfg.key.is_empty() {
            return KeyScopes(scopes);
        }
        for (name, identity) in &sources {
            for field in identity.iter() {
                if !cfg.candidates.iter().any(|c| c.eq_ignore_ascii_case(field)) {
                    scopes.entry(field.to_ascii_lowercase()).or_default().insert((*name).to_string());
                }
            }
        }
        // A field every source declares applies everywhere anyway.
        scopes.retain(|_, names| names.len() < sources.len());
        KeyScopes(scopes)
    }

    /// Whether `field` is part of the instance key of the events of source `source`.
    pub fn applies(&self, field: &str, source: &str) -> bool {
        self.sources_of(field).is_none_or(|names| names.contains(source))
    }

    /// The sources a scoped field applies to; `None` when it applies everywhere.
    pub fn sources_of(&self, field: &str) -> Option<&BTreeSet<String>> {
        if self.0.is_empty() {
            return None;
        }
        self.0.get(&field.to_ascii_lowercase())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// [`FieldPresence`] that counts a scoped identity field only in the sources it applies to.
pub struct ScopedPresence<'a> {
    pub inner: &'a dyn FieldPresence,
    pub scopes: &'a KeyScopes,
    /// The name of each source (by index, as given to [`instance_key`]).
    pub names: &'a [String],
}

impl FieldPresence for ScopedPresence<'_> {
    fn count_in(&self, field: &str, source: usize) -> usize {
        let applies = self.names.get(source).is_none_or(|name| self.scopes.applies(field, name));
        if applies {
            self.inner.count_in(field, source)
        } else {
            0
        }
    }
}

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
    fn identity_fields_apply_only_to_the_sources_that_declare_them() {
        let c = cfg(&["ProcessId"]);
        let (spark, web, evtx) = (
            vec!["Logger".to_string()],
            vec!["ProcessId".to_string()],
            vec!["logger".to_string()],
        );
        let scopes = KeyScopes::new(
            &c,
            [
                ("spark.log", spark.as_slice()),
                ("web.log", web.as_slice()),
                ("app.evtx", evtx.as_slice()),
            ],
        );
        assert!(
            scopes.applies("Logger", "spark.log") && scopes.applies("LOGGER", "app.evtx"),
            "any case"
        );
        assert!(!scopes.applies("Logger", "web.log"));
        assert!(scopes.applies("ProcessId", "spark.log"), "a profile candidate applies everywhere");
        assert!(scopes.applies("Thread", "web.log"), "undeclared fields apply everywhere");
        assert_eq!(scopes.sources_of("logger").map(|s| s.len()), Some(2));
        // Declared by every source, or an explicit key: nothing is scoped.
        assert!(KeyScopes::new(&c, [("a", spark.as_slice()), ("b", spark.as_slice())]).is_empty());
        let explicit = InstanceCfg {
            key: vec!["Logger".into()],
            candidates: Vec::new(),
        };
        assert!(KeyScopes::new(&explicit, [("a", spark.as_slice()), ("b", web.as_slice())]).is_empty());
        // Presence: a scoped field does not count in the other sources.
        let counts = Counts(HashMap::from([(("Logger", 0), 10), (("Logger", 1), 10)]));
        let names = ["spark.log".to_string(), "web.log".to_string()];
        let scoped = ScopedPresence {
            inner: &counts,
            scopes: &scopes,
            names: &names,
        };
        assert_eq!((scoped.count_in("Logger", 0), scoped.count_in("Logger", 1)), (10, 0));
        assert_eq!(scoped.count_in("Logger", 7), 0, "a source without a name falls back to the counts");
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
