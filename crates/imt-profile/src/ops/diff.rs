//! Comparing two profiles (baseline vs target): per-function deltas and a
//! differential flame graph. Values are normalised by each profile's own
//! total, so captures of different lengths compare fairly; functions are
//! matched by name and module, call paths by the functions on them.

use crate::api::{DiffFlameRequest, DiffSide, DiffSort, DiffStatus, FlameGraph, FunctionDiffPage, FunctionDiffRequest, FunctionDiffRow};
use crate::error::{ProfileError, Result};
use crate::model::{MetricId, Profile, NONE};
use crate::ops::common::{check_page, frame_dto, function_key, page, pct, Search};
use crate::ops::flame::{focus_frames, layout, matched_top_down, Frame2d, Item};
use crate::ops::hotspots::{cmp_f64, group};
use crate::ops::tree::{CallTree, CompiledView};
use rustc_hash::FxHashMap;

/// One side of a comparison.
pub struct Side<'a> {
    pub profile: &'a Profile,
    pub tree: &'a CallTree,
    pub view: &'a CompiledView,
}

fn metric_k(side: &Side<'_>, metric: MetricId, which: &str) -> Result<usize> {
    side.tree
        .metrics
        .iter()
        .position(|m| *m == metric)
        .ok_or_else(|| ProfileError::NotAvailable(format!("the {which} profile has no {} data", metric.label().to_lowercase())))
}

/// The metric both profiles share: the request's, else the target's default.
fn common_metric(baseline: &Side<'_>, target: &Side<'_>, metric: Option<MetricId>) -> Result<(MetricId, usize, usize)> {
    let metric = match metric {
        Some(m) => m,
        None => {
            let t = target.profile.default_metric();
            if baseline.tree.metrics.contains(&t) {
                t
            } else {
                *target
                    .tree
                    .metrics
                    .iter()
                    .find(|m| baseline.tree.metrics.contains(m))
                    .ok_or_else(|| ProfileError::NotAvailable("the two profiles measure nothing in common".into()))?
            }
        }
    };
    Ok((metric, metric_k(baseline, metric, "baseline")?, metric_k(target, metric, "target")?))
}

/// Per-function (or per-group) deltas of shares, sorted by change.
pub fn function_diff(baseline: &Side<'_>, target: &Side<'_>, req: &FunctionDiffRequest) -> Result<FunctionDiffPage> {
    check_page(req.limit)?;
    let (metric, kb, kt) = common_metric(baseline, target, req.metric)?;
    let search = Search::new(req.search.as_deref(), false)?;
    let gb = group(baseline.profile, baseline.tree, baseline.view, req.group_by);
    let gt = group(target.profile, target.tree, target.view, req.group_by);
    let (bt, tt) = (baseline.tree.grand[kb], target.tree.grand[kt]);
    let side = |self_value: u64, total_value: u64, whole: u64| DiffSide {
        self_value,
        total_value,
        self_pct: pct(self_value, whole),
        total_pct: pct(total_value, whole),
    };
    let bi = gb.index();
    let mut rows: Vec<FunctionDiffRow> = Vec::new();
    let mut seen = vec![false; gb.len()];
    for t in (0..gt.len()).filter(|&t| gt.touched[t]) {
        let b = bi.get(gt.keys[t].as_str()).copied().filter(|&b| gb.touched[b]);
        if let Some(b) = b {
            seen[b] = true;
        }
        let bs = b.map(|b| side(gb.self_of(b, kb), gb.total_of(b, kb), bt)).unwrap_or(side(0, 0, bt));
        let ts = side(gt.self_of(t, kt), gt.total_of(t, kt), tt);
        let status = match b {
            None => DiffStatus::New,
            Some(_) if bs.self_value == 0 && bs.total_value == 0 => DiffStatus::New,
            Some(_) if (ts.self_pct - bs.self_pct).abs() < 1e-9 && (ts.total_pct - bs.total_pct).abs() < 1e-9 => DiffStatus::Same,
            Some(_) => DiffStatus::Changed,
        };
        rows.push(FunctionDiffRow {
            key: gt.keys[t].clone(),
            name: gt.names[t].clone(),
            module: gt.modules[t].clone(),
            target_frame: gt.frame[t],
            delta_self_pct: ts.self_pct - bs.self_pct,
            delta_total_pct: ts.total_pct - bs.total_pct,
            baseline: bs,
            target: ts,
            status,
        });
    }
    for b in (0..gb.len()).filter(|&b| gb.touched[b] && !seen[b]) {
        let bs = side(gb.self_of(b, kb), gb.total_of(b, kb), bt);
        rows.push(FunctionDiffRow {
            key: gb.keys[b].clone(),
            name: gb.names[b].clone(),
            module: gb.modules[b].clone(),
            target_frame: None,
            delta_self_pct: -bs.self_pct,
            delta_total_pct: -bs.total_pct,
            baseline: bs,
            target: side(0, 0, tt),
            status: DiffStatus::Removed,
        });
    }
    if let Some(s) = &search {
        rows.retain(|r| s.is_match(&r.name) || r.module.as_deref().is_some_and(|m| s.is_match(m)));
    }
    rows.sort_by(|a, b| {
        let o = match req.sort {
            DiffSort::AbsDeltaSelf => cmp_f64(b.delta_self_pct.abs(), a.delta_self_pct.abs()),
            DiffSort::AbsDeltaTotal => cmp_f64(b.delta_total_pct.abs(), a.delta_total_pct.abs()),
            DiffSort::DeltaSelf => cmp_f64(b.delta_self_pct, a.delta_self_pct),
            DiffSort::DeltaTotal => cmp_f64(b.delta_total_pct, a.delta_total_pct),
            DiffSort::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        };
        o.then_with(|| a.key.cmp(&b.key))
    });
    let total_rows = rows.len();
    Ok(FunctionDiffPage {
        rows: page(rows, req.offset, req.limit),
        total_rows,
        baseline_total: bt,
        target_total: tt,
        metric,
    })
}

/// Differential flame graph: the target's layout, each rectangle with the
/// change of its share against the same call path in the baseline.
pub fn diff_flame(baseline: &Side<'_>, target: &Side<'_>, req: &DiffFlameRequest) -> Result<FlameGraph> {
    let (metric, kb, kt) = common_metric(baseline, target, req.metric)?;
    let params = Frame2d::new(
        req.viewport,
        req.width_px,
        req.min_width_px,
        req.max_depth,
        req.max_rects,
        req.order,
    )?;
    let search = Search::new(req.search.as_deref(), req.search_regex)?;
    let (bp, bt) = (baseline.profile, baseline.tree);
    let (tp, tt) = (target.profile, target.tree);

    // Function keys shared by both profiles, and the baseline's children by (parent, key).
    let mut keys: FxHashMap<String, u32> = FxHashMap::default();
    let mut key_of = |f: &crate::model::Frame| {
        let n = keys.len() as u32;
        *keys.entry(function_key(f)).or_insert(n)
    };
    let bkey: Vec<u32> = bp.frames.iter().map(&mut key_of).collect();
    let tkey: Vec<u32> = tp.frames.iter().map(&mut key_of).collect();
    let mut bchild: FxHashMap<(u32, u32), u32> = FxHashMap::default();
    for n in 0..bt.len() as u32 {
        if bt.is_live(n) {
            bchild.insert((bt.parent[n as usize], bkey[bt.frame[n as usize] as usize]), n);
        }
    }
    // The baseline node for target node `n` under baseline node `bparent` (NONE = the top level).
    let counterpart = |bparent: Option<u32>, n: u32| -> Option<u32> {
        bparent.and_then(|bp| bchild.get(&(bp, tkey[tt.frame[n as usize] as usize])).copied())
    };
    let focus = tt.resolve(&req.focus)?;
    let mut b_focus = Some(NONE);
    for &f in &req.focus {
        b_focus = b_focus.and_then(|bp| bchild.get(&(bp, tkey[f as usize])).copied());
    }
    let items = |nodes: &[u32], bparent: Option<u32>| -> Vec<Item<(u32, Option<u32>)>> {
        nodes
            .iter()
            .map(|&n| Item {
                frame: tt.frame[n as usize],
                value: tt.total_of(n, kt),
                self_value: tt.self_of(n, kt),
                handle: (n, counterpart(bparent, n)),
            })
            .collect()
    };
    let (top, focus_value) = match focus {
        None => (items(&tt.roots, Some(NONE)), tt.grand[kt]),
        Some(n) => (
            vec![Item {
                frame: tt.frame[n as usize],
                value: tt.total_of(n, kt),
                self_value: tt.self_of(n, kt),
                handle: (n, b_focus),
            }],
            tt.total_of(n, kt),
        ),
    };
    let (bgrand, tgrand) = (bt.grand[kb], tt.grand[kt]);
    let laid = layout(
        tp,
        &params,
        focus_value,
        top,
        search.as_ref(),
        |&(n, b)| items(tt.children(n), b),
        |&(n, b), rect| {
            let bv = b.map(|b| bt.total_of(b, kb)).unwrap_or(0);
            let bshare = if bgrand == 0 { 0.0 } else { bv as f64 / bgrand as f64 };
            let tshare = if tgrand == 0 {
                0.0
            } else {
                tt.total_of(n, kt) as f64 / tgrand as f64
            };
            rect.delta = Some(tshare - bshare);
            rect.baseline_value = Some(bshare * tgrand as f64);
        },
    );
    let matched_value = search.as_ref().map(|s| matched_top_down(tp, tt, kt, focus, s)).unwrap_or(0);
    Ok(FlameGraph {
        rects: laid.rects,
        frames: laid.frames.iter().map(|&f| frame_dto(tp, f)).collect(),
        focus_frames: focus_frames(tp, &req.focus),
        focus_value,
        grand_total: tgrand,
        matched_value,
        matched_pct: pct(matched_value, focus_value),
        truncated: laid.truncated,
        max_depth: laid.max_depth,
        metric,
        measurement: tp.metrics[kt].measurement,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{FlameOrder, GroupBy, ProfileView, Viewport};
    use crate::ops::tree::tests::{fid, profile};

    fn fd(sort: DiffSort) -> FunctionDiffRequest {
        FunctionDiffRequest {
            baseline_id: 0,
            target_id: 1,
            view: ProfileView::default(),
            group_by: GroupBy::Function,
            metric: None,
            sort,
            search: None,
            offset: 0,
            limit: 100,
        }
    }

    fn dfr() -> DiffFlameRequest {
        DiffFlameRequest {
            baseline_id: 0,
            target_id: 1,
            view: ProfileView::default(),
            focus: vec![],
            viewport: Viewport::default(),
            width_px: 1000.0,
            min_width_px: Some(0.0),
            max_depth: None,
            metric: Some(MetricId::Samples),
            order: FlameOrder::Name,
            search: None,
            search_regex: false,
            max_rects: None,
        }
    }

    #[test]
    fn identical_profiles_have_no_deltas() {
        let p = profile(&[(0, 0, "main;a;b", 3), (0, 1, "main;c", 1)]);
        let t = CallTree::build(&p, &CompiledView::all());
        let v = CompiledView::all();
        let s = Side {
            profile: &p,
            tree: &t,
            view: &v,
        };
        let d = function_diff(&s, &s, &fd(DiffSort::AbsDeltaSelf)).unwrap();
        assert!(d
            .rows
            .iter()
            .all(|r| r.delta_self_pct == 0.0 && r.delta_total_pct == 0.0 && r.status == DiffStatus::Same));
        let g = diff_flame(&s, &s, &dfr()).unwrap();
        assert!(g.rects.iter().all(|r| r.delta == Some(0.0)));
        assert_eq!(g.rects[0].baseline_value, Some(4.0));
    }

    #[test]
    fn deltas_new_and_removed_functions_normalised() {
        // Baseline: json 1 of 4 (25%). Target (twice as long): json 4 of 8 (50%), audit new, c gone.
        let base = profile(&[(0, 0, "main;work", 2), (0, 1, "main;json", 1), (0, 2, "main;c", 1)]);
        let target = profile(&[(0, 0, "main;work", 3), (0, 1, "main;json", 4), (0, 2, "main;audit", 1)]);
        let (bt, tt) = (
            CallTree::build(&base, &CompiledView::all()),
            CallTree::build(&target, &CompiledView::all()),
        );
        let v = CompiledView::all();
        let b = Side {
            profile: &base,
            tree: &bt,
            view: &v,
        };
        let t = Side {
            profile: &target,
            tree: &tt,
            view: &v,
        };
        let d = function_diff(&b, &t, &fd(DiffSort::DeltaSelf)).unwrap();
        let first = &d.rows[0];
        assert_eq!(first.name, "json");
        assert_eq!(first.delta_self_pct, 25.0);
        assert_eq!(first.status, DiffStatus::Changed);
        let audit = d.rows.iter().find(|r| r.name == "audit").unwrap();
        assert_eq!(audit.status, DiffStatus::New);
        let c = d.rows.iter().find(|r| r.name == "c").unwrap();
        assert_eq!((c.status, c.delta_self_pct, c.target_frame), (DiffStatus::Removed, -25.0, None));
        assert_eq!(d.rows.last().unwrap().name, "c", "the biggest drop is last");
        let mut r = fd(DiffSort::Name);
        r.search = Some("AUD".into());
        assert_eq!(function_diff(&b, &t, &r).unwrap().rows.len(), 1);
        for s in [DiffSort::AbsDeltaTotal, DiffSort::DeltaTotal, DiffSort::AbsDeltaSelf] {
            assert_eq!(function_diff(&b, &t, &fd(s)).unwrap().total_rows, 5);
        }
        let mut bad = fd(DiffSort::Name);
        bad.metric = Some(MetricId::AllocBytes);
        assert!(function_diff(&b, &t, &bad).is_err());

        let g = diff_flame(&b, &t, &dfr()).unwrap();
        let json = g.rects.iter().find(|r| r.frame == fid(&target, "json")).unwrap();
        assert!((json.delta.unwrap() - 0.25).abs() < 1e-9);
        let audit = g.rects.iter().find(|r| r.frame == fid(&target, "audit")).unwrap();
        assert_eq!(audit.baseline_value, Some(0.0));
        let mut z = dfr();
        z.focus = vec![fid(&target, "main"), fid(&target, "json")];
        z.search = Some("json".into());
        let g = diff_flame(&b, &t, &z).unwrap();
        assert_eq!((g.rects.len(), g.focus_value, g.matched_value), (1, 4, 4));
        let crumbs: Vec<&str> = g.focus_frames.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(crumbs, ["main", "json"]);
        assert_eq!(g.rects[0].parent, None);
        assert!((g.rects[0].delta.unwrap() - 0.25).abs() < 1e-9);
        let mut z = dfr();
        z.focus = vec![fid(&target, "main"), fid(&target, "audit")];
        assert_eq!(diff_flame(&b, &t, &z).unwrap().rects[0].baseline_value, Some(0.0));
        let mut z = dfr();
        z.focus = vec![999];
        assert!(diff_flame(&b, &t, &z).is_err());
    }
}
