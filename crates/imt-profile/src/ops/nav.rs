//! Navigating the call tree top-down and inverted (bottom-up), with lazy
//! paging of children and "expand hottest path".
//!
//! The inverted tree is never materialised: a node is the set of call-tree
//! nodes whose stacks end with its path (leaf first), so its children are
//! found by grouping those nodes' next callers.

use crate::api::{FramePath, FramePaths, FramePathsRequest, HotPath, HotPathRequest, TreeDirection, TreeNode, TreePage, TreeRequest};
use crate::error::{ProfileError, Result};
use crate::limits::query::{MAX_FRAME_PATHS, MAX_HOT_PATH};
use crate::model::{Profile, NONE};
use crate::ops::common::{check_page, frame_dto, function_key, page, pct, pick_metric, values};
use crate::ops::tree::CallTree;
use rustc_hash::FxHashMap;

/// A position in the tree.
#[derive(Debug, Clone)]
pub enum Level {
    /// A call-tree node (`None` = above the roots).
    TopDown(Option<u32>),
    /// Inverted: `(leaf node, ancestor reached so far)` pairs.
    BottomUp(Vec<(u32, u32)>),
}

/// One child of a [`Level`].
#[derive(Debug, Clone)]
pub struct Child {
    pub frame: u32,
    pub self_row: Vec<u64>,
    pub total_row: Vec<u64>,
    pub has_children: bool,
    pub level: Level,
}

fn fold_row(tree: &CallTree, acc: &mut [u64], row: &[u64]) {
    for (k, id) in tree.metrics.iter().enumerate() {
        id.fold(&mut acc[k], row[k]);
    }
}

/// The level at `path` in `direction`.
pub fn level_at(tree: &CallTree, direction: TreeDirection, path: &[u32]) -> Result<Level> {
    match direction {
        TreeDirection::TopDown => Ok(Level::TopDown(tree.resolve(path)?)),
        TreeDirection::BottomUp => {
            let Some((&first, rest)) = path.split_first() else {
                return Ok(Level::BottomUp(Vec::new()));
            };
            let mut cands: Vec<(u32, u32)> = tree
                .nodes_of_frame(first)
                .iter()
                .filter(|&&n| tree.self_row(n).iter().any(|&v| v > 0))
                .map(|&n| (n, n))
                .collect();
            for &f in rest {
                cands = cands
                    .into_iter()
                    .filter_map(|(leaf, cur)| {
                        let p = tree.parent[cur as usize];
                        (p != NONE && tree.frame[p as usize] == f).then_some((leaf, p))
                    })
                    .collect();
            }
            if cands.is_empty() {
                return Err(ProfileError::invalid(
                    "that call path is not in the current view (the filters changed?)",
                ));
            }
            Ok(Level::BottomUp(cands))
        }
    }
}

/// Children of a level, unsorted.
pub fn children(tree: &CallTree, level: &Level) -> Vec<Child> {
    let m = tree.m();
    match level {
        Level::TopDown(node) => {
            let kids: &[u32] = match node {
                None => &tree.roots,
                Some(n) => tree.children(*n),
            };
            kids.iter()
                .map(|&c| Child {
                    frame: tree.frame[c as usize],
                    self_row: tree.self_row(c).to_vec(),
                    total_row: tree.total_row(c).to_vec(),
                    has_children: !tree.children(c).is_empty(),
                    level: Level::TopDown(Some(c)),
                })
                .collect()
        }
        Level::BottomUp(cands) => {
            let mut out: Vec<Child> = Vec::new();
            let mut by_frame: FxHashMap<u32, usize> = FxHashMap::default();
            let mut add = |out: &mut Vec<Child>, leaf: u32, node: u32| {
                let f = tree.frame[node as usize];
                let i = *by_frame.entry(f).or_insert_with(|| {
                    out.push(Child {
                        frame: f,
                        self_row: vec![0; m],
                        total_row: vec![0; m],
                        has_children: false,
                        level: Level::BottomUp(Vec::new()),
                    });
                    out.len() - 1
                });
                let c = &mut out[i];
                fold_row(tree, &mut c.total_row, tree.self_row(leaf));
                if tree.parent[node as usize] == NONE {
                    fold_row(tree, &mut c.self_row, tree.self_row(leaf));
                } else {
                    c.has_children = true;
                }
                if let Level::BottomUp(v) = &mut c.level {
                    v.push((leaf, node));
                }
            };
            if cands.is_empty() {
                // The top of the inverted tree: every node where time was spent.
                for n in 0..tree.len() as u32 {
                    if tree.self_row(n).iter().any(|&v| v > 0) {
                        add(&mut out, n, n);
                    }
                }
            } else {
                for &(leaf, cur) in cands {
                    let p = tree.parent[cur as usize];
                    if p != NONE {
                        add(&mut out, leaf, p);
                    }
                }
            }
            out
        }
    }
}

/// Sorts children by the total of metric `k`, heaviest first (ties by name).
fn sort_children(profile: &Profile, kids: &mut [Child], k: usize) {
    kids.sort_by(|a, b| {
        b.total_row[k].cmp(&a.total_row[k]).then_with(|| {
            profile.frames[a.frame as usize]
                .function
                .cmp(&profile.frames[b.frame as usize].function)
        })
    });
}

fn node_dto(profile: &Profile, tree: &CallTree, c: &Child, k: usize) -> TreeNode {
    TreeNode {
        frame: frame_dto(profile, c.frame),
        self_values: values(&tree.metrics, &c.self_row),
        total_values: values(&tree.metrics, &c.total_row),
        self_pct: pct(c.self_row[k], tree.grand[k]),
        total_pct: pct(c.total_row[k], tree.grand[k]),
        has_children: c.has_children,
    }
}

/// One page of the children of the node at `req.path`.
pub fn tree_page(profile: &Profile, tree: &CallTree, req: &TreeRequest) -> Result<TreePage> {
    check_page(req.limit)?;
    let (metric, k, measurement) = pick_metric(profile, tree, req.metric)?;
    let level = level_at(tree, req.direction, &req.path)?;
    let mut kids = children(tree, &level);
    sort_children(profile, &mut kids, k);
    let total_children = kids.len();
    let nodes = page(kids, req.offset, req.limit)
        .iter()
        .map(|c| node_dto(profile, tree, c, k))
        .collect();
    Ok(TreePage {
        nodes,
        total_children,
        grand_total: values(&tree.metrics, &tree.grand),
        metric,
        measurement,
    })
}

/// Follows the heaviest child from `req.path` while it carries at least
/// `minFraction` of the grand total.
pub fn hot_path(profile: &Profile, tree: &CallTree, req: &HotPathRequest) -> Result<HotPath> {
    let (_, k, _) = pick_metric(profile, tree, req.metric)?;
    let min_fraction = req.min_fraction.unwrap_or(0.01);
    if !(0.0..=1.0).contains(&min_fraction) {
        return Err(ProfileError::invalid("minFraction must be between 0 and 1"));
    }
    let threshold = (tree.grand[k] as f64 * min_fraction).max(1.0);
    let mut level = level_at(tree, req.direction, &req.path)?;
    let mut path = req.path.clone();
    let mut nodes = Vec::new();
    while nodes.len() < MAX_HOT_PATH {
        let kids = children(tree, &level);
        let Some(best) = kids.into_iter().max_by(|a, b| {
            a.total_row[k].cmp(&b.total_row[k]).then_with(|| {
                profile.frames[b.frame as usize]
                    .function
                    .cmp(&profile.frames[a.frame as usize].function)
            })
        }) else {
            break;
        };
        if (best.total_row[k] as f64) < threshold {
            break;
        }
        path.push(best.frame);
        nodes.push(node_dto(profile, tree, &best, k));
        level = best.level;
    }
    Ok(HotPath { path, nodes })
}

/// The hottest top-down paths from a root to the function of `req.frame`
/// (every frame with the same name and module). A recursive occurrence
/// nested under another is part of the outer one and not listed again.
pub fn frame_paths(profile: &Profile, tree: &CallTree, req: &FramePathsRequest) -> Result<FramePaths> {
    let (metric, k, _) = pick_metric(profile, tree, req.metric)?;
    let limit = req.limit.unwrap_or(5);
    if limit == 0 || limit > MAX_FRAME_PATHS {
        return Err(ProfileError::invalid(format!("limit must be between 1 and {MAX_FRAME_PATHS}")));
    }
    let target = profile
        .frames
        .get(req.frame as usize)
        .ok_or_else(|| ProfileError::invalid(format!("frame {} is not in this profile", req.frame)))?;
    let key = function_key(target);
    let same: Vec<bool> = profile
        .frames
        .iter()
        .map(|f| f.function == target.function && function_key(f) == key)
        .collect();
    // Outermost occurrences: matching nodes with no matching ancestor.
    let mut found: Vec<u32> = Vec::new();
    let mut open = 0u32;
    tree.walk(|n, entering| {
        if !same[tree.frame[n as usize] as usize] {
            return;
        }
        if entering {
            if open == 0 {
                found.push(n);
            }
            open += 1;
        } else {
            open -= 1;
        }
    });
    let total_paths = found.len();
    found.sort_by(|&a, &b| tree.total_of(b, k).cmp(&tree.total_of(a, k)).then(a.cmp(&b)));
    let paths = found
        .into_iter()
        .take(limit)
        .map(|n| {
            let path = tree.path_of(n);
            FramePath {
                frames: path.iter().map(|&f| frame_dto(profile, f)).collect(),
                path,
                value: tree.total_of(n, k),
                pct: pct(tree.total_of(n, k), tree.grand[k]),
            }
        })
        .collect();
    Ok(FramePaths {
        paths,
        total_paths,
        metric,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ProfileView;
    use crate::ops::tree::tests::{fid, profile};
    use crate::ops::tree::CompiledView;

    fn req(direction: TreeDirection, path: Vec<u32>) -> TreeRequest {
        TreeRequest {
            profile_id: 0,
            view: ProfileView::default(),
            direction,
            path,
            metric: None,
            offset: 0,
            limit: 100,
        }
    }

    fn names(p: &crate::model::Profile, page: &TreePage) -> Vec<(String, u64, u64, bool)> {
        page.nodes
            .iter()
            .map(|n| {
                (
                    n.frame.name.clone(),
                    n.self_values[&crate::model::MetricId::Samples],
                    n.total_values[&crate::model::MetricId::Samples],
                    n.has_children,
                )
            })
            .inspect(|_| {
                let _ = p;
            })
            .collect()
    }

    #[test]
    fn top_down_pages() {
        let p = profile(&[(0, 0, "main;a;b", 2), (0, 1, "main;a", 1), (0, 2, "main;c", 3)]);
        let t = CallTree::build(&p, &CompiledView::all());
        let top = tree_page(&p, &t, &req(TreeDirection::TopDown, vec![])).unwrap();
        assert_eq!(names(&p, &top), [("main".to_string(), 0, 6, true)]);
        let kids = tree_page(&p, &t, &req(TreeDirection::TopDown, vec![fid(&p, "main")])).unwrap();
        assert_eq!(names(&p, &kids), [("a".to_string(), 1, 3, true), ("c".to_string(), 3, 3, false)]);
        assert_eq!(kids.total_children, 2);
        assert_eq!(kids.nodes[0].total_pct, 50.0);
        let mut r = req(TreeDirection::TopDown, vec![fid(&p, "main")]);
        r.offset = 1;
        r.limit = 1;
        assert_eq!(tree_page(&p, &t, &r).unwrap().nodes[0].frame.name, "c");
        r.limit = 0;
        assert!(tree_page(&p, &t, &r).is_err());
        assert!(tree_page(&p, &t, &req(TreeDirection::TopDown, vec![99])).is_err());
    }

    #[test]
    fn bottom_up_inverts_and_values_add_up() {
        let p = profile(&[(0, 0, "main;a;b", 2), (0, 1, "main;c;b", 1), (0, 2, "main;a", 3), (0, 3, "b", 4)]);
        let t = CallTree::build(&p, &CompiledView::all());
        let top = tree_page(&p, &t, &req(TreeDirection::BottomUp, vec![])).unwrap();
        // b: 2 + 1 + 4 (4 of them with b as the root: self), a: 3.
        assert_eq!(names(&p, &top), [("b".to_string(), 4, 7, true), ("a".to_string(), 0, 3, true)]);
        let b = fid(&p, "b");
        let callers = tree_page(&p, &t, &req(TreeDirection::BottomUp, vec![b])).unwrap();
        assert_eq!(names(&p, &callers), [("a".to_string(), 0, 2, true), ("c".to_string(), 0, 1, true)]);
        let deeper = tree_page(&p, &t, &req(TreeDirection::BottomUp, vec![b, fid(&p, "a")])).unwrap();
        assert_eq!(names(&p, &deeper), [("main".to_string(), 2, 2, false)]);
        let sum: u64 = top.nodes.iter().map(|n| n.total_values[&crate::model::MetricId::Samples]).sum();
        assert_eq!(sum, t.grand[0], "the inverted top level partitions the total");
        assert!(tree_page(&p, &t, &req(TreeDirection::BottomUp, vec![b, fid(&p, "main")])).is_err());
    }

    #[test]
    fn hottest_path_both_directions() {
        let p = profile(&[(0, 0, "main;a;b;c", 5), (0, 1, "main;a;x", 1), (0, 2, "main;y", 2)]);
        let t = CallTree::build(&p, &CompiledView::all());
        let mk = |direction, path: Vec<u32>, min| HotPathRequest {
            profile_id: 0,
            view: ProfileView::default(),
            direction,
            path,
            metric: None,
            min_fraction: min,
        };
        let h = hot_path(&p, &t, &mk(TreeDirection::TopDown, vec![], None)).unwrap();
        let names: Vec<&str> = h.nodes.iter().map(|n| n.frame.name.as_str()).collect();
        assert_eq!(names, ["main", "a", "b", "c"]);
        assert_eq!(h.path.len(), 4);
        let h = hot_path(&p, &t, &mk(TreeDirection::TopDown, vec![fid(&p, "main")], Some(0.7))).unwrap();
        assert_eq!(h.nodes.len(), 1, "stops when the child is below 70%: {:?}", h.path);
        let h = hot_path(&p, &t, &mk(TreeDirection::BottomUp, vec![], None)).unwrap();
        let names: Vec<&str> = h.nodes.iter().map(|n| n.frame.name.as_str()).collect();
        assert_eq!(names, ["c", "b", "a", "main"]);
        assert!(hot_path(&p, &t, &mk(TreeDirection::TopDown, vec![], Some(2.0))).is_err());
    }

    fn fp(frame: u32, limit: Option<usize>) -> FramePathsRequest {
        FramePathsRequest {
            profile_id: 0,
            view: ProfileView::default(),
            frame,
            metric: Some(crate::model::MetricId::Samples),
            limit,
        }
    }

    #[test]
    fn frame_paths_lead_to_the_hottest_occurrences() {
        let p = profile(&[
            (0, 0, "main;a;json", 5),
            (0, 1, "main;b;json", 2),
            (0, 2, "main;c", 4),
            (0, 3, "json;json;x", 1),
            (0, 4, "other;a", 1),
        ]);
        let t = CallTree::build(&p, &CompiledView::all());
        let json = fid(&p, "json");
        let r = frame_paths(&p, &t, &fp(json, None)).unwrap();
        let named: Vec<(Vec<&str>, u64)> = r
            .paths
            .iter()
            .map(|x| (x.frames.iter().map(|f| f.name.as_str()).collect(), x.value))
            .collect();
        assert_eq!(
            named,
            [(vec!["main", "a", "json"], 5), (vec!["main", "b", "json"], 2), (vec!["json"], 1)],
            "hottest first; the nested json under json is part of the outer one"
        );
        assert_eq!(r.total_paths, 3);
        assert_eq!(r.paths[0].path, vec![fid(&p, "main"), fid(&p, "a"), json]);
        assert!((r.paths[0].pct - 5.0 / 13.0 * 100.0).abs() < 1e-9);
        // The path is a valid call-tree path and flame focus.
        assert!(t.node_at(&r.paths[0].path).is_some());
        let r = frame_paths(&p, &t, &fp(json, Some(1))).unwrap();
        assert_eq!((r.paths.len(), r.total_paths), (1, 3));
        assert!(frame_paths(&p, &t, &fp(json, Some(0))).is_err());
        assert!(frame_paths(&p, &t, &fp(json, Some(MAX_FRAME_PATHS + 1))).is_err());
        assert!(frame_paths(&p, &t, &fp(999, None)).is_err());
        // Not in this view: no paths, not an error.
        let view = CompiledView::compile(
            &p,
            &ProfileView {
                range: Some(crate::api::TimeRange { from_ns: 2, to_ns: 2 }),
                ..ProfileView::default()
            },
        )
        .unwrap();
        let narrow = CallTree::build(&p, &view);
        let r = frame_paths(&p, &narrow, &fp(json, None)).unwrap();
        assert!(r.paths.is_empty() && r.total_paths == 0);
    }
}
