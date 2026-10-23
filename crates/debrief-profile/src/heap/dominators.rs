//! Dominators of a heap's object graph (Lengauer–Tarjan with path
//! compression, iterative so deep graphs cannot overflow the stack),
//! retained sizes, retained size per type, and paths to GC roots.
//!
//! A virtual super-root (id `n`) points at every GC root, so the dominator
//! tree has one top. An object `d` dominates `o` when every path from the
//! roots to `o` passes through `d`; `o`'s retained size is the memory freed
//! if its immediate dominator chain let go of it.
//!
//! Reference: T. Lengauer, R. E. Tarjan, "A fast algorithm for finding
//! dominators in a flowgraph", ACM TOPLAS 1(1), 1979.

use super::model::ObjectGraph;
use crate::model::NONE;

/// Results of analysing an object graph. Index `n` (= `graph.len()`) is the super-root.
#[derive(Debug, Clone)]
pub struct HeapAnalysis {
    /// Immediate dominator of each object: `n` for objects only the super-root dominates, [`NONE`] if unreachable.
    pub idom: Vec<u32>,
    /// Retained size (bytes) of each object and, at `n`, of the whole reachable heap.
    pub retained: Vec<u64>,
    /// Dominator tree children (CSR over `0..=n`), sorted by retained size, descending.
    dom_start: Vec<u64>,
    dom_list: Vec<u32>,
    /// Retained size per type (objects of the type not dominated by another of the same type).
    pub type_retained: Vec<u64>,
    /// Predecessors of each object (CSR over `0..=n`; the super-root's edges included).
    pred_start: Vec<u64>,
    pred_list: Vec<u32>,
    pub is_root: Vec<bool>,
    pub reachable: u64,
}

fn csr(n: usize, pairs: impl Iterator<Item = (u32, u32)> + Clone) -> (Vec<u64>, Vec<u32>) {
    let mut start = vec![0u64; n + 1];
    for (a, _) in pairs.clone() {
        start[a as usize + 1] += 1;
    }
    for i in 0..n {
        start[i + 1] += start[i];
    }
    let mut fill = start.clone();
    let mut list = vec![0u32; start[n] as usize];
    for (a, b) in pairs {
        list[fill[a as usize] as usize] = b;
        fill[a as usize] += 1;
    }
    (start, list)
}

impl HeapAnalysis {
    /// Analyses `graph` (time O(E α(E, N)), memory O(N + E)).
    pub fn new(graph: &ObjectGraph, types: usize) -> HeapAnalysis {
        let n = graph.len();
        let total = n + 1;
        let root = n as u32;
        let succ = |v: u32| -> &[u32] {
            if v == root {
                &graph.roots
            } else {
                let (a, b) = (graph.edge_start[v as usize] as usize, graph.edge_start[v as usize + 1] as usize);
                &graph.edge_to[a..b]
            }
        };

        // Depth-first preorder from the super-root.
        let mut dfnum = vec![NONE; total];
        let mut vertex: Vec<u32> = Vec::with_capacity(total);
        let mut parent = vec![NONE; total];
        let mut stack: Vec<(u32, usize)> = vec![(root, 0)];
        dfnum[root as usize] = 0;
        vertex.push(root);
        while let Some(top) = stack.last_mut() {
            let (v, i) = *top;
            match succ(v).get(i) {
                Some(&w) => {
                    top.1 += 1;
                    if dfnum[w as usize] == NONE {
                        dfnum[w as usize] = vertex.len() as u32;
                        vertex.push(w);
                        parent[w as usize] = v;
                        stack.push((w, 0));
                    }
                }
                None => {
                    stack.pop();
                }
            }
        }
        let reached = vertex.len();

        // Predecessors among reachable objects.
        let edges = || {
            (0..total as u32)
                .filter(|&v| dfnum[v as usize] != NONE)
                .flat_map(move |v| succ(v).iter().map(move |&w| (w, v)))
        };
        let (pred_start, pred_list) = csr(total, edges());

        // Lengauer–Tarjan (simple version: path compression, no balancing).
        let mut semi: Vec<u32> = dfnum.clone();
        let mut label: Vec<u32> = (0..total as u32).collect();
        let mut ancestor = vec![NONE; total];
        let mut idom = vec![NONE; total];
        let mut bucket_head = vec![NONE; total];
        let mut bucket_next = vec![NONE; total];
        let mut chain: Vec<u32> = Vec::new();
        let mut eval = |v: u32, ancestor: &mut Vec<u32>, label: &mut Vec<u32>, semi: &Vec<u32>| -> u32 {
            if ancestor[v as usize] == NONE {
                return v;
            }
            chain.clear();
            let mut x = v;
            while ancestor[ancestor[x as usize] as usize] != NONE {
                chain.push(x);
                x = ancestor[x as usize];
            }
            for &x in chain.iter().rev() {
                let a = ancestor[x as usize];
                if semi[label[a as usize] as usize] < semi[label[x as usize] as usize] {
                    label[x as usize] = label[a as usize];
                }
                ancestor[x as usize] = ancestor[a as usize];
            }
            label[v as usize]
        };
        for i in (1..reached).rev() {
            let w = vertex[i];
            let (a, b) = (pred_start[w as usize] as usize, pred_start[w as usize + 1] as usize);
            for &v in &pred_list[a..b] {
                let u = eval(v, &mut ancestor, &mut label, &semi);
                if semi[u as usize] < semi[w as usize] {
                    semi[w as usize] = semi[u as usize];
                }
            }
            let s = vertex[semi[w as usize] as usize];
            bucket_next[w as usize] = bucket_head[s as usize];
            bucket_head[s as usize] = w;
            let p = parent[w as usize];
            ancestor[w as usize] = p;
            let mut v = std::mem::replace(&mut bucket_head[p as usize], NONE);
            while v != NONE {
                let next = bucket_next[v as usize];
                let u = eval(v, &mut ancestor, &mut label, &semi);
                idom[v as usize] = if semi[u as usize] < semi[v as usize] { u } else { p };
                v = next;
            }
        }
        for &w in &vertex[1..reached] {
            if idom[w as usize] != vertex[semi[w as usize] as usize] {
                idom[w as usize] = idom[idom[w as usize] as usize];
            }
        }
        idom[root as usize] = root;

        // Retained sizes: children after their dominator in preorder.
        let mut retained: Vec<u64> = (0..total).map(|v| if v < n { u64::from(graph.size[v]) } else { 0 }).collect();
        for &w in vertex[1..reached].iter().rev() {
            let d = idom[w as usize] as usize;
            retained[d] = retained[d].saturating_add(retained[w as usize]);
        }

        // Dominator tree children, heaviest first.
        let (dom_start, mut dom_list) = csr(total, vertex[1..reached].iter().map(|&w| (idom[w as usize], w)));
        for v in 0..total {
            let (a, b) = (dom_start[v] as usize, dom_start[v + 1] as usize);
            dom_list[a..b].sort_by(|&x, &y| retained[y as usize].cmp(&retained[x as usize]).then(x.cmp(&y)));
        }

        // Retained per type: count an object only when no dominator above it has its type.
        let mut type_retained = vec![0u64; types];
        let mut on_path = vec![0u32; types];
        let mut walk: Vec<(u32, usize)> = vec![(root, 0)];
        while let Some(top) = walk.last_mut() {
            let (v, i) = *top;
            let kids = &dom_list[dom_start[v as usize] as usize..dom_start[v as usize + 1] as usize];
            match kids.get(i) {
                Some(&c) => {
                    top.1 += 1;
                    if let Some(&t) = graph.type_of.get(c as usize) {
                        let t = t as usize;
                        if t < types {
                            if on_path[t] == 0 {
                                type_retained[t] = type_retained[t].saturating_add(retained[c as usize]);
                            }
                            on_path[t] += 1;
                        }
                    }
                    walk.push((c, 0));
                }
                None => {
                    walk.pop();
                    if let Some(&t) = graph.type_of.get(v as usize) {
                        if (t as usize) < types {
                            on_path[t as usize] -= 1;
                        }
                    }
                }
            }
        }

        let mut is_root = vec![false; n];
        for &r in &graph.roots {
            is_root[r as usize] = true;
        }
        idom.truncate(total);
        HeapAnalysis {
            idom,
            retained,
            dom_start,
            dom_list,
            type_retained,
            pred_start,
            pred_list,
            is_root,
            reachable: reached.saturating_sub(1) as u64,
        }
    }

    /// Objects `v` immediately dominates (`v = n` for the top level), heaviest first.
    pub fn dominated(&self, v: u32) -> &[u32] {
        match (self.dom_start.get(v as usize), self.dom_start.get(v as usize + 1)) {
            (Some(&a), Some(&b)) => &self.dom_list[a as usize..b as usize],
            _ => &[],
        }
    }

    /// Objects (or the super-root) referencing `v`.
    pub fn referrers(&self, v: u32) -> &[u32] {
        match (self.pred_start.get(v as usize), self.pred_start.get(v as usize + 1)) {
            (Some(&a), Some(&b)) => &self.pred_list[a as usize..b as usize],
            _ => &[],
        }
    }

    pub fn is_reachable(&self, v: u32) -> bool {
        self.idom.get(v as usize).is_some_and(|&d| d != NONE)
    }
}

/// Paths from GC roots to `target`, shortest first, each ending at a
/// different root: a breadth-first search over referrers, bounded by
/// `max_depth` steps and `max_visits` objects. Returns the paths (objects,
/// root first) and whether the search hit its visit limit.
pub fn paths_to_roots(
    graph: &ObjectGraph,
    analysis: &HeapAnalysis,
    target: u32,
    max_paths: usize,
    max_depth: usize,
    max_visits: usize,
) -> (Vec<Vec<u32>>, bool) {
    let n = graph.len() as u32;
    let mut next: Vec<u32> = vec![NONE; graph.len()];
    let mut depth: Vec<u32> = vec![0; graph.len()];
    let mut seen = vec![false; graph.len()];
    let mut queue = std::collections::VecDeque::new();
    let mut paths = Vec::new();
    let mut visits = 0usize;
    seen[target as usize] = true;
    queue.push_back(target);
    while let Some(v) = queue.pop_front() {
        if analysis.is_root[v as usize] {
            let mut path = vec![v];
            let mut x = v;
            while x != target {
                x = next[x as usize];
                path.push(x);
            }
            paths.push(path);
            if paths.len() >= max_paths {
                return (paths, false);
            }
        }
        if depth[v as usize] as usize + 1 >= max_depth {
            continue;
        }
        for &u in analysis.referrers(v) {
            if u == n || seen[u as usize] {
                continue;
            }
            visits += 1;
            if visits > max_visits {
                return (paths, true);
            }
            seen[u as usize] = true;
            next[u as usize] = v;
            depth[u as usize] = depth[v as usize] + 1;
            queue.push_back(u);
        }
    }
    (paths, false)
}

/// The label of the first reference from `from` to `to`.
pub fn edge_label(graph: &ObjectGraph, from: u32, to: u32) -> Option<String> {
    graph
        .edges(from)
        .find(|&(t, _)| t == to)
        .and_then(|(_, l)| graph.label(l).map(str::to_string))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::heap::model::HeapBuilder;
    use crate::limits::Limits;

    /// A graph from `(type, size, [edges])` objects and root ids.
    pub(crate) fn graph(objects: &[(&str, u64, &[u32])], roots: &[u32]) -> crate::heap::HeapSnapshot {
        let mut b = HeapBuilder::new("t", Limits::default());
        for (t, size, edges) in objects {
            let ty = b.type_id(t).unwrap();
            b.object(ty, *size).unwrap();
            for (i, &e) in edges.iter().enumerate() {
                b.edge(e, Some(&format!("f{i}"))).unwrap();
            }
        }
        for &r in roots {
            b.root(r);
        }
        b.finish().unwrap()
    }

    /// Naive dominators by definition: d dominates v if v is unreachable when d is removed.
    fn naive_idom(g: &ObjectGraph) -> Vec<u32> {
        let n = g.len();
        let reach = |skip: Option<u32>| {
            let mut seen = vec![false; n];
            let mut stack: Vec<u32> = g.roots.iter().copied().filter(|&r| Some(r) != skip).collect();
            for &r in &stack {
                seen[r as usize] = true;
            }
            while let Some(v) = stack.pop() {
                for (w, _) in g.edges(v) {
                    if Some(w) != skip && !seen[w as usize] {
                        seen[w as usize] = true;
                        stack.push(w);
                    }
                }
            }
            seen
        };
        let all = reach(None);
        let doms: Vec<Vec<bool>> = (0..n as u32).map(|d| reach(Some(d))).collect();
        (0..n)
            .map(|v| {
                if !all[v] {
                    return NONE;
                }
                // Strict dominators of v; the immediate one is dominated by all the others.
                let ds: Vec<u32> = (0..n as u32).filter(|&d| d as usize != v && !doms[d as usize][v]).collect();
                ds.iter()
                    .copied()
                    .find(|&d| ds.iter().all(|&o| o == d || !doms[o as usize][d as usize]))
                    .unwrap_or(n as u32)
            })
            .collect()
    }

    #[test]
    fn classic_graph() {
        // root -> a; a -> b, c; b -> d; c -> d; d -> e; e -> b (cycle); f unreachable.
        let h = graph(
            &[
                ("A", 10, &[1, 2]),
                ("B", 5, &[3]),
                ("C", 5, &[3]),
                ("D", 1, &[4]),
                ("E", 2, &[1]),
                ("F", 7, &[0]),
            ],
            &[0],
        );
        let g = h.graph.as_ref().unwrap();
        let a = HeapAnalysis::new(g, h.types.len());
        assert_eq!(&a.idom[..6], &[6, 0, 0, 0, 3, NONE]);
        assert_eq!(a.idom, {
            let mut v = naive_idom(g);
            v.push(6);
            v
        });
        assert_eq!(a.retained[0], 23);
        assert_eq!(a.retained[3], 3);
        assert_eq!(a.retained[5], 7, "unreachable keeps its own size");
        assert_eq!(a.retained[6], 23, "the super-root retains the reachable heap");
        assert_eq!(a.dominated(6), &[0]);
        assert_eq!(a.dominated(0), &[1, 2, 3], "heaviest first, ties by id");
        assert!(a.dominated(99).is_empty());
        assert!(!a.is_reachable(5));
        assert_eq!(a.reachable, 5);
        let ta: Vec<u64> = a.type_retained.clone();
        assert_eq!(ta[0], 23);
        assert_eq!(ta[5], 0, "unreachable types retain nothing");
        assert_eq!(a.referrers(1), &[0, 4]);
    }

    #[test]
    fn same_type_nesting_counts_once_and_paths() {
        // root -> n0 (List) -> n1 (List) -> n2 (Item); second root n3 -> n2.
        let h = graph(
            &[("List", 10, &[1]), ("List", 10, &[2]), ("Item", 1, &[]), ("Holder", 1, &[2])],
            &[0, 3],
        );
        let g = h.graph.as_ref().unwrap();
        let a = HeapAnalysis::new(g, h.types.len());
        assert_eq!(a.type_retained[0], 20, "the inner list is inside the outer one");
        assert_eq!(a.idom[2], 4, "two roots reach the item: only the super-root dominates it");
        let (paths, truncated) = paths_to_roots(g, &a, 2, 5, 10, 1000);
        assert!(!truncated);
        assert_eq!(paths, vec![vec![3, 2], vec![0, 1, 2]], "shortest first");
        assert_eq!(edge_label(g, 1, 2).as_deref(), Some("f0"));
        assert_eq!(edge_label(g, 2, 1), None);
        let (one, _) = paths_to_roots(g, &a, 2, 1, 10, 1000);
        assert_eq!(one.len(), 1);
        let (short, _) = paths_to_roots(g, &a, 2, 5, 2, 1000);
        assert_eq!(short, vec![vec![3, 2]], "depth limit");
        let (_, truncated) = paths_to_roots(g, &a, 2, 5, 10, 1);
        assert!(truncated);
    }

    #[test]
    fn matches_the_definition_on_pseudo_random_graphs() {
        let mut seed = 12345u64;
        let mut rnd = |m: u64| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) % m
        };
        for _ in 0..200 {
            let n = 1 + rnd(25) as usize;
            let mut objects: Vec<(String, u64, Vec<u32>)> = Vec::new();
            for _ in 0..n {
                let edges: Vec<u32> = (0..rnd(4)).map(|_| rnd(n as u64) as u32).collect();
                objects.push((format!("T{}", rnd(3)), 1 + rnd(9), edges));
            }
            let roots: Vec<u32> = (0..1 + rnd(3)).map(|_| rnd(n as u64) as u32).collect();
            let refs: Vec<(&str, u64, &[u32])> = objects.iter().map(|(t, s, e)| (t.as_str(), *s, e.as_slice())).collect();
            let h = graph(&refs, &roots);
            let g = h.graph.as_ref().unwrap();
            let a = HeapAnalysis::new(g, h.types.len());
            let mut expect = naive_idom(g);
            expect.push(n as u32);
            assert_eq!(a.idom, expect, "{objects:?} roots {roots:?}");
            // Retained of the super-root is the reachable heap.
            let reachable: u64 = (0..n).filter(|&v| a.is_reachable(v as u32)).map(|v| u64::from(g.size[v])).sum();
            assert_eq!(a.retained[n], reachable);
        }
    }

    #[test]
    fn deep_chains_do_not_overflow() {
        let n = 200_000u32;
        let mut b = HeapBuilder::new("t", Limits::default());
        let t = b.type_id("Node").unwrap();
        for i in 0..n {
            b.object(t, 1).unwrap();
            if i + 1 < n {
                b.edge(i + 1, None).unwrap();
            }
        }
        b.root(0);
        let h = b.finish().unwrap();
        let a = HeapAnalysis::new(h.graph.as_ref().unwrap(), 1);
        assert_eq!(a.retained[0], u64::from(n));
        assert_eq!(a.idom[(n - 1) as usize], n - 2);
        assert_eq!(a.type_retained[0], u64::from(n));
    }
}
