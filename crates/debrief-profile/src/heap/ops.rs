//! Queries over heap snapshots: top types, snapshot diff, growth across a
//! series, the dominator tree, instances of a type and paths to GC roots.

use super::dominators::{edge_label, paths_to_roots, HeapAnalysis};
use super::model::HeapSnapshot;
use crate::api::*;
use crate::error::{ProfileError, Result};
use crate::limits::query::{MAX_PATHS, MAX_PATH_LEN, MAX_PATH_VISITS};
use crate::ops::common::{check_page, page, pct, Search};
use rustc_hash::FxHashMap;
use std::cmp::Ordering;

/// A snapshot with its (optional) graph analysis.
pub struct Heap<'a> {
    pub snapshot: &'a HeapSnapshot,
    pub analysis: Option<&'a HeapAnalysis>,
}

fn need_graph<'a>(h: &'a Heap<'_>) -> Result<(&'a super::ObjectGraph, &'a HeapAnalysis)> {
    match (&h.snapshot.graph, h.analysis) {
        (Some(g), Some(a)) => Ok((g, a)),
        _ => Err(ProfileError::NotAvailable(format!(
            "this {} snapshot has no object graph (only type statistics)",
            h.snapshot.source
        ))),
    }
}

pub fn types(h: &Heap<'_>, req: &HeapTypesRequest) -> Result<HeapTypesPage> {
    check_page(req.limit)?;
    let search = Search::new(req.search.as_deref(), false)?;
    let s = h.snapshot;
    let retained = |i: usize| h.analysis.map(|a| a.type_retained.get(i).copied().unwrap_or(0));
    if req.sort == HeapSort::Retained && h.analysis.is_none() {
        return Err(ProfileError::NotAvailable("retained sizes need the object graph".into()));
    }
    let mut idx: Vec<usize> = (0..s.types.len())
        .filter(|&i| s.types[i].count > 0 || s.types[i].bytes > 0)
        .filter(|&i| search.as_ref().is_none_or(|q| q.is_match(&s.types[i].name)))
        .collect();
    idx.sort_by(|&a, &b| {
        let (ta, tb) = (&s.types[a], &s.types[b]);
        match req.sort {
            HeapSort::Name => ta.name.to_lowercase().cmp(&tb.name.to_lowercase()),
            HeapSort::Count => tb.count.cmp(&ta.count),
            HeapSort::Bytes => tb.bytes.cmp(&ta.bytes),
            HeapSort::Retained => retained(b).cmp(&retained(a)),
        }
        .then_with(|| ta.name.cmp(&tb.name))
    });
    let total = s.total_bytes();
    let total_rows = idx.len();
    let rows = page(idx, req.offset, req.limit)
        .into_iter()
        .map(|i| HeapTypeRow {
            name: s.types[i].name.clone(),
            count: s.types[i].count,
            bytes: s.types[i].bytes,
            retained: retained(i),
            bytes_pct: pct(s.types[i].bytes, total),
        })
        .collect();
    Ok(HeapTypesPage {
        rows,
        total_rows,
        total_bytes: total,
        total_count: s.total_count(),
        has_graph: h.analysis.is_some(),
    })
}

fn signed(after: u64, before: u64) -> i64 {
    (i128::from(after) - i128::from(before)).clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

pub fn diff(baseline: &HeapSnapshot, target: &HeapSnapshot, req: &HeapDiffRequest) -> Result<HeapDiffPage> {
    check_page(req.limit)?;
    let search = Search::new(req.search.as_deref(), false)?;
    let before: FxHashMap<&str, (u64, u64)> = baseline.types.iter().map(|t| (t.name.as_str(), (t.count, t.bytes))).collect();
    let after: FxHashMap<&str, (u64, u64)> = target.types.iter().map(|t| (t.name.as_str(), (t.count, t.bytes))).collect();
    let mut names: Vec<&str> = before.keys().chain(after.keys()).copied().collect();
    names.sort_unstable();
    names.dedup();
    let mut rows: Vec<HeapDiffRow> = names
        .into_iter()
        .filter(|n| search.as_ref().is_none_or(|q| q.is_match(n)))
        .map(|n| {
            let (cb, bb) = before.get(n).copied().unwrap_or((0, 0));
            let (ca, ba) = after.get(n).copied().unwrap_or((0, 0));
            HeapDiffRow {
                name: n.to_string(),
                count_before: cb,
                count_after: ca,
                count_delta: signed(ca, cb),
                bytes_before: bb,
                bytes_after: ba,
                bytes_delta: signed(ba, bb),
                is_new: cb == 0 && bb == 0 && (ca > 0 || ba > 0),
                is_gone: (cb > 0 || bb > 0) && ca == 0 && ba == 0,
            }
        })
        .filter(|r| r.count_before + r.count_after + r.bytes_before + r.bytes_after > 0)
        .collect();
    rows.sort_by(|a, b| {
        match req.sort {
            HeapDiffSort::BytesDelta => b.bytes_delta.cmp(&a.bytes_delta),
            HeapDiffSort::CountDelta => b.count_delta.cmp(&a.count_delta),
            HeapDiffSort::Name => Ordering::Equal,
        }
        .then_with(|| a.name.cmp(&b.name))
    });
    let total_rows = rows.len();
    Ok(HeapDiffPage {
        rows: page(rows, req.offset, req.limit),
        total_rows,
        bytes_before: baseline.total_bytes(),
        bytes_after: target.total_bytes(),
        count_before: baseline.total_count(),
        count_after: target.total_count(),
    })
}

/// Least-squares slope of `ys` over 0, 1, 2, ...
fn slope(ys: &[u64]) -> f64 {
    let n = ys.len() as f64;
    if ys.len() < 2 {
        return 0.0;
    }
    let mx = (n - 1.0) / 2.0;
    let my = ys.iter().map(|&y| y as f64).sum::<f64>() / n;
    let (mut num, mut den) = (0.0, 0.0);
    for (i, &y) in ys.iter().enumerate() {
        let dx = i as f64 - mx;
        num += dx * (y as f64 - my);
        den += dx * dx;
    }
    num / den
}

/// Never shrinks and grows overall.
fn steady(series: &[u64]) -> bool {
    series.windows(2).all(|w| w[1] >= w[0]) && series.last() > series.first()
}

pub fn growth(snapshots: &[&HeapSnapshot], req: &HeapGrowthRequest) -> Result<HeapGrowthPage> {
    check_page(req.limit)?;
    if snapshots.len() < 3 {
        return Err(ProfileError::invalid("growth needs three or more snapshots, oldest first"));
    }
    let search = Search::new(req.search.as_deref(), false)?;
    let maps: Vec<FxHashMap<&str, (u64, u64)>> = snapshots
        .iter()
        .map(|s| s.types.iter().map(|t| (t.name.as_str(), (t.count, t.bytes))).collect())
        .collect();
    let mut names: Vec<&str> = maps.iter().flat_map(|m| m.keys().copied()).collect();
    names.sort_unstable();
    names.dedup();
    let mut rows: Vec<HeapGrowthRow> = names
        .into_iter()
        .filter(|n| search.as_ref().is_none_or(|q| q.is_match(n)))
        .map(|n| {
            let bytes: Vec<u64> = maps.iter().map(|m| m.get(n).map(|v| v.1).unwrap_or(0)).collect();
            let counts: Vec<u64> = maps.iter().map(|m| m.get(n).map(|v| v.0).unwrap_or(0)).collect();
            HeapGrowthRow {
                name: n.to_string(),
                slope_bytes: slope(&bytes),
                growth_bytes: signed(*bytes.last().unwrap_or(&0), bytes[0]),
                steady: steady(&bytes) || steady(&counts),
                bytes,
                counts,
            }
        })
        .collect();
    let steady_count = rows.iter().filter(|r| r.steady).count();
    if req.steady_only {
        rows.retain(|r| r.steady);
    }
    rows.sort_by(|a, b| {
        b.steady
            .cmp(&a.steady)
            .then_with(|| b.slope_bytes.partial_cmp(&a.slope_bytes).unwrap_or(Ordering::Equal))
            .then_with(|| a.name.cmp(&b.name))
    });
    let total_rows = rows.len();
    Ok(HeapGrowthPage {
        rows: page(rows, req.offset, req.limit),
        total_rows,
        steady_count,
    })
}

fn object_row(h: &Heap<'_>, g: &super::ObjectGraph, a: &HeapAnalysis, o: u32) -> HeapObjectRow {
    HeapObjectRow {
        object: o,
        type_name: h.snapshot.types[g.type_of[o as usize] as usize].name.clone(),
        shallow: u64::from(g.size[o as usize]),
        retained: a.retained[o as usize],
        child_count: a.dominated(o).len() as u64,
    }
}

pub fn dominators(h: &Heap<'_>, req: &HeapDominatorsRequest) -> Result<HeapObjectsPage> {
    check_page(req.limit)?;
    let (g, a) = need_graph(h)?;
    let n = g.len() as u32;
    let parent = match req.parent {
        None => n,
        Some(o) if o < n && a.is_reachable(o) => o,
        Some(o) => return Err(ProfileError::invalid(format!("object {o} is not in the dominator tree"))),
    };
    let kids = a.dominated(parent);
    let rows = kids
        .iter()
        .skip(req.offset)
        .take(req.limit)
        .map(|&o| object_row(h, g, a, o))
        .collect();
    Ok(HeapObjectsPage {
        rows,
        total_rows: kids.len(),
        total_bytes: a.retained[n as usize],
    })
}

pub fn instances(h: &Heap<'_>, req: &HeapInstancesRequest) -> Result<HeapObjectsPage> {
    check_page(req.limit)?;
    let (g, a) = need_graph(h)?;
    let t = h
        .snapshot
        .types
        .iter()
        .position(|t| t.name == req.type_name)
        .ok_or_else(|| ProfileError::invalid(format!("the snapshot has no type {}", req.type_name)))? as u32;
    let mut objs: Vec<u32> = (0..g.len() as u32).filter(|&o| g.type_of[o as usize] == t).collect();
    objs.sort_by(|&x, &y| a.retained[y as usize].cmp(&a.retained[x as usize]).then(x.cmp(&y)));
    let total_rows = objs.len();
    let total_bytes = objs.iter().map(|&o| u64::from(g.size[o as usize])).sum();
    Ok(HeapObjectsPage {
        rows: page(objs, req.offset, req.limit)
            .into_iter()
            .map(|o| object_row(h, g, a, o))
            .collect(),
        total_rows,
        total_bytes,
    })
}

pub fn paths(h: &Heap<'_>, req: &HeapPathsRequest) -> Result<HeapPaths> {
    let (g, a) = need_graph(h)?;
    if req.object as usize >= g.len() {
        return Err(ProfileError::invalid(format!("the snapshot has no object {}", req.object)));
    }
    let max_paths = req.max_paths.unwrap_or(5).clamp(1, MAX_PATHS);
    let max_depth = req.max_depth.unwrap_or(MAX_PATH_LEN).clamp(1, MAX_PATH_LEN);
    if !a.is_reachable(req.object) {
        return Ok(HeapPaths {
            paths: Vec::new(),
            unreachable: true,
            truncated: false,
        });
    }
    let (found, truncated) = paths_to_roots(g, a, req.object, max_paths, max_depth, MAX_PATH_VISITS);
    let type_name = |o: u32| h.snapshot.types[g.type_of[o as usize] as usize].name.clone();
    let paths = found
        .into_iter()
        .map(|p| {
            p.iter()
                .enumerate()
                .map(|(i, &o)| PathStep {
                    object: o,
                    type_name: type_name(o),
                    edge: if i == 0 { None } else { edge_label(g, p[i - 1], o) },
                })
                .collect()
        })
        .collect();
    Ok(HeapPaths {
        paths,
        unreachable: false,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heap::dominators::tests::graph;
    use crate::heap::HeapBuilder;
    use crate::limits::Limits;

    fn stats(types: &[(&str, u64, u64)]) -> HeapSnapshot {
        let mut b = HeapBuilder::new("stats", Limits::default());
        for (n, c, by) in types {
            b.type_stat(n, *c, *by).unwrap();
        }
        b.finish().unwrap()
    }

    #[test]
    fn types_sorting_search_and_retained() {
        let h = graph(
            &[("Cache", 10, &[1, 2]), ("Order", 50, &[]), ("Order", 50, &[]), ("Str", 5, &[])],
            &[0, 3],
        );
        let a = HeapAnalysis::new(h.graph.as_ref().unwrap(), h.types.len());
        let heap = Heap {
            snapshot: &h,
            analysis: Some(&a),
        };
        let mut req = HeapTypesRequest {
            snapshot_id: 0,
            sort: HeapSort::Retained,
            search: None,
            offset: 0,
            limit: 10,
        };
        let p = types(&heap, &req).unwrap();
        assert_eq!(p.rows[0].name, "Cache");
        assert_eq!(p.rows[0].retained, Some(110));
        assert_eq!(p.total_bytes, 115);
        assert!(p.has_graph);
        req.sort = HeapSort::Bytes;
        assert_eq!(types(&heap, &req).unwrap().rows[0].name, "Order");
        req.sort = HeapSort::Count;
        assert_eq!(types(&heap, &req).unwrap().rows[0].count, 2);
        req.sort = HeapSort::Name;
        req.search = Some("r".into());
        let names: Vec<String> = types(&heap, &req).unwrap().rows.into_iter().map(|r| r.name).collect();
        assert_eq!(names, ["Order", "Str"]);
        let flat = stats(&[("X", 1, 1)]);
        let heap = Heap {
            snapshot: &flat,
            analysis: None,
        };
        req.sort = HeapSort::Retained;
        assert!(types(&heap, &req).is_err());
        req.sort = HeapSort::Bytes;
        req.search = None;
        assert_eq!(types(&heap, &req).unwrap().rows[0].retained, None);
    }

    #[test]
    fn diff_flags_new_and_gone_types() {
        let a = stats(&[("Order", 10, 1000), ("Old", 1, 10), ("Same", 2, 20)]);
        let b = stats(&[("Order", 30, 3000), ("New", 5, 500), ("Same", 2, 20)]);
        let mut req = HeapDiffRequest {
            baseline_id: 0,
            target_id: 1,
            search: None,
            sort: HeapDiffSort::BytesDelta,
            offset: 0,
            limit: 10,
        };
        let d = diff(&a, &b, &req).unwrap();
        let r: Vec<(&str, i64, bool, bool)> = d
            .rows
            .iter()
            .map(|r| (r.name.as_str(), r.bytes_delta, r.is_new, r.is_gone))
            .collect();
        assert_eq!(
            r,
            [
                ("Order", 2000, false, false),
                ("New", 500, true, false),
                ("Same", 0, false, false),
                ("Old", -10, false, true)
            ]
        );
        assert_eq!((d.bytes_before, d.bytes_after), (1030, 3520));
        req.sort = HeapDiffSort::CountDelta;
        assert_eq!(diff(&a, &b, &req).unwrap().rows[0].name, "Order");
        req.sort = HeapDiffSort::Name;
        req.search = Some("o".into());
        let names: Vec<String> = diff(&a, &b, &req).unwrap().rows.into_iter().map(|r| r.name).collect();
        assert_eq!(names, ["Old", "Order"]);
        assert_eq!(signed(0, u64::MAX), i64::MIN);
    }

    #[test]
    fn growth_detects_steady_increase() {
        let s1 = stats(&[("Order", 10, 1000), ("Noise", 5, 50)]);
        let s2 = stats(&[("Order", 20, 2000), ("Noise", 3, 30), ("Late", 1, 5)]);
        let s3 = stats(&[("Order", 30, 3000), ("Noise", 6, 60), ("Late", 2, 10)]);
        let mut req = HeapGrowthRequest {
            snapshot_ids: vec![0, 1, 2],
            search: None,
            steady_only: false,
            offset: 0,
            limit: 10,
        };
        let g = growth(&[&s1, &s2, &s3], &req).unwrap();
        assert_eq!(g.steady_count, 2);
        assert_eq!(g.rows[0].name, "Order");
        assert_eq!(g.rows[0].slope_bytes, 1000.0);
        assert_eq!(g.rows[0].growth_bytes, 2000);
        assert_eq!(g.rows[1].name, "Late");
        assert_eq!(g.rows[1].bytes, [0, 5, 10]);
        assert!(!g.rows[2].steady);
        req.steady_only = true;
        assert_eq!(growth(&[&s1, &s2, &s3], &req).unwrap().total_rows, 2);
        assert!(growth(&[&s1, &s2], &req).is_err());
        assert_eq!(slope(&[5]), 0.0);
    }

    #[test]
    fn dominator_tree_instances_and_paths() {
        // root -> cache(0) -> list(1) -> order(2), order(3); root -> holder(4) -> order(3).
        let h = graph(
            &[
                ("Cache", 10, &[1]),
                ("List", 10, &[2, 3]),
                ("Order", 50, &[]),
                ("Order", 40, &[]),
                ("Holder", 1, &[3]),
                ("Orphan", 3, &[]),
            ],
            &[0, 4],
        );
        let a = HeapAnalysis::new(h.graph.as_ref().unwrap(), h.types.len());
        let heap = Heap {
            snapshot: &h,
            analysis: Some(&a),
        };
        let mut req = HeapDominatorsRequest {
            snapshot_id: 0,
            parent: None,
            offset: 0,
            limit: 10,
        };
        let top = dominators(&heap, &req).unwrap();
        let r: Vec<(&str, u64)> = top.rows.iter().map(|r| (r.type_name.as_str(), r.retained)).collect();
        assert_eq!(r, [("Cache", 70), ("Order", 40), ("Holder", 1)]);
        assert_eq!(top.total_bytes, 111);
        req.parent = Some(0);
        assert_eq!(dominators(&heap, &req).unwrap().rows[0].type_name, "List");
        req.parent = Some(5);
        assert!(dominators(&heap, &req).is_err(), "unreachable");
        req.parent = Some(99);
        assert!(dominators(&heap, &req).is_err());

        let inst = instances(
            &heap,
            &HeapInstancesRequest {
                snapshot_id: 0,
                type_name: "Order".into(),
                offset: 0,
                limit: 10,
            },
        )
        .unwrap();
        assert_eq!(inst.rows.iter().map(|r| r.object).collect::<Vec<_>>(), [2, 3]);
        assert_eq!(inst.total_bytes, 90);
        assert!(instances(
            &heap,
            &HeapInstancesRequest {
                snapshot_id: 0,
                type_name: "Nope".into(),
                offset: 0,
                limit: 10
            }
        )
        .is_err());

        let mut preq = HeapPathsRequest {
            snapshot_id: 0,
            object: 3,
            max_paths: None,
            max_depth: None,
        };
        let p = paths(&heap, &preq).unwrap();
        assert_eq!(p.paths.len(), 2);
        let first: Vec<(&str, Option<&str>)> = p.paths[0].iter().map(|s| (s.type_name.as_str(), s.edge.as_deref())).collect();
        assert_eq!(first, [("Holder", None), ("Order", Some("f0"))]);
        assert_eq!(p.paths[1].len(), 3);
        preq.object = 5;
        assert!(paths(&heap, &preq).unwrap().unreachable);
        preq.object = 99;
        assert!(paths(&heap, &preq).is_err());
        let flat = stats(&[("X", 1, 1)]);
        let none = Heap {
            snapshot: &flat,
            analysis: None,
        };
        assert!(paths(&none, &preq).unwrap_err().to_string().contains("no object graph"));
    }
}
