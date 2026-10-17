//! Caller/callee ("butterfly") view of one function: who calls it and what
//! it calls, each with the inclusive value that flows through that edge.

use crate::api::{Butterfly, ButterflyRequest, ButterflyRow, GroupBy};
use crate::error::{ProfileError, Result};
use crate::model::{Profile, NONE};
use crate::ops::common::{frame_dto, function_key, pct, pick_metric, values};
use crate::ops::hotspots::{group, row};
use crate::ops::tree::{CallTree, CompiledView};
use rustc_hash::FxHashMap;

pub fn butterfly(profile: &Profile, tree: &CallTree, view: &CompiledView, req: &ButterflyRequest) -> Result<Butterfly> {
    crate::ops::common::check_page(req.limit)?;
    let (metric, k, _) = pick_metric(profile, tree, req.metric)?;
    let target = profile
        .frames
        .get(req.frame as usize)
        .ok_or_else(|| ProfileError::invalid(format!("the profile has no frame {}", req.frame)))?;
    let key = function_key(target);
    let same: Vec<bool> = profile.frames.iter().map(|f| function_key(f) == key).collect();
    let m = tree.m();

    // Callers and callees keyed by function (a representative frame); None = no caller.
    let mut callers: FxHashMap<Option<String>, (Option<u32>, Vec<u64>)> = FxHashMap::default();
    let mut callees: FxHashMap<String, (u32, Vec<u64>)> = FxHashMap::default();
    for n in 0..tree.len() as u32 {
        if !same[tree.frame[n as usize] as usize] || !tree.is_live(n) {
            continue;
        }
        let p = tree.parent[n as usize];
        let (ck, cf) = if p == NONE {
            (None, None)
        } else {
            let f = tree.frame[p as usize];
            (Some(function_key(&profile.frames[f as usize])), Some(f))
        };
        let e = callers.entry(ck).or_insert_with(|| (cf, vec![0; m]));
        for (i, id) in tree.metrics.iter().enumerate() {
            id.fold(&mut e.1[i], tree.total_of(n, i));
        }
        for &c in tree.children(n) {
            let f = tree.frame[c as usize];
            let e = callees
                .entry(function_key(&profile.frames[f as usize]))
                .or_insert_with(|| (f, vec![0; m]));
            for (i, id) in tree.metrics.iter().enumerate() {
                id.fold(&mut e.1[i], tree.total_of(c, i));
            }
        }
    }

    let groups = group(profile, tree, view, GroupBy::Function);
    let g = groups
        .index()
        .get(key.as_str())
        .copied()
        .filter(|&g| groups.touched[g])
        .ok_or_else(|| ProfileError::invalid("that function is not in the current view"))?;
    let function = row(profile, &groups, g, k, tree.grand[k]);
    let whole = groups.total_of(g, k);

    let mut caller_rows: Vec<ButterflyRow> = callers
        .into_values()
        .map(|(f, v)| ButterflyRow {
            frame: f.map(|f| frame_dto(profile, f)),
            pct: pct(v[k], whole),
            values: values(&tree.metrics, &v),
        })
        .collect();
    let mut callee_rows: Vec<ButterflyRow> = callees
        .into_values()
        .map(|(f, v)| ButterflyRow {
            frame: Some(frame_dto(profile, f)),
            pct: pct(v[k], whole),
            values: values(&tree.metrics, &v),
        })
        .collect();
    let order = |a: &ButterflyRow, b: &ButterflyRow| {
        let va = a.values.get(&metric).copied().unwrap_or(0);
        let vb = b.values.get(&metric).copied().unwrap_or(0);
        vb.cmp(&va).then_with(|| {
            let na = a.frame.as_ref().map(|f| f.name.as_str()).unwrap_or("");
            let nb = b.frame.as_ref().map(|f| f.name.as_str()).unwrap_or("");
            na.cmp(nb)
        })
    };
    caller_rows.sort_by(order);
    callee_rows.sort_by(order);
    caller_rows.truncate(req.limit);
    callee_rows.truncate(req.limit);
    Ok(Butterfly {
        function,
        callers: caller_rows,
        callees: callee_rows,
        metric,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ProfileView;
    use crate::model::MetricId;
    use crate::ops::tree::tests::{fid, profile};

    #[test]
    fn callers_and_callees() {
        let p = profile(&[
            (0, 0, "main;a;lock", 2),
            (0, 1, "main;b;lock;wait", 3),
            (0, 2, "lock", 1),
            (0, 3, "main;a", 4),
        ]);
        let t = CallTree::build(&p, &CompiledView::all());
        let req = ButterflyRequest {
            profile_id: 0,
            view: ProfileView::default(),
            frame: fid(&p, "lock"),
            metric: Some(MetricId::Samples),
            limit: 10,
        };
        let b = butterfly(&p, &t, &CompiledView::all(), &req).unwrap();
        assert_eq!(b.function.total_values[&MetricId::Samples], 6);
        assert_eq!(b.function.self_values[&MetricId::Samples], 3);
        let callers: Vec<(Option<&str>, u64)> = b
            .callers
            .iter()
            .map(|r| (r.frame.as_ref().map(|f| f.name.as_str()), r.values[&MetricId::Samples]))
            .collect();
        assert_eq!(callers, [(Some("b"), 3), (Some("a"), 2), (None, 1)]);
        assert_eq!(b.callers[0].pct, 50.0);
        assert_eq!(b.callees.len(), 1);
        assert_eq!(b.callees[0].frame.as_ref().unwrap().name, "wait");
        let mut bad = req.clone();
        bad.frame = 999;
        assert!(butterfly(&p, &t, &CompiledView::all(), &bad).is_err());
        let mut none = req.clone();
        none.metric = Some(MetricId::AllocBytes);
        assert!(butterfly(&p, &t, &CompiledView::all(), &none).is_err());
        let one = ButterflyRequest { limit: 1, ..req };
        assert_eq!(butterfly(&p, &t, &CompiledView::all(), &one).unwrap().callers.len(), 1);
    }

    #[test]
    fn functions_outside_the_view_are_reported() {
        let p = profile(&[(0, 0, "main;a", 1), (1, 5, "main;b", 1)]);
        let view = CompiledView::compile(
            &p,
            &ProfileView {
                threads: Some(vec![0]),
                ..ProfileView::default()
            },
        )
        .unwrap();
        let t = CallTree::build(&p, &view);
        let req = ButterflyRequest {
            profile_id: 0,
            view: ProfileView::default(),
            frame: fid(&p, "b"),
            metric: None,
            limit: 10,
        };
        assert!(butterfly(&p, &t, &view, &req).is_err());
    }
}
