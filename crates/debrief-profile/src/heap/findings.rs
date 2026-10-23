//! Automatic findings over a heap snapshot (and its predecessor): what holds
//! the memory, what is unusual for a managed heap, and what grew. Each
//! finding names the type and the memory view that shows the evidence.

use super::dominators::HeapAnalysis;
use super::model::HeapSnapshot;
use super::ops::Heap;
use crate::api::{FindingSeverity, HeapFinding, HeapFindingKind, HeapFindingView};
use crate::ops::common::pct;
use rustc_hash::FxHashMap;

/// Objects this large live on the .NET large object heap.
pub const LARGE_OBJECT_BYTES: u32 = 85_000;
/// Share of the reachable heap from which retention is worth a warning.
const RETAINED_WARN_PCT: f64 = 20.0;
const STATIC_PCT: f64 = 10.0;
const DELEGATE_PCT: f64 = 10.0;
const LARGE_PCT: f64 = 20.0;
const STRING_PCT: f64 = 25.0;
const FINALIZER_COUNT: usize = 1000;

fn mb(v: u64) -> String {
    format!("{:.1} MB", v as f64 / 1e6)
}

/// A synthetic root group (`[Stack roots]`, `[Static variables]`, ...).
fn is_group(name: &str) -> bool {
    name.starts_with('[') && name.ends_with(']')
}

fn is_delegate(name: &str) -> bool {
    let n = name.split('<').next().unwrap_or(name);
    n.ends_with("EventHandler") || n.ends_with("Delegate") || n.starts_with("System.Action") || n.starts_with("System.Func")
}

struct Out(Vec<HeapFinding>);

impl Out {
    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        kind: HeapFindingKind,
        severity: FindingSeverity,
        title: String,
        detail: String,
        value_bytes: Option<u64>,
        type_name: Option<String>,
        view: HeapFindingView,
    ) {
        self.0.push(HeapFinding {
            kind,
            severity,
            title,
            detail,
            value_bytes,
            type_name,
            view,
        });
    }
}

/// Findings for `h` (with `baseline`, the snapshot before it, for growth), most severe first.
pub fn findings(h: &Heap<'_>, baseline: Option<&HeapSnapshot>) -> Vec<HeapFinding> {
    let s = h.snapshot;
    let mut out = Out(Vec::new());
    if let (Some(g), Some(a)) = (&s.graph, h.analysis) {
        graph_findings(&mut out, s, g, a);
    } else {
        top_types_by_bytes(&mut out, s);
    }
    strings(&mut out, s);
    if let Some(b) = baseline {
        growth(&mut out, b, s);
    }
    let mut v = out.0;
    v.sort_by(|a, b| b.severity.cmp(&a.severity).then(b.value_bytes.cmp(&a.value_bytes)));
    v
}

fn top_types_by_bytes(out: &mut Out, s: &HeapSnapshot) {
    let total = s.total_bytes();
    if let Some(t) = s.types.iter().filter(|t| !is_group(&t.name)).max_by_key(|t| t.bytes) {
        out.push(
            HeapFindingKind::TopRetainers,
            FindingSeverity::Info,
            format!("{} takes {:.0}% of the heap", t.name, pct(t.bytes, total)),
            format!(
                "{} objects, {} (shallow size; this snapshot has no object graph for retained sizes).",
                t.count,
                mb(t.bytes)
            ),
            Some(t.bytes),
            Some(t.name.clone()),
            HeapFindingView::Types,
        );
    }
}

fn graph_findings(out: &mut Out, s: &HeapSnapshot, g: &super::ObjectGraph, a: &HeapAnalysis) {
    let n = g.len();
    let reachable = a.retained.get(n).copied().unwrap_or(0).max(1);
    // What holds the most memory (retained size per type).
    if let Some((t, &bytes)) = a
        .type_retained
        .iter()
        .enumerate()
        .filter(|(t, _)| !is_group(&s.types[*t].name))
        .max_by_key(|(_, b)| **b)
    {
        let share = pct(bytes, reachable);
        out.push(
            HeapFindingKind::TopRetainers,
            if share >= RETAINED_WARN_PCT {
                FindingSeverity::Warning
            } else {
                FindingSeverity::Info
            },
            format!("{} keeps {share:.0}% of the heap alive", s.types[t].name),
            format!(
                "{} objects retain {} (everything only reachable through them). Its paths to roots show who holds it.",
                s.types[t].count,
                mb(bytes)
            ),
            Some(bytes),
            Some(s.types[t].name.clone()),
            HeapFindingView::Dominators,
        );
    }
    // Root groups: statics and the finalizer queue.
    for o in 0..n as u32 {
        let name = s.types[g.type_of[o as usize] as usize].name.as_str();
        match name {
            "[Static variables]" => {
                let retained = a.retained[o as usize];
                let share = pct(retained, reachable);
                if share >= STATIC_PCT {
                    let mut fields: Vec<(u64, &str)> = g
                        .edges(o)
                        .map(|(to, l)| (a.retained[to as usize], g.label(l).unwrap_or("?")))
                        .collect();
                    fields.sort_by(|x, y| y.0.cmp(&x.0).then(x.1.cmp(y.1)));
                    let top: Vec<String> = fields.iter().take(3).map(|(b, f)| format!("{f} ({})", mb(*b))).collect();
                    out.push(
                        HeapFindingKind::StaticRetention,
                        FindingSeverity::Warning,
                        format!("Static fields keep {} alive ({share:.0}% of the heap)", mb(retained)),
                        format!(
                            "Mostly {}. Statics live as long as the process: caches without eviction and static collections are the usual leak.",
                            top.join(", ")
                        ),
                        Some(retained),
                        None,
                        HeapFindingView::Paths,
                    );
                }
            }
            "[Finalizer queue]" => {
                let count = g.edges(o).count();
                if count >= FINALIZER_COUNT {
                    out.push(
                        HeapFindingKind::Finalizable,
                        FindingSeverity::Warning,
                        format!("{count} objects are waiting for their finalizer"),
                        format!(
                            "They hold {} until the finalizer thread runs. Dispose them (using / Dispose()) and call GC.SuppressFinalize in Dispose; a blocked finalizer thread makes this grow without bound.",
                            mb(a.retained[o as usize])
                        ),
                        Some(a.retained[o as usize]),
                        None,
                        HeapFindingView::Paths,
                    );
                }
            }
            _ => {}
        }
    }
    // Large objects (the large object heap).
    let mut large: FxHashMap<u32, (u64, u64)> = FxHashMap::default();
    for o in 0..n {
        if g.size[o] >= LARGE_OBJECT_BYTES {
            let e = large.entry(g.type_of[o]).or_default();
            e.0 += 1;
            e.1 = e.1.saturating_add(u64::from(g.size[o]));
        }
    }
    let large_bytes: u64 = large.values().map(|v| v.1).sum();
    let total = s.total_bytes().max(1);
    if large_bytes > 0 {
        let share = pct(large_bytes, total);
        let Some((&t, &(count, bytes))) = large.iter().max_by(|x, y| x.1 .1.cmp(&y.1 .1).then(y.0.cmp(x.0))) else {
            return;
        };
        out.push(
            HeapFindingKind::LargeObjects,
            if share >= LARGE_PCT { FindingSeverity::Warning } else { FindingSeverity::Info },
            format!("{} in large objects ({share:.0}% of the heap)", mb(large_bytes)),
            format!(
                "Objects of 85,000 bytes or more live on the large object heap, which is collected only with gen 2 and is not compacted by default. Most are {} ({count} objects, {}): pool them (ArrayPool) or keep them smaller.",
                s.types[t as usize].name,
                mb(bytes)
            ),
            Some(large_bytes),
            Some(s.types[t as usize].name.clone()),
            HeapFindingView::Types,
        );
    }
    // Delegates and event handlers keeping their targets alive.
    let delegates: u64 = a
        .type_retained
        .iter()
        .enumerate()
        .filter(|(t, _)| is_delegate(&s.types[*t].name))
        .map(|(_, b)| *b)
        .sum();
    let share = pct(delegates, reachable);
    if share >= DELEGATE_PCT {
        out.push(
            HeapFindingKind::EventHandlers,
            FindingSeverity::Warning,
            format!("Delegates and event handlers keep {} alive ({share:.0}%)", mb(delegates)),
            "An event keeps every subscriber alive until it unsubscribes: remove handlers (-=) when the subscriber is done, or use weak events.".into(),
            Some(delegates),
            None,
            HeapFindingView::Dominators,
        );
    }
}

fn strings(out: &mut Out, s: &HeapSnapshot) {
    let total = s.total_bytes().max(1);
    if let Some(t) = s.types.iter().find(|t| t.name == "System.String" || t.name == "string") {
        let share = pct(t.bytes, total);
        if share >= STRING_PCT {
            out.push(
                HeapFindingKind::Strings,
                FindingSeverity::Info,
                format!("Strings take {share:.0}% of the heap ({} strings)", t.count),
                "A string-heavy heap usually holds duplicates: intern or cache repeated values, avoid building strings in loops, and keep spans instead of substrings. (Event-based snapshots do not record string contents, so duplicates are not counted here.)".into(),
                Some(t.bytes),
                Some(t.name.clone()),
                HeapFindingView::Types,
            );
        }
    }
}

fn growth(out: &mut Out, baseline: &HeapSnapshot, s: &HeapSnapshot) {
    let before: FxHashMap<&str, u64> = baseline.types.iter().map(|t| (t.name.as_str(), t.bytes)).collect();
    let mut grown: Vec<(i128, &str, u64)> = s
        .types
        .iter()
        .filter(|t| !is_group(&t.name))
        .map(|t| {
            (
                i128::from(t.bytes) - i128::from(*before.get(t.name.as_str()).unwrap_or(&0)),
                t.name.as_str(),
                t.bytes,
            )
        })
        .filter(|(d, _, _)| *d > 0)
        .collect();
    grown.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
    let delta = i128::from(s.total_bytes()) - i128::from(baseline.total_bytes());
    if let Some((d, name, _)) = grown.first() {
        let top: Vec<String> = grown.iter().take(3).map(|(d, n, _)| format!("{n} (+{})", mb(*d as u64))).collect();
        let warn = delta > 0 && pct(delta as u64, baseline.total_bytes().max(1)) >= 20.0;
        out.push(
            HeapFindingKind::Growth,
            if warn { FindingSeverity::Warning } else { FindingSeverity::Info },
            if delta >= 0 {
                format!("The heap grew by {} since {}", mb(delta as u64), baseline.name)
            } else {
                format!("The heap shrank by {} since {}, but some types grew", mb((-delta) as u64), baseline.name)
            },
            format!(
                "Grown most: {}. Steady growth across several snapshots is a leak: compare them and follow the paths to roots of what keeps growing.",
                top.join(", ")
            ),
            Some(*d as u64),
            Some((*name).to_string()),
            HeapFindingView::Diff,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heap::HeapBuilder;
    use crate::limits::Limits;

    fn snapshot(big: bool, statics: bool) -> HeapSnapshot {
        let mut b = HeapBuilder::new("t", Limits::default());
        let st = b.type_id("[Static variables]").unwrap();
        let fq = b.type_id("[Finalizer queue]").unwrap();
        let cache = b.type_id("App.Cache").unwrap();
        let s = b.type_id("System.String").unwrap();
        let arr = b.type_id("System.Byte[]").unwrap();
        let h = b.type_id("System.EventHandler").unwrap();
        // 0: statics → 2 (cache); 1: finalizer queue → 1000 strings.
        b.object(st, 0).unwrap();
        b.edge(2, Some("App.Cache.Instance")).unwrap();
        b.object(fq, 0).unwrap();
        for i in 0..1000 {
            b.edge(5 + i, None).unwrap();
        }
        b.object(cache, 32).unwrap();
        b.edge(3, Some("buffer")).unwrap();
        b.edge(4, Some("handler")).unwrap();
        b.object(arr, if big { 1_000_000 } else { 1000 }).unwrap();
        b.object(h, 64).unwrap();
        for _ in 0..1000 {
            b.object(s, 400).unwrap();
        }
        if statics {
            b.root(0);
        }
        b.root(1);
        b.finish().unwrap()
    }

    fn run(s: &HeapSnapshot, base: Option<&HeapSnapshot>) -> Vec<HeapFinding> {
        let a = HeapAnalysis::new(s.graph.as_ref().unwrap(), s.types.len());
        findings(
            &Heap {
                snapshot: s,
                analysis: Some(&a),
            },
            base,
        )
    }

    #[test]
    fn graph_findings_cover_retention_statics_finalizers_and_large_objects() {
        let s = snapshot(true, true);
        let f = run(&s, None);
        let kinds: Vec<HeapFindingKind> = f.iter().map(|f| f.kind).collect();
        assert!(kinds.contains(&HeapFindingKind::TopRetainers), "{kinds:?}");
        let st = f.iter().find(|f| f.kind == HeapFindingKind::StaticRetention).unwrap();
        assert!(st.detail.contains("App.Cache.Instance"), "{}", st.detail);
        assert!(f
            .iter()
            .any(|f| f.kind == HeapFindingKind::Finalizable && f.title.starts_with("1000 objects")));
        let large = f.iter().find(|f| f.kind == HeapFindingKind::LargeObjects).unwrap();
        assert_eq!(large.type_name.as_deref(), Some("System.Byte[]"));
        assert_eq!(large.severity, FindingSeverity::Warning);
        assert!(f.windows(2).all(|w| w[0].severity >= w[1].severity));
        let top = f.iter().find(|f| f.kind == HeapFindingKind::TopRetainers).unwrap();
        assert_eq!(top.type_name.as_deref(), Some("App.Cache"), "{}", top.title);
        // Without large objects or statics, no such findings; strings dominate then.
        let small = snapshot(false, false);
        let f = run(&small, None);
        assert!(!f
            .iter()
            .any(|f| matches!(f.kind, HeapFindingKind::LargeObjects | HeapFindingKind::StaticRetention)));
        assert!(f.iter().any(|f| f.kind == HeapFindingKind::Strings));
    }

    #[test]
    fn growth_against_a_baseline_and_type_only_snapshots() {
        let before = snapshot(false, true);
        let after = snapshot(true, true);
        let f = run(&after, Some(&before));
        let g = f.iter().find(|f| f.kind == HeapFindingKind::Growth).unwrap();
        assert!(g.detail.contains("System.Byte[]"), "{}", g.detail);
        assert_eq!(g.severity, FindingSeverity::Warning);
        let f = run(&before, Some(&after));
        assert!(
            f.iter().any(|f| f.kind == HeapFindingKind::Growth && f.title.contains("shrank"))
                || !f.iter().any(|f| f.kind == HeapFindingKind::Growth)
        );

        let mut b = HeapBuilder::new("stats", Limits::default());
        b.type_stat("System.String", 10, 1000).unwrap();
        b.type_stat("App.X", 1, 10).unwrap();
        let s = b.finish().unwrap();
        let f = findings(
            &Heap {
                snapshot: &s,
                analysis: None,
            },
            None,
        );
        assert!(f
            .iter()
            .any(|f| f.kind == HeapFindingKind::TopRetainers && f.view == HeapFindingView::Types));
        assert!(f.iter().any(|f| f.kind == HeapFindingKind::Strings));
        assert!(is_delegate("System.Action<int>") && is_delegate("System.EventHandler") && !is_delegate("App.Cache"));
    }
}
