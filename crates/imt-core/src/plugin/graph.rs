//! Dependency resolution: a pure function from installed plugins (their
//! manifests and health) to an activation order and a state per plugin.
//!
//! * Cycles over required dependencies are rejected: every plugin on one is
//!   inactive with the cycle named (`dependency cycle: a → b → a`).
//! * A required dependency that is missing, has an incompatible version, is
//!   disabled, quarantined or itself inactive leaves the dependent inactive
//!   with a stated reason; inactivity cascades transitively.
//! * Optional dependencies only order activation and add warnings.

use super::manifest::Dep;
use semver::Version;
use std::collections::{HashMap, HashSet, VecDeque};

/// Longest reason kept; long dependency chains are cut.
const MAX_REASON: usize = 1000;

/// One installed plugin, as the resolver sees it.
#[derive(Debug, Clone)]
pub(crate) struct Node<'a> {
    pub id: &'a str,
    pub version: &'a Version,
    pub requires: &'a [Dep],
    pub disabled: bool,
    pub quarantined: bool,
    /// Registration or activation failed: inactive with this reason.
    pub failure: Option<&'a str>,
}

/// The resolved state of one plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum State {
    Active,
    Disabled,
    Quarantined,
    Inactive(String),
}

#[derive(Debug, Default)]
pub(crate) struct Resolution {
    /// Node indexes, dependencies before their dependents.
    pub order: Vec<usize>,
    pub states: Vec<State>,
    /// Unavailable optional dependencies, per node.
    pub warnings: Vec<Vec<String>>,
    /// Ids of the plugins that require each node (required dependencies only).
    pub dependents: Vec<Vec<String>>,
}

pub(crate) fn resolve(nodes: &[Node]) -> Resolution {
    let index: HashMap<&str, usize> = nodes.iter().enumerate().map(|(i, n)| (n.id, i)).collect();
    let required: Vec<Vec<usize>> = nodes
        .iter()
        .map(|n| {
            n.requires
                .iter()
                .filter(|d| !d.optional)
                .filter_map(|d| index.get(d.id.as_str()).copied())
                .collect()
        })
        .collect();
    // Optional edges order activation too, unless the target itself requires
    // this plugin (then the required direction wins).
    let all: Vec<Vec<usize>> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            n.requires
                .iter()
                .filter_map(|d| index.get(d.id.as_str()).map(|&j| (d, j)))
                .filter(|(d, j)| !d.optional || !reaches(&required, *j, i))
                .map(|(_, j)| j)
                .collect()
        })
        .collect();
    let cycles = cycles(nodes, &required);
    let order = topo_order(&all);

    let mut states: Vec<Option<State>> = vec![None; nodes.len()];
    let mut warnings = vec![Vec::new(); nodes.len()];
    for &i in &order {
        let n = &nodes[i];
        let state = if n.disabled {
            State::Disabled
        } else if n.quarantined {
            State::Quarantined
        } else if let Some(cycle) = &cycles[i] {
            State::Inactive(cycle.clone())
        } else if let Some(f) = n.failure {
            State::Inactive(f.to_string())
        } else {
            let mut problems = Vec::new();
            for d in n.requires {
                let problem = dep_problem(d, index.get(d.id.as_str()).map(|&j| (nodes[j].version, states[j].as_ref())));
                match (problem, d.optional) {
                    (Some(p), false) => problems.push(p),
                    (Some(p), true) => warnings[i].push(format!("optional: {p}")),
                    (None, _) => {}
                }
            }
            if problems.is_empty() {
                State::Active
            } else {
                State::Inactive(cut(problems.join("; ")))
            }
        };
        states[i] = Some(state);
    }
    let mut dependents = vec![Vec::new(); nodes.len()];
    for (i, deps) in required.iter().enumerate() {
        for &j in deps {
            dependents[j].push(nodes[i].id.to_string());
        }
    }
    Resolution {
        order,
        states: states.into_iter().map(|s| s.unwrap_or(State::Active)).collect(),
        warnings,
        dependents,
    }
}

/// Why `dep` is not usable; `found` is the installed version and its state
/// (`None` while it is not resolved yet, only possible across a cycle).
fn dep_problem(dep: &Dep, found: Option<(&Version, Option<&State>)>) -> Option<String> {
    let id = &dep.id;
    let Some((version, state)) = found else {
        return Some(format!("needs {id} {} (not installed)", dep.req));
    };
    if !dep.req.matches(version) {
        return Some(format!("needs {id} {} (installed {version})", dep.req));
    }
    match state {
        Some(State::Active) => None,
        Some(State::Disabled) => Some(format!("needs {id} which is disabled")),
        Some(State::Quarantined) => Some(format!("needs {id} which is quarantined")),
        Some(State::Inactive(reason)) => Some(format!("needs {id} which is inactive ({reason})")),
        None => Some(format!("needs {id} which depends on this plugin")),
    }
}

fn cut(mut s: String) -> String {
    if s.chars().count() > MAX_REASON {
        s = s.chars().take(MAX_REASON).collect::<String>() + "…";
    }
    s
}

/// For every node on a cycle of required dependencies, the cycle through it.
fn cycles(nodes: &[Node], edges: &[Vec<usize>]) -> Vec<Option<String>> {
    let mut out = vec![None; nodes.len()];
    for start in 0..nodes.len() {
        // Shortest path start → … → start (breadth first), if any.
        let mut prev: HashMap<usize, usize> = HashMap::new();
        let mut queue: VecDeque<usize> = VecDeque::from([start]);
        let mut found = false;
        while let Some(n) = queue.pop_front() {
            for &m in &edges[n] {
                if let std::collections::hash_map::Entry::Vacant(e) = prev.entry(m) {
                    e.insert(n);
                    queue.push_back(m);
                }
            }
            if prev.contains_key(&start) {
                found = true;
                break;
            }
        }
        if !found {
            continue;
        }
        let mut path = vec![start];
        let mut at = prev[&start];
        while at != start && path.len() <= nodes.len() {
            path.push(at);
            at = prev[&at];
        }
        path.push(start);
        path.reverse();
        let names: Vec<&str> = path.iter().map(|&i| nodes[i].id).collect();
        out[start] = Some(format!("dependency cycle: {}", names.join(" → ")));
    }
    out
}

/// Whether `to` is reachable from `from` along `edges`.
fn reaches(edges: &[Vec<usize>], from: usize, to: usize) -> bool {
    let mut seen = HashSet::from([from]);
    let mut stack = vec![from];
    while let Some(n) = stack.pop() {
        if n == to {
            return true;
        }
        stack.extend(edges[n].iter().copied().filter(|m| seen.insert(*m)));
    }
    false
}

/// Dependencies before dependents; edges that would close a cycle are
/// ignored, ties keep install order.
fn topo_order(edges: &[Vec<usize>]) -> Vec<usize> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        New,
        Open,
        Done,
    }
    let mut mark = vec![Mark::New; edges.len()];
    let mut order = Vec::with_capacity(edges.len());
    for root in 0..edges.len() {
        if mark[root] != Mark::New {
            continue;
        }
        // Iterative depth-first search: (node, next edge to follow).
        let mut stack = vec![(root, 0usize)];
        mark[root] = Mark::Open;
        while let Some((n, next)) = stack.last_mut() {
            let n = *n;
            if let Some(&m) = edges[n].get(*next) {
                *next += 1;
                if mark[m] == Mark::New {
                    mark[m] = Mark::Open;
                    stack.push((m, 0));
                }
            } else {
                mark[n] = Mark::Done;
                order.push(n);
                stack.pop();
            }
        }
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;
    use semver::VersionReq;

    struct Spec {
        id: &'static str,
        version: Version,
        requires: Vec<Dep>,
        disabled: bool,
        quarantined: bool,
        failure: Option<&'static str>,
    }

    fn spec(id: &'static str, version: &str, deps: &[(&str, &str, bool)]) -> Spec {
        Spec {
            id,
            version: Version::parse(version).unwrap(),
            requires: deps
                .iter()
                .map(|(d, r, optional)| Dep {
                    id: d.to_string(),
                    req: VersionReq::parse(r).unwrap(),
                    optional: *optional,
                })
                .collect(),
            disabled: false,
            quarantined: false,
            failure: None,
        }
    }

    fn run(specs: &[Spec]) -> (Resolution, Vec<&'static str>) {
        let nodes: Vec<Node> = specs
            .iter()
            .map(|s| Node {
                id: s.id,
                version: &s.version,
                requires: &s.requires,
                disabled: s.disabled,
                quarantined: s.quarantined,
                failure: s.failure,
            })
            .collect();
        let r = resolve(&nodes);
        let order = r.order.iter().map(|&i| specs[i].id).collect();
        (r, order)
    }

    #[test]
    fn orders_dependencies_first_and_cascades_reasons() {
        let mut specs = vec![
            spec("app", "1.0.0", &[("mid", "^1", false), ("extra", "^1", true)]),
            spec("mid", "1.2.0", &[("base", "^2", false)]),
            spec("base", "2.0.0", &[]),
        ];
        let (r, order) = run(&specs);
        assert_eq!(order, vec!["base", "mid", "app"]);
        assert!(r.states.iter().all(|s| *s == State::Active));
        assert_eq!(r.warnings[0], vec!["optional: needs extra ^1 (not installed)"]);
        assert_eq!(r.dependents[2], vec!["mid"]);
        assert_eq!(r.dependents[1], vec!["app"]);

        specs[2].quarantined = true;
        let (r, _) = run(&specs);
        assert_eq!(r.states[2], State::Quarantined);
        assert_eq!(r.states[1], State::Inactive("needs base which is quarantined".into()));
        assert_eq!(
            r.states[0],
            State::Inactive("needs mid which is inactive (needs base which is quarantined)".into())
        );

        specs[2].quarantined = false;
        specs[2].disabled = true;
        assert_eq!(run(&specs).0.states[1], State::Inactive("needs base which is disabled".into()));

        specs[2].disabled = false;
        specs[2].version = Version::new(1, 9, 0);
        assert_eq!(run(&specs).0.states[1], State::Inactive("needs base ^2 (installed 1.9.0)".into()));

        specs[2].version = Version::new(2, 0, 0);
        specs[1].failure = Some("activation failed: boom");
        let (r, _) = run(&specs);
        assert_eq!(r.states[1], State::Inactive("activation failed: boom".into()));
    }

    #[test]
    fn cycles_are_named_and_optional_back_edges_are_ignored() {
        let specs = vec![
            spec("a", "1.0.0", &[("b", "*", false)]),
            spec("b", "1.0.0", &[("c", "*", false)]),
            spec("c", "1.0.0", &[("a", "*", false)]),
            spec("d", "1.0.0", &[("a", "*", false)]),
            spec("e", "1.0.0", &[("f", "*", true)]),
            spec("f", "1.0.0", &[("e", "*", false)]),
        ];
        let (r, order) = run(&specs);
        assert_eq!(r.states[0], State::Inactive("dependency cycle: a → b → c → a".into()));
        assert_eq!(r.states[1], State::Inactive("dependency cycle: b → c → a → b".into()));
        assert!(matches!(&r.states[3], State::Inactive(s) if s.starts_with("needs a which is inactive (dependency cycle")));
        // e optionally uses f, which requires e: f activates after e, e only warns.
        assert_eq!(r.states[4], State::Active);
        assert_eq!(r.states[5], State::Active);
        assert_eq!(r.warnings[4], vec!["optional: needs f which depends on this plugin"]);
        let pos = |id| order.iter().position(|x| *x == id).unwrap();
        assert!(pos("e") < pos("f"));
    }

    #[test]
    fn long_reasons_are_cut() {
        let long = "x".repeat(MAX_REASON + 10);
        assert_eq!(cut(long).chars().count(), MAX_REASON + 1);
        assert_eq!(cut("short".into()), "short");
    }
}
