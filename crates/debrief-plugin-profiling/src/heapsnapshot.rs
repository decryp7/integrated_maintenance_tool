//! V8 heap snapshot importer (`.heapsnapshot`, written by Chrome DevTools'
//! Memory panel, `node --heapsnapshot-signal`, `v8.writeHeapSnapshot()`).
//!
//! Format (V8 `src/profiler/heap-snapshot-generator.cc`, `HeapSnapshotJSONSerializer`;
//! read by DevTools' `HeapSnapshot.ts`):
//! `{"snapshot": {"meta": {node_fields, node_types, edge_fields, edge_types, ...},
//! "node_count", "edge_count"}, "nodes": [flat ints], "edges": [flat ints], "strings": [...]}`.
//! Every node is `node_fields.len()` consecutive integers, every edge
//! `edge_fields.len()`; field positions are read from `meta` (`type`, `name`,
//! `self_size`, `edge_count` for nodes; `type`, `name_or_index`, `to_node`
//! for edges). The edges of node `k` are the next `edge_count[k]` edges.
//! `to_node` is an offset into `nodes` (node index × node field count) and
//! is checked for divisibility and range; every string index is checked too.
//!
//! Graph mapping:
//! - Every node becomes an object, in order (so object ids are node
//!   indices), with its `self_size`.
//! - Types are grouped like the DevTools summary: `object` and `native`
//!   nodes by their name (the constructor), `closure` → `(closure)`, the
//!   string kinds → `(string)`, `array` → `(array)`, `code` →
//!   `(compiled code)`, `hidden` → `(system)`, `regexp` → `(regexp)`,
//!   `number`/`heap number` → `(number)`, others `(<type>)`. Synthetic nodes
//!   keep their own name when it is parenthesised (`(GC roots)`, `(Strong
//!   roots)`, ...) so paths to roots stay readable; the unnamed snapshot root
//!   is `(root)`, other synthetic nodes `(synthetic)`.
//! - Edges keep their label: `[n]` for `element` and `hidden` edges (whose
//!   `name_or_index` is an index), the property / variable name otherwise.
//!   `weak` edges are dropped (they do not retain, so dominators and
//!   retained sizes must ignore them), as are `shortcut` edges (DevTools
//!   leaves them out of retainers too).
//! - Only node 0, V8's synthetic snapshot root, is a GC root. It references
//!   `(GC roots)` and the global objects, so every retained object is
//!   reachable from it and paths to roots start there.
//!
//! Counts are validated against the limits (objects, references) before
//! anything is built. The snapshot records no wall-clock time.

use crate::util;
use debrief_profile::api::CaptureKind;
use debrief_profile::heap::{HeapBuilder, HeapSnapshot};
use debrief_profile::{Capture, CaptureImporter, Limits, ProfileError, Result};
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer};

const ID: &str = "heapsnapshot";

/// Imports V8 heap snapshots.
pub struct HeapSnapshotImporter;

#[derive(Deserialize)]
struct File {
    snapshot: Header,
    #[serde(default)]
    nodes: Vec<Sat32>,
    #[serde(default)]
    edges: Vec<Sat32>,
    #[serde(default)]
    strings: Vec<String>,
}

#[derive(Deserialize)]
struct Header {
    meta: Meta,
    #[serde(default)]
    node_count: Option<u64>,
    #[serde(default)]
    edge_count: Option<u64>,
}

#[derive(Deserialize)]
struct Meta {
    node_fields: Vec<String>,
    node_types: Vec<serde_json::Value>,
    edge_fields: Vec<String>,
    edge_types: Vec<serde_json::Value>,
}

/// A non-negative integer stored in 32 bits (larger values saturate, which
/// later fails index validation), halving the memory of the flat arrays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Sat32(u32);

impl<'de> Deserialize<'de> for Sat32 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = Sat32;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a non-negative integer")
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<Sat32, E> {
                Ok(Sat32(u32::try_from(v).unwrap_or(u32::MAX)))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<Sat32, E> {
                u64::try_from(v)
                    .map(|v| Sat32(u32::try_from(v).unwrap_or(u32::MAX)))
                    .map_err(|_| E::custom("negative number"))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> std::result::Result<Sat32, E> {
                if v.is_finite() && v >= 0.0 {
                    // `as` saturates.
                    Ok(Sat32(v as u32))
                } else {
                    Err(E::custom("negative or non-finite number"))
                }
            }
        }
        d.deserialize_any(V)
    }
}

/// Whether `text` starts with `tokens`, allowing whitespace between them.
fn starts_with_tokens(text: &str, tokens: &[&str]) -> bool {
    let mut rest = text;
    for t in tokens {
        rest = rest.trim_start();
        match rest.strip_prefix(t) {
            Some(r) => rest = r,
            None => return false,
        }
    }
    true
}

impl CaptureImporter for HeapSnapshotImporter {
    fn id(&self) -> &'static str {
        ID
    }

    fn name(&self) -> &'static str {
        "V8 heap snapshot"
    }

    fn sniff(&self, file_name: &str, head: &[u8]) -> u8 {
        let text = util::head_text(head);
        if starts_with_tokens(&text, &["{", "\"snapshot\"", ":", "{", "\"meta\""]) {
            100
        } else if util::extension(file_name) == "heapsnapshot" && text.starts_with('{') {
            90
        } else if starts_with_tokens(&text, &["{", "\"snapshot\""]) {
            80
        } else {
            0
        }
    }

    fn import(&self, file_name: &str, bytes: &[u8], limits: &Limits) -> Result<Capture> {
        if bytes.len() as u64 > limits.max_input_bytes {
            return Err(ProfileError::LimitExceeded {
                what: "file size",
                limit: limits.max_input_bytes,
                hint: "open a smaller capture",
            });
        }
        let bytes = util::maybe_gunzip(bytes, limits, ID)?;
        let file: File = util::json(&bytes, ID)?;
        build(&file, file_name, limits).map(Capture::Heap)
    }

    fn produces(&self) -> CaptureKind {
        CaptureKind::Heap
    }
}

fn position(fields: &[String], name: &str, what: &str) -> Result<usize> {
    fields
        .iter()
        .position(|f| f == name)
        .ok_or_else(|| ProfileError::malformed(ID, format!("the snapshot's meta has no {what} field \"{name}\"")))
}

/// The enum names of a type field (`node_types[pos]` / `edge_types[pos]` is an array of strings).
fn type_names(types: &[serde_json::Value], pos: usize, what: &str) -> Result<Vec<String>> {
    types
        .get(pos)
        .and_then(|v| v.as_array())
        .map(|a| a.iter().map(|s| s.as_str().unwrap_or("").to_string()).collect())
        .ok_or_else(|| ProfileError::malformed(ID, format!("the snapshot's meta does not list the {what} types")))
}

/// DevTools-style type name of a node.
fn group(node_type: &str, name: &str) -> String {
    match node_type {
        "object" | "native" if !name.is_empty() => name.to_string(),
        "object" => "(object)".into(),
        "closure" => "(closure)".into(),
        "string" | "concatenated string" | "sliced string" => "(string)".into(),
        "array" => "(array)".into(),
        "code" => "(compiled code)".into(),
        "hidden" => "(system)".into(),
        "regexp" => "(regexp)".into(),
        "number" | "heap number" => "(number)".into(),
        "synthetic" if name.is_empty() => "(root)".into(),
        "synthetic" if name.starts_with('(') => name.to_string(),
        "synthetic" => "(synthetic)".into(),
        other => format!("({other})"),
    }
}

/// Field positions and enum names from `meta`.
struct Layout {
    nf: usize,
    n_type: usize,
    n_name: usize,
    n_size: usize,
    n_edges: usize,
    node_types: Vec<String>,
    ef: usize,
    e_type: usize,
    e_name: usize,
    e_to: usize,
    edge_types: Vec<String>,
}

impl Layout {
    fn new(meta: &Meta) -> Result<Layout> {
        let nf = meta.node_fields.len();
        let ef = meta.edge_fields.len();
        let n_type = position(&meta.node_fields, "type", "node")?;
        let e_type = position(&meta.edge_fields, "type", "edge")?;
        Ok(Layout {
            nf,
            n_type,
            n_name: position(&meta.node_fields, "name", "node")?,
            n_size: position(&meta.node_fields, "self_size", "node")?,
            n_edges: position(&meta.node_fields, "edge_count", "node")?,
            node_types: type_names(&meta.node_types, n_type, "node")?,
            ef,
            e_type,
            e_name: position(&meta.edge_fields, "name_or_index", "edge")?,
            e_to: position(&meta.edge_fields, "to_node", "edge")?,
            edge_types: type_names(&meta.edge_types, e_type, "edge")?,
        })
    }
}

fn build(file: &File, file_name: &str, limits: &Limits) -> Result<HeapSnapshot> {
    let l = Layout::new(&file.snapshot.meta)?;
    let (nodes, edges) = (&file.nodes, &file.edges);
    if !nodes.len().is_multiple_of(l.nf) || !edges.len().is_multiple_of(l.ef) {
        return Err(ProfileError::malformed(
            ID,
            "the nodes or edges array is not a whole number of records",
        ));
    }
    let n = nodes.len() / l.nf;
    let m = edges.len() / l.ef;
    if file.snapshot.node_count.is_some_and(|c| c != n as u64) || file.snapshot.edge_count.is_some_and(|c| c != m as u64) {
        return Err(ProfileError::malformed(
            ID,
            format!(
                "the snapshot declares {} nodes and {} edges but contains {n} and {m}",
                file.snapshot.node_count.unwrap_or(n as u64),
                file.snapshot.edge_count.unwrap_or(m as u64)
            ),
        ));
    }
    if n == 0 {
        return Err(ProfileError::malformed(ID, "the snapshot contains no objects"));
    }
    if n as u64 > u64::from(limits.max_objects) {
        return Err(ProfileError::LimitExceeded {
            what: "objects",
            limit: u64::from(limits.max_objects),
            hint: "the snapshot is too large to analyse here",
        });
    }
    if m as u64 > limits.max_edges {
        return Err(ProfileError::LimitExceeded {
            what: "references",
            limit: limits.max_edges,
            hint: "the snapshot is too large to analyse here",
        });
    }
    let string = |i: u32, what: &str| {
        file.strings.get(i as usize).map(String::as_str).ok_or_else(|| {
            ProfileError::malformed(
                ID,
                format!("{what} refers to string {i}, but the snapshot has {}", file.strings.len()),
            )
        })
    };
    let mut hb = HeapBuilder::new(ID, *limits);
    hb.set_name(debrief_profile::frames::basename(file_name));
    let mut cursor = 0usize;
    let mut label = String::new();
    for k in 0..n {
        let node = &nodes[k * l.nf..(k + 1) * l.nf];
        let t = node[l.n_type].0;
        let node_type = l
            .node_types
            .get(t as usize)
            .ok_or_else(|| ProfileError::malformed(ID, format!("node {k} has the unknown type {t}")))?;
        let name = string(node[l.n_name].0, "a node's name")?;
        let type_id = hb.type_id(&group(node_type, name))?;
        hb.object(type_id, u64::from(node[l.n_size].0))?;
        let count = node[l.n_edges].0 as usize;
        if count > m - cursor {
            return Err(ProfileError::malformed(
                ID,
                "the nodes' edge counts add up to more edges than the snapshot has",
            ));
        }
        for e in cursor..cursor + count {
            let edge = &edges[e * l.ef..(e + 1) * l.ef];
            let et = edge[l.e_type].0;
            let edge_type = l
                .edge_types
                .get(et as usize)
                .ok_or_else(|| ProfileError::malformed(ID, format!("edge {e} has the unknown type {et}")))?;
            let to = edge[l.e_to].0 as usize;
            if !to.is_multiple_of(l.nf) || to >= nodes.len() {
                return Err(ProfileError::malformed(
                    ID,
                    format!("edge {e} points to {to}, which is not the start of a node"),
                ));
            }
            let value = edge[l.e_name].0;
            label.clear();
            match edge_type.as_str() {
                "weak" | "shortcut" => continue,
                "element" | "hidden" => {
                    use std::fmt::Write;
                    let _ = write!(label, "[{value}]");
                }
                _ => label.push_str(string(value, "an edge's name")?),
            }
            hb.edge((to / l.nf) as u32, Some(&label))?;
        }
        cursor += count;
    }
    if cursor != m {
        return Err(ProfileError::malformed(
            ID,
            format!("the nodes' edge counts add up to {cursor} edges but the snapshot has {m}"),
        ));
    }
    hb.root(0);
    hb.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    const META: &str = r#""meta":{
        "node_fields":["type","name","id","self_size","edge_count","trace_node_id","detachedness"],
        "node_types":[["hidden","array","string","object","code","closure","regexp","number","native","synthetic","concatenated string","sliced string","symbol","bigint","object shape"],"string","number","number","number","number","number"],
        "edge_fields":["type","name_or_index","to_node"],
        "edge_types":[["context","element","property","internal","hidden","shortcut","weak"],"string_or_number","node"]}"#;

    /// root(0) -[element 1]-> GC roots(7) -[property "cache"]-> Cache(14) -[element 0]-> Order(21), -[weak]-> Order(28)
    /// Order(21) -[property "fn"]-> closure(35); Cache -[shortcut]-> closure.
    fn snapshot(meta: &str, nodes: &str, edges: &str) -> String {
        format!(
            r#"{{"snapshot":{{{meta},"node_count":6,"edge_count":6}},"nodes":[{nodes}],"edges":[{edges}],"strings":["","(GC roots)","Cache","Order","cache","fn","handler"]}}"#
        )
    }

    const NODES: &str = "9,0,1,0,1,0,0, 9,1,3,0,1,0,0, 3,2,5,40,3,0,0, 3,3,7,24,1,0,0, 3,3,9,24,0,0,0, 5,6,11,32,0,0,0";
    const EDGES: &str = "1,1,7, 2,4,14, 1,0,21, 6,0,28, 5,5,35, 2,5,35";

    fn import(json: &str) -> Result<HeapSnapshot> {
        import_with(json, &Limits::default())
    }

    fn import_with(json: &str, limits: &Limits) -> Result<HeapSnapshot> {
        match HeapSnapshotImporter.import("d/h.heapsnapshot", json.as_bytes(), limits)? {
            Capture::Heap(h) => Ok(h),
            Capture::Profile(_) => panic!("not a heap"),
        }
    }

    #[test]
    fn builds_the_graph_with_types_labels_and_root() {
        let h = import(&snapshot(META, NODES, EDGES)).unwrap();
        assert_eq!(h.name, "h.heapsnapshot");
        assert_eq!(h.taken_ms, None);
        let names: Vec<&str> = h.types.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["(root)", "(GC roots)", "Cache", "Order", "(closure)"]);
        let order = h.types.iter().find(|t| t.name == "Order").unwrap();
        assert_eq!((order.count, order.bytes), (2, 48));
        assert_eq!(h.total_bytes(), 120);
        let g = h.graph.as_ref().unwrap();
        assert_eq!(g.len(), 6);
        assert_eq!(g.roots, vec![0]);
        let edges = |o: u32| -> Vec<(u32, String)> { g.edges(o).map(|(t, l)| (t, g.label(l).unwrap().to_string())).collect() };
        assert_eq!(edges(0), vec![(1, "[1]".to_string())]);
        assert_eq!(edges(1), vec![(2, "cache".to_string())]);
        assert_eq!(edges(2), vec![(3, "[0]".to_string())], "weak and shortcut edges are skipped");
        assert_eq!(edges(3), vec![(5, "fn".to_string())]);
        assert!(edges(4).is_empty());
    }

    #[test]
    fn field_positions_come_from_meta() {
        // Same graph with node fields and edge fields reordered.
        let meta = META
            .replace(
                r#"["type","name","id","self_size","edge_count","trace_node_id","detachedness"]"#,
                r#"["edge_count","self_size","name","type"]"#,
            )
            .replace(r#""node_types":[["#, r#""node_types":["number","number","string",["#)
            .replace(r#"["type","name_or_index","to_node"]"#, r#"["to_node","type","name_or_index"]"#)
            .replace(r#""edge_types":[["#, r#""edge_types":["node",["#);
        let nodes = "1,0,0,9, 1,0,1,9, 3,40,2,3, 1,24,3,3, 0,24,3,3, 0,32,6,5";
        let edges = "4,1,1, 8,2,4, 12,1,0, 16,6,0, 20,5,5, 20,2,5";
        let h = import(&snapshot(&meta, nodes, edges)).unwrap();
        let g = h.graph.unwrap();
        assert_eq!(g.edges(2).count(), 1);
        assert_eq!(h.types.iter().find(|t| t.name == "Order").unwrap().count, 2);
    }

    #[test]
    fn grouping() {
        assert_eq!(group("object", "Map"), "Map");
        assert_eq!(group("object", ""), "(object)");
        assert_eq!(group("native", "Detached HTMLDivElement"), "Detached HTMLDivElement");
        assert_eq!(group("sliced string", "abc"), "(string)");
        assert_eq!(group("concatenated string", "abc"), "(string)");
        assert_eq!(group("array", "x"), "(array)");
        assert_eq!(group("code", "x"), "(compiled code)");
        assert_eq!(group("hidden", "x"), "(system)");
        assert_eq!(group("regexp", "a+"), "(regexp)");
        assert_eq!(group("heap number", "x"), "(number)");
        assert_eq!(group("synthetic", "(Strong roots)"), "(Strong roots)");
        assert_eq!(group("synthetic", "x"), "(synthetic)");
        assert_eq!(group("bigint", "x"), "(bigint)");
    }

    #[test]
    fn rejects_inconsistent_snapshots() {
        let err = |json: String| import(&json).unwrap_err().to_string();
        assert!(err(snapshot(META, NODES, "1,1,7, 2,4,14, 1,0,21, 6,0,28, 5,5,35, 2,5,36")).contains("not the start of a node"));
        assert!(err(snapshot(META, NODES, "1,1,7, 2,4,14, 1,0,21, 6,0,28, 5,5,35, 2,5,42")).contains("not the start of a node"));
        assert!(err(snapshot(META, NODES, "1,1,7, 2,4,14, 1,0,21, 6,0,28, 5,5,35, 2,99,35")).contains("string 99"));
        assert!(err(snapshot(META, NODES, "1,1,7, 2,4,14, 1,0,21, 6,0,28, 5,5,35, 9,5,35")).contains("unknown type 9"));
        assert!(err(snapshot(META, NODES, "1,1,7, 2,4,14, 1,0,21, 6,0,28, 5,5,35")).contains("declares"));
        assert!(err(snapshot(META, NODES, "1,1,7, 2,4,14")).contains("declares"));
        assert!(err(snapshot(META, NODES, "1,1,7, 2,4")).contains("whole number"));
        assert!(err(snapshot(
            META,
            "9,0,1,0,1,0,0, 9,1,3,0,1,0,0, 3,2,5,40,3,0,0, 3,3,7,24,1,0,0, 3,3,9,24,0,0,0, 5,99,11,32,0,0,0",
            EDGES
        ))
        .contains("string 99"));
        assert!(err(snapshot(
            META,
            "9,0,1,0,1,0,0, 9,1,3,0,1,0,0, 3,2,5,40,3,0,0, 3,3,7,24,1,0,0, 3,3,9,24,0,0,0, 77,6,11,32,0,0,0",
            EDGES
        ))
        .contains("unknown type 77"));
        // Edge counts: too many, and too few.
        assert!(err(snapshot(
            META,
            "9,0,1,0,1,0,0, 9,1,3,0,1,0,0, 3,2,5,40,3,0,0, 3,3,7,24,1,0,0, 3,3,9,24,0,0,0, 5,6,11,32,9,0,0",
            EDGES
        ))
        .contains("more edges"));
        assert!(err(snapshot(
            META,
            "9,0,1,0,1,0,0, 9,1,3,0,1,0,0, 3,2,5,40,3,0,0, 3,3,7,24,0,0,0, 3,3,9,24,0,0,0, 5,6,11,32,0,0,0",
            EDGES
        ))
        .contains("add up to 5"));
        let no_size = META.replace("\"self_size\"", "\"size\"");
        assert!(err(snapshot(&no_size, NODES, EDGES)).contains("\"self_size\""));
        let no_types = META.replace(r#""node_types":[["#, r#""node_types":["x",["#);
        assert!(err(snapshot(&no_types, NODES, EDGES)).contains("node types"));
        assert!(err(snapshot(META, "", "")).contains("declares"));
        let empty = r#"{"snapshot":{"meta":{"node_fields":["type","name","self_size","edge_count"],"node_types":[[]],"edge_fields":["type","name_or_index","to_node"],"edge_types":[[]]}},"nodes":[],"edges":[]}"#;
        assert!(import(empty).unwrap_err().to_string().contains("no objects"));
        let no_fields = empty.replace(r#"["type","name","self_size","edge_count"]"#, "[]");
        assert!(import(&no_fields).unwrap_err().to_string().contains("\"type\""));
        assert!(err(snapshot(META, "9,0,1,0,1,0,-1", "")).contains("invalid JSON"));
        assert!(
            err(snapshot(META, "9,0,1,0,1,0,1.5e300", "")).contains("declares"),
            "huge floats saturate"
        );
        assert!(err(snapshot(META, "9,0,1,0,1,0,\"x\"", "")).contains("invalid JSON"));
        assert!(import("[]").unwrap_err().to_string().contains("invalid JSON"));
    }

    #[test]
    fn limits_are_checked_before_building() {
        let json = snapshot(META, NODES, EDGES);
        let few_objects = Limits {
            max_objects: 5,
            ..Limits::default()
        };
        assert!(matches!(
            import_with(&json, &few_objects),
            Err(ProfileError::LimitExceeded { what: "objects", .. })
        ));
        let few_edges = Limits {
            max_edges: 5,
            ..Limits::default()
        };
        assert!(matches!(
            import_with(&json, &few_edges),
            Err(ProfileError::LimitExceeded { what: "references", .. })
        ));
        let small = Limits {
            max_input_bytes: 10,
            ..Limits::default()
        };
        assert!(matches!(
            import_with(&json, &small),
            Err(ProfileError::LimitExceeded { what: "file size", .. })
        ));
    }

    #[test]
    fn sniffing() {
        let s = HeapSnapshotImporter;
        assert_eq!(s.produces(), CaptureKind::Heap);
        assert_eq!(s.sniff("x.json", b"{ \"snapshot\" : {\n \"meta\":{"), 100);
        assert_eq!(s.sniff("x.heapsnapshot", br#"{"nodes":[]}"#), 90);
        assert_eq!(s.sniff("x.json", br#"{"snapshot":{"node_count":1"#), 80);
        assert_eq!(s.sniff("x.json", br#"{"nodes":[{"callFrame":{}}]}"#), 0);
        assert_eq!(s.sniff("x.heapsnapshot", b"garbage"), 0);
        assert!(starts_with_tokens("  {", &["{"]));
        assert!(!starts_with_tokens("", &["{"]));
    }
}
