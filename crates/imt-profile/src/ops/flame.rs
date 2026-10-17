//! Flame graph / icicle layout: rectangles for the visible part of a zoomed
//! node, culled below a minimum width, with a search highlight mask. The
//! top-down layout is the classic flame graph (or icicle); the bottom-up
//! layout draws the inverted tree.

use crate::api::{FlameGraph, FlameOrder, FlameRect, FlameRequest, FrameDto, Viewport};
use crate::error::{ProfileError, Result};
use crate::limits::query::MAX_RECTS;
use crate::model::{Profile, NONE};
use crate::ops::common::{frame_dto, pct, pick_metric, Search};
use crate::ops::nav::{children, level_at, Level};
use crate::ops::tree::CallTree;
use rustc_hash::FxHashSet;

/// Layout parameters, validated.
#[derive(Debug, Clone, Copy)]
pub struct Frame2d {
    pub viewport: Viewport,
    pub width_px: f64,
    pub min_width_px: f64,
    pub max_depth: u32,
    pub max_rects: usize,
    pub order: FlameOrder,
}

impl Frame2d {
    pub fn new(
        viewport: Viewport,
        width_px: f64,
        min_width_px: Option<f64>,
        max_depth: Option<u32>,
        max_rects: Option<usize>,
        order: FlameOrder,
    ) -> Result<Frame2d> {
        let Viewport { start, end } = viewport;
        if !(start.is_finite() && end.is_finite() && (0.0..1.0).contains(&start) && start < end && end <= 1.0) {
            return Err(ProfileError::invalid("the viewport must satisfy 0 ≤ start < end ≤ 1"));
        }
        if !(width_px.is_finite() && (1.0..=100_000.0).contains(&width_px)) {
            return Err(ProfileError::invalid("widthPx must be between 1 and 100000"));
        }
        let min_width_px = min_width_px.unwrap_or(0.5);
        if !(min_width_px.is_finite() && min_width_px >= 0.0) {
            return Err(ProfileError::invalid("minWidthPx must be 0 or more"));
        }
        Ok(Frame2d {
            viewport,
            width_px,
            min_width_px,
            max_depth: max_depth.unwrap_or(u32::MAX).max(1),
            max_rects: max_rects.unwrap_or(50_000).clamp(1, MAX_RECTS),
            order,
        })
    }
}

/// A node to lay out: frame, value, self value and a handle to expand it.
pub struct Item<H> {
    pub frame: u32,
    pub value: u64,
    pub self_value: u64,
    pub handle: H,
}

/// Result of [`layout`].
pub struct Laid {
    pub rects: Vec<FlameRect>,
    pub frames: Vec<u32>,
    pub truncated: bool,
    pub max_depth: u32,
}

/// Lays out `top` (depth 0, spanning `total`) and their descendants.
pub fn layout<H>(
    profile: &Profile,
    params: &Frame2d,
    total: u64,
    top: Vec<Item<H>>,
    search: Option<&Search>,
    mut expand: impl FnMut(&H) -> Vec<Item<H>>,
    mut decorate: impl FnMut(&H, &mut FlameRect),
) -> Laid {
    let mut out = Laid {
        rects: Vec::new(),
        frames: Vec::new(),
        truncated: false,
        max_depth: 0,
    };
    if total == 0 {
        return out;
    }
    let span = params.viewport.end - params.viewport.start;
    let total = total as f64;
    let mut seen: FxHashSet<u32> = FxHashSet::default();
    let order = |items: &mut Vec<Item<H>>| match params.order {
        FlameOrder::Name => items.sort_by(|a, b| {
            profile.frames[a.frame as usize]
                .function
                .cmp(&profile.frames[b.frame as usize].function)
                .then(a.frame.cmp(&b.frame))
        }),
        FlameOrder::Value => items.sort_by(|a, b| b.value.cmp(&a.value).then(a.frame.cmp(&b.frame))),
    };
    let mut top = top;
    order(&mut top);
    // Depth-first, explicit stack of (item, absolute x as a fraction of total, depth, parent rect).
    let mut placed: Vec<(Item<H>, f64, u32, Option<u32>)> = Vec::with_capacity(top.len());
    let mut x = 0.0;
    for it in top {
        let w = it.value as f64 / total;
        placed.push((it, x, 0, None));
        x += w;
    }
    // Reversed, so the leftmost item is laid out first.
    let mut stack: Vec<(Item<H>, f64, u32, Option<u32>)> = placed.into_iter().rev().collect();
    while let Some((it, x_abs, depth, parent)) = stack.pop() {
        let w_abs = it.value as f64 / total;
        let vx = (x_abs - params.viewport.start) / span;
        let vw = w_abs / span;
        if vx + vw <= 0.0 || vx >= 1.0 || vw * params.width_px < params.min_width_px || it.value == 0 {
            continue;
        }
        if out.rects.len() >= params.max_rects {
            out.truncated = true;
            break;
        }
        let index = out.rects.len() as u32;
        let mut rect = FlameRect {
            frame: it.frame,
            parent,
            depth,
            x: vx,
            w: vw,
            value: it.value,
            self_value: it.self_value,
            matched: search.is_some_and(|s| s.matches_frame(&profile.frames[it.frame as usize])),
            delta: None,
            baseline_value: None,
        };
        decorate(&it.handle, &mut rect);
        out.rects.push(rect);
        out.max_depth = out.max_depth.max(depth);
        if seen.insert(it.frame) {
            out.frames.push(it.frame);
        }
        if depth + 1 >= params.max_depth {
            continue;
        }
        let mut kids = expand(&it.handle);
        order(&mut kids);
        let mut cx = x_abs;
        let mut placed = Vec::with_capacity(kids.len());
        for k in kids {
            let w = k.value as f64 / total;
            placed.push((k, cx, depth + 1, Some(index)));
            cx += w;
        }
        // Push in reverse so the leftmost child is laid out first.
        while let Some(p) = placed.pop() {
            stack.push(p);
        }
    }
    out
}

/// The frames of a focus path (ids already resolved against the tree, so they exist).
pub fn focus_frames(profile: &Profile, focus: &[u32]) -> Vec<FrameDto> {
    focus
        .iter()
        .filter(|&&f| (f as usize) < profile.frames.len())
        .map(|&f| frame_dto(profile, f))
        .collect()
}

/// Per tree node: its frame or an ancestor's (up to the root) matches the search.
pub fn ancestor_matches(profile: &Profile, tree: &CallTree, search: &Search) -> Vec<bool> {
    let hit: Vec<bool> = profile.frames.iter().map(|f| search.matches_frame(f)).collect();
    let mut out = vec![false; tree.len()];
    for n in 0..tree.len() {
        let p = tree.parent[n];
        out[n] = hit[tree.frame[n] as usize] || (p != NONE && out[p as usize]);
    }
    out
}

/// Matched value under a top-down node (or the whole tree), nested matches counted once.
pub fn matched_top_down(profile: &Profile, tree: &CallTree, k: usize, focus: Option<u32>, search: &Search) -> u64 {
    let hit: Vec<bool> = profile.frames.iter().map(|f| search.matches_frame(f)).collect();
    let mut sum = 0u64;
    let mut stack: Vec<u32> = match focus {
        Some(n) => vec![n],
        None => tree.roots.clone(),
    };
    while let Some(n) = stack.pop() {
        if hit[tree.frame[n as usize] as usize] {
            sum = sum.saturating_add(tree.total_of(n, k));
            continue;
        }
        stack.extend_from_slice(tree.children(n));
    }
    sum
}

fn to_items(kids: Vec<crate::ops::nav::Child>, k: usize) -> Vec<Item<Level>> {
    kids.into_iter()
        .map(|c| Item {
            frame: c.frame,
            value: c.total_row[k],
            self_value: c.self_row[k],
            handle: c.level,
        })
        .collect()
}

/// The flame graph (top-down) or inverted flame graph (bottom-up) of one profile.
pub fn flame(profile: &Profile, tree: &CallTree, req: &FlameRequest) -> Result<FlameGraph> {
    let (metric, k, measurement) = pick_metric(profile, tree, req.metric)?;
    let params = Frame2d::new(
        req.viewport,
        req.width_px,
        req.min_width_px,
        req.max_depth,
        req.max_rects,
        req.order,
    )?;
    let search = Search::new(req.search.as_deref(), req.search_regex)?;
    let level = level_at(tree, req.direction, &req.focus)?;

    // The focused item(s) at depth 0 and the value that spans the width.
    let (top, focus_value): (Vec<Item<Level>>, u64) = if req.focus.is_empty() {
        (to_items(children(tree, &level), k), tree.grand[k])
    } else {
        let (value, self_value) = match &level {
            Level::TopDown(Some(n)) => (tree.total_of(*n, k), tree.self_of(*n, k)),
            Level::BottomUp(cands) => {
                let v = cands.iter().map(|&(leaf, _)| tree.self_of(leaf, k)).fold(0u64, u64::saturating_add);
                let s = cands
                    .iter()
                    .filter(|&&(_, cur)| tree.parent[cur as usize] == NONE)
                    .map(|&(leaf, _)| tree.self_of(leaf, k))
                    .fold(0u64, u64::saturating_add);
                (v, s)
            }
            Level::TopDown(None) => (tree.grand[k], 0),
        };
        let frame = *req.focus.last().unwrap_or(&0);
        (
            vec![Item {
                frame,
                value,
                self_value,
                handle: level.clone(),
            }],
            value,
        )
    };

    let matched_value = match (&search, &level) {
        (None, _) => 0,
        (Some(s), Level::TopDown(focus)) => matched_top_down(profile, tree, k, *focus, s),
        (Some(s), Level::BottomUp(cands)) => {
            let anc = ancestor_matches(profile, tree, s);
            if cands.is_empty() {
                (0..tree.len() as u32)
                    .filter(|&n| anc[n as usize])
                    .map(|n| tree.self_of(n, k))
                    .fold(0, u64::saturating_add)
            } else {
                cands
                    .iter()
                    .filter(|&&(_, cur)| anc[cur as usize])
                    .map(|&(leaf, _)| tree.self_of(leaf, k))
                    .fold(0, u64::saturating_add)
            }
        }
    };

    let laid = layout(
        profile,
        &params,
        focus_value,
        top,
        search.as_ref(),
        |h| to_items(children(tree, h), k),
        |_, _| {},
    );
    Ok(FlameGraph {
        rects: laid.rects,
        frames: laid.frames.iter().map(|&f| frame_dto(profile, f)).collect(),
        focus_frames: focus_frames(profile, &req.focus),
        focus_value,
        grand_total: tree.grand[k],
        matched_value,
        matched_pct: pct(matched_value, focus_value),
        truncated: laid.truncated,
        max_depth: laid.max_depth,
        metric,
        measurement,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{ProfileView, TreeDirection};
    use crate::ops::tree::tests::{fid, profile};
    use crate::ops::tree::CompiledView;

    fn req(direction: TreeDirection) -> FlameRequest {
        FlameRequest {
            profile_id: 0,
            view: ProfileView::default(),
            direction,
            focus: vec![],
            viewport: Viewport::default(),
            width_px: 1000.0,
            min_width_px: Some(0.0),
            max_depth: None,
            metric: Some(crate::model::MetricId::Samples),
            order: FlameOrder::Name,
            search: None,
            search_regex: false,
            max_rects: None,
        }
    }

    #[test]
    fn lays_out_children_left_aligned_in_name_order() {
        let p = profile(&[(0, 0, "main;b", 2), (0, 1, "main;a", 1), (0, 2, "main", 1)]);
        let t = CallTree::build(&p, &CompiledView::all());
        let g = flame(&p, &t, &req(TreeDirection::TopDown)).unwrap();
        let r: Vec<(String, u32, f64, f64)> = g
            .rects
            .iter()
            .map(|r| (p.frames[r.frame as usize].function.clone(), r.depth, r.x, r.w))
            .collect();
        assert_eq!(
            r,
            [
                ("main".to_string(), 0, 0.0, 1.0),
                ("a".to_string(), 1, 0.0, 0.25),
                ("b".to_string(), 1, 0.25, 0.5)
            ]
        );
        assert_eq!(g.frames.len(), 3);
        let parents: Vec<Option<u32>> = g.rects.iter().map(|r| r.parent).collect();
        assert_eq!(parents, [None, Some(0), Some(0)], "children link to their enclosing rectangle");
        assert!(g.focus_frames.is_empty(), "no zoom, no breadcrumbs");
        assert_eq!(g.focus_value, 4);
        assert_eq!(g.max_depth, 1);
        let mut by_value = req(TreeDirection::TopDown);
        by_value.order = FlameOrder::Value;
        let g = flame(&p, &t, &by_value).unwrap();
        assert_eq!(p.frames[g.rects[1].frame as usize].function, "b");
    }

    #[test]
    fn zoom_viewport_culling_search_and_caps() {
        let p = profile(&[(0, 0, "main;a;json", 6), (0, 1, "main;b;json", 2), (0, 2, "main;c", 2)]);
        let t = CallTree::build(&p, &CompiledView::all());
        let mut r = req(TreeDirection::TopDown);
        r.focus = vec![fid(&p, "main"), fid(&p, "a")];
        let g = flame(&p, &t, &r).unwrap();
        assert_eq!(g.focus_value, 6);
        assert_eq!(g.rects[0].w, 1.0);
        assert_eq!(g.rects.len(), 2);
        let crumbs: Vec<&str> = g.focus_frames.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(crumbs, ["main", "a"]);
        assert_eq!((g.rects[0].parent, g.rects[1].parent), (None, Some(0)));
        // Deeper levels link to the right parent (not just the previous rectangle).
        let g = flame(&p, &t, &req(TreeDirection::TopDown)).unwrap();
        for (i, r) in g.rects.iter().enumerate() {
            match r.parent {
                None => assert_eq!(r.depth, 0),
                Some(pi) => {
                    let parent = &g.rects[pi as usize];
                    assert!((pi as usize) < i && parent.depth + 1 == r.depth);
                    assert!(
                        parent.x <= r.x + 1e-9 && r.x + r.w <= parent.x + parent.w + 1e-9,
                        "nested in its parent"
                    );
                }
            }
        }
        // Viewport: the right half of main only shows b and c (and main itself).
        let mut r = req(TreeDirection::TopDown);
        r.viewport = Viewport { start: 0.6, end: 1.0 };
        let g = flame(&p, &t, &r).unwrap();
        let names: Vec<&str> = g.rects.iter().map(|r| p.frames[r.frame as usize].function.as_str()).collect();
        assert_eq!(names, ["main", "b", "json", "c"]);
        assert!((g.rects[1].x - 0.0).abs() < 1e-9 && (g.rects[1].w - 0.5).abs() < 1e-9);
        // Culling by width.
        let mut r = req(TreeDirection::TopDown);
        r.width_px = 10.0;
        r.min_width_px = Some(3.0);
        let g = flame(&p, &t, &r).unwrap();
        assert!(g.rects.iter().all(|r| r.w * 10.0 >= 3.0));
        // Search: json matched under a and b, counted once each.
        let mut r = req(TreeDirection::TopDown);
        r.search = Some("JSON".into());
        let g = flame(&p, &t, &r).unwrap();
        assert_eq!(g.matched_value, 8);
        assert_eq!(g.matched_pct, 80.0);
        assert_eq!(g.rects.iter().filter(|r| r.matched).count(), 2);
        let mut r = req(TreeDirection::TopDown);
        r.max_rects = Some(2);
        assert!(flame(&p, &t, &r).unwrap().truncated);
        let mut r = req(TreeDirection::TopDown);
        r.max_depth = Some(1);
        assert_eq!(flame(&p, &t, &r).unwrap().rects.len(), 1);
    }

    #[test]
    fn bottom_up_layout_and_search() {
        let p = profile(&[(0, 0, "main;a;json", 6), (0, 1, "main;b;json", 2), (0, 2, "main;c", 2)]);
        let t = CallTree::build(&p, &CompiledView::all());
        let g = flame(&p, &t, &req(TreeDirection::BottomUp)).unwrap();
        let top: Vec<(&str, u64)> = g
            .rects
            .iter()
            .filter(|r| r.depth == 0)
            .map(|r| (p.frames[r.frame as usize].function.as_str(), r.value))
            .collect();
        assert_eq!(top, [("c", 2), ("json", 8)]);
        let mut r = req(TreeDirection::BottomUp);
        r.focus = vec![fid(&p, "json")];
        r.search = Some("^a$".into());
        r.search_regex = true;
        let g = flame(&p, &t, &r).unwrap();
        assert_eq!(g.focus_value, 8);
        assert_eq!(g.matched_value, 6);
        let mut r = req(TreeDirection::BottomUp);
        r.search = Some("main".into());
        assert_eq!(flame(&p, &t, &r).unwrap().matched_value, 10);
        let mut r = req(TreeDirection::BottomUp);
        r.focus = vec![fid(&p, "c"), fid(&p, "main")];
        let g = flame(&p, &t, &r).unwrap();
        let crumbs: Vec<&str> = g.focus_frames.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(crumbs, ["c", "main"]);
        assert_eq!(focus_frames(&p, &[999]), vec![], "unknown ids are skipped");
        assert_eq!((g.rects[0].value, g.rects[0].self_value), (2, 2), "complete stacks are self");
    }

    #[test]
    fn invalid_parameters() {
        let p = profile(&[(0, 0, "main", 1)]);
        let t = CallTree::build(&p, &CompiledView::all());
        for (start, end, w) in [
            (0.5, 0.5, 100.0),
            (-0.1, 1.0, 100.0),
            (0.0, 1.1, 100.0),
            (0.0, 1.0, 0.0),
            (f64::NAN, 1.0, 10.0),
        ] {
            let mut r = req(TreeDirection::TopDown);
            r.viewport = Viewport { start, end };
            r.width_px = w;
            assert!(flame(&p, &t, &r).is_err(), "{start} {end} {w}");
        }
        let mut r = req(TreeDirection::TopDown);
        r.min_width_px = Some(-1.0);
        assert!(flame(&p, &t, &r).is_err());
        let mut r = req(TreeDirection::TopDown);
        r.search = Some("[".into());
        r.search_regex = true;
        assert!(flame(&p, &t, &r).is_err());
        // An empty view lays out nothing.
        let empty = CallTree::build(
            &p,
            &CompiledView::compile(
                &p,
                &ProfileView {
                    range: Some(crate::api::TimeRange { from_ns: 5, to_ns: 6 }),
                    ..ProfileView::default()
                },
            )
            .unwrap(),
        );
        assert!(flame(&p, &empty, &req(TreeDirection::TopDown)).unwrap().rects.is_empty());
    }
}
