//! The V8 CPU profile node tree (`Profiler.ProfileNode`): nodes with a call
//! frame, linked by `children` (`.cpuprofile`) or `parent` (trace chunks).
//!
//! [`NodeStacks::resolve`] validates the tree (unique ids, known parents and
//! children, one parent per node, no cycles) and turns every node into a
//! builder stack without recursion, memoising each node's stack. The
//! synthetic `(root)` node is dropped; `(program)`, `(idle)` and
//! `(garbage collector)` stay as native frames (`(program)` and `(idle)` are
//! framework code, garbage collection is not, so findings can see it).

use debrief_profile::frames::basename;
use debrief_profile::{FrameKind, FrameSpec, ProfileBuilder, ProfileError, Result, NONE};
use serde::Deserialize;
use std::collections::HashMap;

/// A profile node as both formats write it.
#[derive(Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Node {
    pub id: u64,
    #[serde(default)]
    pub call_frame: CallFrame,
    #[serde(default)]
    pub hit_count: Option<u64>,
    #[serde(default)]
    pub children: Vec<u64>,
    #[serde(default)]
    pub parent: Option<u64>,
}

/// `Runtime.CallFrame` (`scriptId`, `columnNumber`, `codeType` are not used).
#[derive(Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CallFrame {
    #[serde(default)]
    pub function_name: String,
    #[serde(default)]
    pub url: String,
    /// 0-based; -1 when unknown.
    #[serde(default)]
    pub line_number: Option<f64>,
}

/// Interns the frame of a V8 call frame.
pub(crate) fn frame(b: &mut ProfileBuilder, cf: &CallFrame) -> Result<u32> {
    let name = cf.function_name.as_str();
    match name {
        "(program)" | "(idle)" => return b.frame(FrameSpec::new(name).kind(FrameKind::Native).framework(true)),
        "(garbage collector)" => return b.frame(FrameSpec::new(name).kind(FrameKind::Native).framework(false)),
        _ => {}
    }
    let url = cf.url.as_str();
    let line = cf
        .line_number
        .filter(|l| l.is_finite() && *l >= 0.0 && *l < f64::from(u32::MAX))
        .map(|l| l as u32 + 1);
    let module = match basename(url) {
        "" => url,
        base => base,
    };
    let mut spec = FrameSpec::new(if name.is_empty() { "(anonymous)" } else { name })
        .module(Some(module))
        .file(Some(url))
        .line(line)
        .kind(if url.is_empty() { FrameKind::Native } else { FrameKind::Managed });
    if url.starts_with("node:") || url.starts_with("internal/") || url.contains("/node_modules/") {
        spec = spec.framework(true);
    }
    b.frame(spec)
}

/// Builder stacks of every node of one profile.
pub(crate) struct NodeStacks {
    index: HashMap<u64, usize>,
    /// Builder stack per node; [`NONE`] for the dropped `(root)`.
    stacks: Vec<u32>,
    /// Nodes cut because they were deeper than the stack depth limit.
    pub truncated: u64,
}

impl NodeStacks {
    /// Validates `nodes` and interns their stacks.
    pub(crate) fn resolve(format: &'static str, nodes: &[Node], b: &mut ProfileBuilder) -> Result<NodeStacks> {
        let mut index = HashMap::with_capacity(nodes.len());
        for (i, n) in nodes.iter().enumerate() {
            if index.insert(n.id, i).is_some() {
                return Err(ProfileError::malformed(format, format!("profile node id {} appears twice", n.id)));
            }
        }
        let find = |id: u64, what: &str| {
            index
                .get(&id)
                .copied()
                .ok_or_else(|| ProfileError::malformed(format, format!("{what} refers to profile node {id}, which does not exist")))
        };
        let mut parent: Vec<Option<usize>> = vec![None; nodes.len()];
        for (i, n) in nodes.iter().enumerate() {
            if let Some(p) = n.parent {
                let p = find(p, "a node's parent")?;
                set_parent(format, &mut parent, nodes, i, p)?;
            }
            for &c in &n.children {
                let c = find(c, "a node's children list")?;
                set_parent(format, &mut parent, nodes, c, i)?;
            }
        }

        let max_depth = b.limits().max_stack_depth;
        let mut stacks = vec![NONE; nodes.len()];
        let mut depth = vec![0u32; nodes.len()];
        // 0 = not visited, 1 = on the current path, 2 = resolved.
        let mut state = vec![0u8; nodes.len()];
        let mut path = Vec::new();
        let mut truncated = 0;
        for start in 0..nodes.len() {
            path.clear();
            let mut cur = start;
            loop {
                match state[cur] {
                    2 => break,
                    1 => {
                        return Err(ProfileError::malformed(
                            format,
                            format!("profile node {} is its own ancestor", nodes[cur].id),
                        ))
                    }
                    _ => {}
                }
                state[cur] = 1;
                path.push(cur);
                match parent[cur] {
                    Some(p) => cur = p,
                    None => break,
                }
            }
            for &i in path.iter().rev() {
                let (parent_stack, parent_depth) = parent[i].map_or((NONE, 0), |p| (stacks[p], depth[p]));
                let is_root = parent[i].is_none() && nodes[i].call_frame.function_name == "(root)";
                if is_root {
                    stacks[i] = NONE;
                    depth[i] = 0;
                } else if parent_depth >= max_depth {
                    truncated += 1;
                    stacks[i] = parent_stack;
                    depth[i] = parent_depth;
                } else {
                    let f = frame(b, &nodes[i].call_frame)?;
                    stacks[i] = b.push(parent_stack, f)?;
                    depth[i] = parent_depth + 1;
                }
                state[i] = 2;
            }
        }
        Ok(NodeStacks { index, stacks, truncated })
    }

    /// The stack of a sampled node (`(no stack)` for the root).
    pub(crate) fn stack(&self, format: &'static str, id: u64, b: &mut ProfileBuilder) -> Result<u32> {
        let i = *self
            .index
            .get(&id)
            .ok_or_else(|| ProfileError::malformed(format, format!("a sample refers to profile node {id}, which does not exist")))?;
        match self.stacks[i] {
            NONE => b.empty_stack(),
            s => Ok(s),
        }
    }

    /// Node ids with their hit counts (old profiles without a `samples` array).
    pub(crate) fn hit_counts(nodes: &[Node]) -> impl Iterator<Item = (u64, u64)> + '_ {
        nodes.iter().filter_map(|n| n.hit_count.filter(|&h| h > 0).map(|h| (n.id, h)))
    }
}

fn set_parent(format: &'static str, parent: &mut [Option<usize>], nodes: &[Node], child: usize, p: usize) -> Result<()> {
    match parent[child] {
        Some(old) if old != p => Err(ProfileError::malformed(
            format,
            format!(
                "profile node {} has two parents ({} and {})",
                nodes[child].id, nodes[old].id, nodes[p].id
            ),
        )),
        _ => {
            parent[child] = Some(p);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use debrief_profile::{Limits, Measurement, MetricId, MetricInfo, ProfileKind};

    fn builder(limits: Limits) -> ProfileBuilder {
        ProfileBuilder::new(
            "t",
            ProfileKind::Cpu,
            &[MetricInfo::new(MetricId::Samples, Measurement::Sampled)],
            limits,
        )
        .unwrap()
    }

    fn node(id: u64, name: &str, children: &[u64]) -> Node {
        Node {
            id,
            call_frame: CallFrame {
                function_name: name.into(),
                url: "file:///srv/app.js".into(),
                line_number: Some(9.0),
            },
            children: children.to_vec(),
            ..Node::default()
        }
    }

    fn names(mut b: ProfileBuilder, stack: u32) -> Vec<String> {
        let t = b.thread(None, "x").unwrap();
        b.sample(t, 0, stack, &[1]).unwrap();
        let p = b.finish().unwrap();
        p.stack_frames(stack)
            .iter()
            .map(|&f| p.frames[f as usize].function.clone())
            .collect()
    }

    #[test]
    fn resolves_children_and_parents() {
        let mut b = builder(Limits::default());
        let mut nodes = vec![node(1, "(root)", &[2]), node(2, "main", &[3]), node(3, "", &[])];
        nodes.push(Node {
            parent: Some(3),
            ..node(4, "leaf", &[])
        });
        let s = NodeStacks::resolve("t", &nodes, &mut b).unwrap();
        assert_eq!(s.truncated, 0);
        let leaf = s.stack("t", 4, &mut b).unwrap();
        let root = s.stack("t", 1, &mut b).unwrap();
        assert!(s.stack("t", 99, &mut b).unwrap_err().to_string().contains("node 99"));
        assert_ne!(leaf, root);
        assert_eq!(names(b, leaf), ["main", "(anonymous)", "leaf"]);
    }

    #[test]
    fn frames_follow_devtools_conventions() {
        let mut b = builder(Limits::default());
        let mk = |name: &str, url: &str| CallFrame {
            function_name: name.into(),
            url: url.into(),
            line_number: Some(-1.0),
        };
        let gc = frame(&mut b, &mk("(garbage collector)", "")).unwrap();
        let idle = frame(&mut b, &mk("(idle)", "")).unwrap();
        let internal = frame(&mut b, &mk("emit", "node:internal/events")).unwrap();
        let app = frame(
            &mut b,
            &CallFrame {
                line_number: Some(41.0),
                ..mk("saveOrder", "file:///srv/orders/app.js")
            },
        )
        .unwrap();
        let builtin = frame(&mut b, &mk("JSON.stringify", "")).unwrap();
        let dir = frame(&mut b, &mk("f", "http://x/")).unwrap();
        let t = b.thread(None, "m").unwrap();
        let s = b.push(NONE, gc).unwrap();
        b.sample(t, 0, s, &[1]).unwrap();
        let p = b.finish().unwrap();
        let f = |i: u32| &p.frames[i as usize];
        assert!(!f(gc).is_framework && f(gc).kind == FrameKind::Native);
        assert!(f(idle).is_framework);
        assert!(f(internal).is_framework);
        assert_eq!(f(internal).module.as_deref(), Some("events"));
        assert!(!f(app).is_framework);
        assert_eq!(
            (f(app).module.as_deref(), f(app).line, f(app).kind),
            (Some("app.js"), Some(42), FrameKind::Managed)
        );
        assert_eq!(
            (f(builtin).kind, f(builtin).line, f(builtin).module.as_deref()),
            (FrameKind::Native, None, None)
        );
        assert_eq!(f(dir).module.as_deref(), Some("http://x/"));
    }

    #[test]
    fn rejects_bad_trees() {
        let err = |nodes: Vec<Node>| {
            let mut b = builder(Limits::default());
            NodeStacks::resolve("t", &nodes, &mut b).err().unwrap().to_string()
        };
        assert!(err(vec![node(1, "a", &[]), node(1, "b", &[])]).contains("appears twice"));
        assert!(err(vec![node(1, "a", &[5])]).contains("does not exist"));
        assert!(err(vec![node(1, "a", &[2]), node(2, "b", &[1])]).contains("own ancestor"));
        assert!(err(vec![node(1, "a", &[1])]).contains("own ancestor"));
        assert!(err(vec![node(1, "a", &[3]), node(2, "b", &[3]), node(3, "c", &[])]).contains("two parents"));
        let orphan = Node {
            parent: Some(9),
            ..node(1, "a", &[])
        };
        assert!(err(vec![orphan]).contains("parent"));
    }

    #[test]
    fn deep_chains_are_cut_without_recursion() {
        let n = 50_000u64;
        let nodes: Vec<Node> = (0..n)
            .map(|i| Node {
                parent: i.checked_sub(1),
                ..node(i, &format!("f{i}"), &[])
            })
            .collect();
        let limits = Limits {
            max_stack_depth: 100,
            ..Limits::default()
        };
        let mut b = builder(limits);
        let s = NodeStacks::resolve("t", &nodes, &mut b).unwrap();
        assert_eq!(s.truncated, n - 100);
        let deepest = s.stack("t", n - 1, &mut b).unwrap();
        assert_eq!(names(b, deepest).len(), 100);
        let hits: Vec<_> = NodeStacks::hit_counts(&[
            Node {
                hit_count: Some(3),
                ..node(1, "a", &[])
            },
            node(2, "b", &[]),
        ])
        .collect();
        assert_eq!(hits, vec![(1, 3)]);
    }
}
