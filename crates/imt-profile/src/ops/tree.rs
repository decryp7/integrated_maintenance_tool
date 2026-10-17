//! The aggregated call tree of a profile under a view (time range, threads,
//! "just my code", recursion folding): per node self and total values for
//! every metric. Every other view (hotspots, trees, flame graphs,
//! comparisons, findings) reads one of these.

use crate::api::{ProfileView, RecursionMode};
use crate::error::{ProfileError, Result};
use crate::frames::FrameworkMatcher;
use crate::model::{MetricId, Profile, NONE};
use rustc_hash::FxHashMap;
use std::sync::OnceLock;

/// A validated [`ProfileView`] for one profile.
#[derive(Debug, Clone)]
pub struct CompiledView {
    /// `[from, to]` in ns, or everything.
    pub range: Option<(u64, u64)>,
    /// Per thread: included.
    pub threads: Option<Vec<bool>>,
    /// Per frame: framework (collapsed); `None` when "just my code" is off.
    pub framework: Option<Vec<bool>>,
    pub recursion: RecursionMode,
}

impl CompiledView {
    /// Checks the view against the profile (threads exist, range ordered, patterns valid).
    pub fn compile(profile: &Profile, view: &ProfileView) -> Result<CompiledView> {
        let range = match view.range {
            Some(r) if r.from_ns > r.to_ns => {
                return Err(ProfileError::invalid("the time range ends before it starts"));
            }
            Some(r) if profile.has_time => Some((r.from_ns, r.to_ns)),
            _ => None,
        };
        let threads = match &view.threads {
            None => None,
            Some(ids) => {
                let mut mask = vec![false; profile.threads.len()];
                for &t in ids {
                    match mask.get_mut(t as usize) {
                        Some(m) => *m = true,
                        None => return Err(ProfileError::invalid(format!("the profile has no thread {t}"))),
                    }
                }
                Some(mask)
            }
        };
        let framework = match &view.just_my_code {
            Some(j) if j.enabled => {
                let m = FrameworkMatcher::new(&j.framework_patterns, j.use_frame_flags)?;
                Some(profile.frames.iter().map(|f| m.matches(f)).collect())
            }
            _ => None,
        };
        Ok(CompiledView {
            range,
            threads,
            framework,
            recursion: view.recursion,
        })
    }

    /// Everything, as recorded.
    pub fn all() -> CompiledView {
        CompiledView {
            range: None,
            threads: None,
            framework: None,
            recursion: RecursionMode::Off,
        }
    }

    /// Indices of the samples the view selects.
    pub fn sample_indices<'a>(&'a self, profile: &'a Profile) -> impl Iterator<Item = usize> + 'a {
        let range = match self.range {
            Some((a, b)) => profile.sample_range(a, b),
            None => 0..profile.samples.len(),
        };
        range.filter(move |&i| match &self.threads {
            Some(mask) => mask.get(profile.samples.thread[i] as usize).copied().unwrap_or(false),
            None => true,
        })
    }
}

/// The aggregated tree. Node ids are dense; a parent's id is smaller than its children's.
#[derive(Debug)]
pub struct CallTree {
    pub metrics: Vec<MetricId>,
    pub frame: Vec<u32>,
    pub parent: Vec<u32>,
    self_v: Vec<u64>,
    total_v: Vec<u64>,
    child_start: Vec<u32>,
    child_list: Vec<u32>,
    pub roots: Vec<u32>,
    /// Grand totals per metric (sum over roots; sum of all nodes for calls; max for maxima).
    pub grand: Vec<u64>,
    pub samples_in_view: u64,
    frame_nodes: OnceLock<(Vec<u32>, Vec<u32>)>,
}

impl CallTree {
    /// Aggregates the profile's samples under `view`.
    pub fn build(profile: &Profile, view: &CompiledView) -> CallTree {
        let m = profile.metrics.len();
        let metrics: Vec<MetricId> = profile.metrics.iter().map(|x| x.id).collect();

        // 1. Map every stack node to a node of the transformed tree.
        let mut frame: Vec<u32> = Vec::new();
        let mut parent: Vec<u32> = Vec::new();
        let mut index: FxHashMap<(u32, u32), u32> = FxHashMap::default();
        let mut map: Vec<u32> = Vec::with_capacity(profile.stacks.len());
        let fw = view.framework.as_deref();
        let is_fw = |f: u32| fw.is_some_and(|v| v.get(f as usize).copied().unwrap_or(false));
        for node in &profile.stacks {
            let p = if node.parent == NONE { NONE } else { map[node.parent as usize] };
            let f = node.frame;
            let mapped = if p != NONE && is_fw(f) && is_fw(frame[p as usize]) {
                // Inside a run of framework frames: stays in the run's first frame.
                p
            } else if p != NONE && view.recursion == RecursionMode::Direct && frame[p as usize] == f {
                p
            } else if p != NONE && view.recursion == RecursionMode::All && {
                let mut a = p;
                while a != NONE && frame[a as usize] != f {
                    a = parent[a as usize];
                }
                a != NONE
            } {
                let mut a = p;
                while frame[a as usize] != f {
                    a = parent[a as usize];
                }
                a
            } else {
                *index.entry((p, f)).or_insert_with(|| {
                    frame.push(f);
                    parent.push(p);
                    (frame.len() - 1) as u32
                })
            };
            map.push(mapped);
        }
        drop(index);
        let n = frame.len();

        // 2. Self values from the selected samples.
        let mut self_v = vec![0u64; n * m];
        let mut samples_in_view = 0u64;
        for i in view.sample_indices(profile) {
            let node = map[profile.samples.stack[i] as usize] as usize;
            let vals = &profile.samples.values[i * m..(i + 1) * m];
            for (k, id) in metrics.iter().enumerate() {
                id.fold(&mut self_v[node * m + k], vals[k]);
            }
            samples_in_view += 1;
        }

        // 3. Totals: children after parents, so one reverse pass.
        let mut total_v = self_v.clone();
        let inclusive: Vec<usize> = (0..m).filter(|&k| metrics[k].inclusive()).collect();
        for c in (0..n).rev() {
            let p = parent[c];
            if p != NONE {
                for &k in &inclusive {
                    let v = total_v[c * m + k];
                    let t = &mut total_v[p as usize * m + k];
                    *t = t.saturating_add(v);
                }
            }
        }

        // 4. Children of live nodes (something measured in the view).
        let live = |c: usize| total_v[c * m..(c + 1) * m].iter().any(|&v| v > 0) || self_v[c * m..(c + 1) * m].iter().any(|&v| v > 0);
        let mut counts = vec![0u32; n + 1];
        let mut roots = Vec::new();
        for (c, &p) in parent.iter().enumerate() {
            if !live(c) {
                continue;
            }
            match p {
                NONE => roots.push(c as u32),
                p => counts[p as usize + 1] += 1,
            }
        }
        for i in 0..n {
            counts[i + 1] += counts[i];
        }
        let child_start = counts;
        let mut fill = child_start.clone();
        let mut child_list = vec![0u32; child_start[n] as usize];
        for (c, &p) in parent.iter().enumerate() {
            if p != NONE && live(c) {
                child_list[fill[p as usize] as usize] = c as u32;
                fill[p as usize] += 1;
            }
        }

        // 5. Grand totals.
        let mut grand = vec![0u64; m];
        for (k, id) in metrics.iter().enumerate() {
            if id.inclusive() {
                for &r in &roots {
                    id.fold(&mut grand[k], total_v[r as usize * m + k]);
                }
            } else {
                for c in 0..n {
                    id.fold(&mut grand[k], self_v[c * m + k]);
                }
            }
        }

        CallTree {
            metrics,
            frame,
            parent,
            self_v,
            total_v,
            child_start,
            child_list,
            roots,
            grand,
            samples_in_view,
            frame_nodes: OnceLock::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.frame.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frame.is_empty()
    }

    pub fn m(&self) -> usize {
        self.metrics.len()
    }

    /// Position of a metric, or an error naming what is missing.
    pub fn metric_index(&self, id: MetricId) -> Result<usize> {
        self.metrics
            .iter()
            .position(|m| *m == id)
            .ok_or_else(|| ProfileError::NotAvailable(format!("this profile has no {} data", id.label().to_lowercase())))
    }

    #[inline]
    pub fn self_of(&self, node: u32, k: usize) -> u64 {
        self.self_v[node as usize * self.m() + k]
    }

    #[inline]
    pub fn total_of(&self, node: u32, k: usize) -> u64 {
        self.total_v[node as usize * self.m() + k]
    }

    pub fn self_row(&self, node: u32) -> &[u64] {
        let m = self.m();
        &self.self_v[node as usize * m..(node as usize + 1) * m]
    }

    pub fn total_row(&self, node: u32) -> &[u64] {
        let m = self.m();
        &self.total_v[node as usize * m..(node as usize + 1) * m]
    }

    /// Children of a node (live ones only), unsorted.
    pub fn children(&self, node: u32) -> &[u32] {
        let (a, b) = (
            self.child_start[node as usize] as usize,
            self.child_start[node as usize + 1] as usize,
        );
        &self.child_list[a..b]
    }

    /// The node reached by following `path` (frame ids) from the roots.
    pub fn node_at(&self, path: &[u32]) -> Option<u32> {
        let mut level: &[u32] = &self.roots;
        let mut node = None;
        for &f in path {
            let n = *level.iter().find(|&&c| self.frame[c as usize] == f)?;
            level = self.children(n);
            node = Some(n);
        }
        node
    }

    /// Like [`CallTree::node_at`], with an error that tells the user what happened.
    pub fn resolve(&self, path: &[u32]) -> Result<Option<u32>> {
        if path.is_empty() {
            return Ok(None);
        }
        self.node_at(path)
            .map(Some)
            .ok_or_else(|| ProfileError::invalid("that call path is not in the current view (the filters changed?)"))
    }

    /// Live nodes of a frame (lazily indexed).
    pub fn nodes_of_frame(&self, frame: u32) -> &[u32] {
        let (start, list) = self.frame_nodes.get_or_init(|| {
            let max = self.frame.iter().copied().max().map(|f| f as usize + 1).unwrap_or(0);
            let mut counts = vec![0u32; max + 1];
            let live: Vec<bool> = (0..self.len()).map(|n| self.is_live(n as u32)).collect();
            for (n, &f) in self.frame.iter().enumerate() {
                if live[n] {
                    counts[f as usize + 1] += 1;
                }
            }
            for i in 0..max {
                counts[i + 1] += counts[i];
            }
            let mut fill = counts.clone();
            let mut list = vec![0u32; counts[max] as usize];
            for (n, &f) in self.frame.iter().enumerate() {
                if live[n] {
                    list[fill[f as usize] as usize] = n as u32;
                    fill[f as usize] += 1;
                }
            }
            (counts, list)
        });
        match (start.get(frame as usize), start.get(frame as usize + 1)) {
            (Some(&a), Some(&b)) => &list[a as usize..b as usize],
            _ => &[],
        }
    }

    /// Something was measured at or under this node.
    pub fn is_live(&self, node: u32) -> bool {
        self.total_row(node).iter().any(|&v| v > 0) || self.self_row(node).iter().any(|&v| v > 0)
    }

    /// Depth-first walk over the live tree (no recursion): `visit(node, true)`
    /// before a node's children, `visit(node, false)` after them.
    pub fn walk(&self, mut visit: impl FnMut(u32, bool)) {
        let mut stack: Vec<(u32, usize)> = Vec::new();
        for &r in &self.roots {
            visit(r, true);
            stack.push((r, 0));
            while let Some(top) = stack.last_mut() {
                let (node, i) = *top;
                match self.children(node).get(i) {
                    Some(&c) => {
                        top.1 += 1;
                        visit(c, true);
                        stack.push((c, 0));
                    }
                    None => {
                        visit(node, false);
                        stack.pop();
                    }
                }
            }
        }
    }

    /// Frames from the root to `node`.
    pub fn path_of(&self, mut node: u32) -> Vec<u32> {
        let mut out = Vec::new();
        while node != NONE {
            out.push(self.frame[node as usize]);
            node = self.parent[node as usize];
        }
        out.reverse();
        out
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::api::{JustMyCode, TimeRange};
    use crate::builder::{FrameSpec, ProfileBuilder};
    use crate::limits::Limits;
    use crate::model::{Measurement, MetricInfo, ProfileKind};

    /// A profile from `(thread, time, "a;b;c", samples)` lines; frames named
    /// `fw.*` are framework code. Metrics: samples and time (10 ns per sample).
    pub(crate) fn profile(lines: &[(u32, u64, &str, u64)]) -> Profile {
        let mut b = ProfileBuilder::new(
            "test",
            ProfileKind::Cpu,
            &[
                MetricInfo::new(MetricId::Samples, Measurement::Sampled),
                MetricInfo::new(MetricId::TimeNs, Measurement::Sampled),
            ],
            Limits::default(),
        )
        .unwrap();
        b.set_has_time(true);
        for t in 0..3 {
            b.thread(Some(t), &format!("T{t}")).unwrap();
        }
        for (t, time, stack, n) in lines {
            let mut frames = Vec::new();
            for f in stack.split(';') {
                frames.push(b.frame(FrameSpec::new(f).framework(f.starts_with("fw."))).unwrap());
            }
            let s = b.stack(&frames).unwrap();
            b.sample(*t, *time, s, &[*n, n * 10]).unwrap();
        }
        b.finish().unwrap()
    }

    pub(crate) fn fid(p: &Profile, name: &str) -> u32 {
        p.frames.iter().position(|f| f.function == name).unwrap() as u32
    }

    fn names(p: &Profile, t: &CallTree, nodes: &[u32]) -> Vec<String> {
        let mut v: Vec<String> = nodes
            .iter()
            .map(|&n| p.frames[t.frame[n as usize] as usize].function.clone())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn totals_add_up_and_paths_resolve() {
        let p = profile(&[(0, 0, "main;a;b", 2), (0, 1, "main;a", 1), (1, 2, "main;c", 3), (1, 3, "other", 4)]);
        let t = CallTree::build(&p, &CompiledView::all());
        assert_eq!(t.grand, [10, 100]);
        assert_eq!(t.samples_in_view, 4);
        assert_eq!(names(&p, &t, &t.roots), ["main", "other"]);
        let main = t.node_at(&[fid(&p, "main")]).unwrap();
        assert_eq!(t.total_of(main, 0), 6);
        assert_eq!(t.self_of(main, 0), 0);
        let a = t.node_at(&[fid(&p, "main"), fid(&p, "a")]).unwrap();
        assert_eq!((t.self_of(a, 0), t.total_of(a, 0)), (1, 3));
        assert_eq!(t.path_of(a), [fid(&p, "main"), fid(&p, "a")]);
        assert!(t.node_at(&[fid(&p, "a")]).is_none());
        assert!(t.resolve(&[fid(&p, "a")]).is_err());
        assert_eq!(t.resolve(&[]).unwrap(), None);
        assert_eq!(t.nodes_of_frame(fid(&p, "b")).len(), 1);
        assert!(t.nodes_of_frame(9999).is_empty());
        assert!(t
            .metric_index(MetricId::AllocBytes)
            .unwrap_err()
            .to_string()
            .contains("allocated bytes"));
        assert!(!t.is_empty());
    }

    #[test]
    fn range_and_thread_filters() {
        let p = profile(&[(0, 0, "main;a", 1), (0, 10, "main;b", 1), (1, 20, "main;c", 1)]);
        let view = ProfileView {
            range: Some(TimeRange { from_ns: 5, to_ns: 25 }),
            threads: Some(vec![0]),
            ..ProfileView::default()
        };
        let cv = CompiledView::compile(&p, &view).unwrap();
        let t = CallTree::build(&p, &cv);
        assert_eq!(t.grand[0], 1);
        let main = t.node_at(&[fid(&p, "main")]).unwrap();
        assert_eq!(names(&p, &t, t.children(main)), ["b"], "dead branches are pruned");
        let bad = ProfileView {
            threads: Some(vec![7]),
            ..ProfileView::default()
        };
        assert!(CompiledView::compile(&p, &bad).is_err());
        let backwards = ProfileView {
            range: Some(TimeRange { from_ns: 5, to_ns: 1 }),
            ..ProfileView::default()
        };
        assert!(CompiledView::compile(&p, &backwards).is_err());
    }

    #[test]
    fn just_my_code_collapses_framework_runs() {
        let p = profile(&[
            (0, 0, "main;fw.sort;fw.cmp;user.cb", 2),
            (0, 1, "main;fw.sort;fw.cmp", 3),
            (0, 2, "fw.start;fw.run;main", 1),
        ]);
        let view = ProfileView {
            just_my_code: Some(JustMyCode::default()),
            ..ProfileView::default()
        };
        let t = CallTree::build(&p, &CompiledView::compile(&p, &view).unwrap());
        let sort = t.node_at(&[fid(&p, "main"), fid(&p, "fw.sort")]).unwrap();
        assert_eq!(t.self_of(sort, 0), 3, "fw.cmp's time stays in fw.sort");
        assert_eq!(names(&p, &t, t.children(sort)), ["user.cb"]);
        let start = t.node_at(&[fid(&p, "fw.start"), fid(&p, "main")]).unwrap();
        assert_eq!(t.total_of(start, 0), 1);
        assert_eq!(t.grand[0], 6);
        let off = ProfileView {
            just_my_code: Some(JustMyCode {
                enabled: false,
                ..JustMyCode::default()
            }),
            ..ProfileView::default()
        };
        let t = CallTree::build(&p, &CompiledView::compile(&p, &off).unwrap());
        assert!(t.node_at(&[fid(&p, "main"), fid(&p, "fw.sort"), fid(&p, "fw.cmp")]).is_some());
    }

    #[test]
    fn recursion_folding() {
        let p = profile(&[(0, 0, "a;a;a;b", 1), (0, 1, "a;b;a;c", 1)]);
        let direct = CompiledView {
            recursion: RecursionMode::Direct,
            ..CompiledView::all()
        };
        let t = CallTree::build(&p, &direct);
        let (a, b, c) = (fid(&p, "a"), fid(&p, "b"), fid(&p, "c"));
        assert!(t.node_at(&[a, b]).is_some());
        assert!(t.node_at(&[a, a]).is_none());
        assert!(t.node_at(&[a, b, a, c]).is_some(), "indirect recursion stays");
        let all = CompiledView {
            recursion: RecursionMode::All,
            ..CompiledView::all()
        };
        let t = CallTree::build(&p, &all);
        assert!(t.node_at(&[a, c]).is_some(), "folded back into the ancestor");
        assert_eq!(t.total_of(t.node_at(&[a]).unwrap(), 0), 2);
        assert_eq!(t.grand[0], 2);
    }
}
