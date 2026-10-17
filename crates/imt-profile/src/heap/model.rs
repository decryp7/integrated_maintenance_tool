//! Heap snapshot model: per-type statistics and, when the format records it,
//! the object graph (objects, references, GC roots).

use crate::error::{ProfileError, Result};
use crate::limits::{clip, Limits};
use crate::model::NONE;
use rustc_hash::FxHashMap;
use serde::Serialize;

/// Count and shallow size of the objects of one type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeStat {
    pub name: String,
    pub count: u64,
    pub bytes: u64,
}

/// Objects and references in compressed sparse row form: the references of
/// object `i` are `edge_to[edge_start[i]..edge_start[i + 1]]`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ObjectGraph {
    /// Index into [`HeapSnapshot::types`].
    pub type_of: Vec<u32>,
    /// Shallow size in bytes.
    pub size: Vec<u32>,
    pub edge_start: Vec<u64>,
    pub edge_to: Vec<u32>,
    /// Index into `labels`, or [`NONE`].
    pub edge_label: Vec<u32>,
    /// Field names, array indices, variable names of references.
    pub labels: Vec<String>,
    /// Objects referenced by GC roots (statics, stacks, handles).
    pub roots: Vec<u32>,
}

impl ObjectGraph {
    pub fn len(&self) -> usize {
        self.type_of.len()
    }

    pub fn is_empty(&self) -> bool {
        self.type_of.is_empty()
    }

    /// References of object `o`, with their label index.
    pub fn edges(&self, o: u32) -> impl Iterator<Item = (u32, u32)> + '_ {
        let (a, b) = (self.edge_start[o as usize] as usize, self.edge_start[o as usize + 1] as usize);
        self.edge_to[a..b].iter().copied().zip(self.edge_label[a..b].iter().copied())
    }

    pub fn label(&self, label: u32) -> Option<&str> {
        self.labels.get(label as usize).map(String::as_str)
    }
}

/// An imported heap snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct HeapSnapshot {
    pub name: String,
    /// The importer (format) it came from.
    pub source: String,
    /// When it was taken (Unix ms), if recorded.
    pub taken_ms: Option<i64>,
    pub types: Vec<TypeStat>,
    pub graph: Option<ObjectGraph>,
    pub notes: Vec<String>,
}

impl HeapSnapshot {
    pub fn total_bytes(&self) -> u64 {
        self.types.iter().map(|t| t.bytes).sum()
    }

    pub fn total_count(&self) -> u64 {
        self.types.iter().map(|t| t.count).sum()
    }
}

/// Builds a [`HeapSnapshot`]: either type statistics only, or objects with
/// their references (added object by object, each followed by its edges).
pub struct HeapBuilder {
    limits: Limits,
    source: &'static str,
    name: String,
    taken_ms: Option<i64>,
    types: Vec<TypeStat>,
    type_index: FxHashMap<String, u32>,
    graph: ObjectGraph,
    label_index: FxHashMap<String, u32>,
    has_graph: bool,
    notes: Vec<String>,
}

impl HeapBuilder {
    pub fn new(source: &'static str, limits: Limits) -> HeapBuilder {
        HeapBuilder {
            limits,
            source,
            name: String::new(),
            taken_ms: None,
            types: Vec::new(),
            type_index: FxHashMap::default(),
            graph: ObjectGraph {
                edge_start: vec![0],
                ..ObjectGraph::default()
            },
            label_index: FxHashMap::default(),
            has_graph: false,
            notes: Vec::new(),
        }
    }

    pub fn set_name(&mut self, name: &str) -> &mut Self {
        self.name = clip(name, self.limits.max_name_len);
        self
    }

    pub fn set_taken_ms(&mut self, ms: Option<i64>) -> &mut Self {
        self.taken_ms = ms;
        self
    }

    pub fn note(&mut self, note: impl Into<String>) -> &mut Self {
        if self.notes.len() < 100 {
            self.notes.push(note.into());
        }
        self
    }

    /// The id of a type (created on first use).
    pub fn type_id(&mut self, name: &str) -> Result<u32> {
        let name = clip(name, self.limits.max_name_len);
        if let Some(&id) = self.type_index.get(&name) {
            return Ok(id);
        }
        if self.types.len() as u64 >= u64::from(self.limits.max_frames) {
            return Err(ProfileError::LimitExceeded {
                what: "types",
                limit: u64::from(self.limits.max_frames),
                hint: "the snapshot has an unusual number of types",
            });
        }
        let id = self.types.len() as u32;
        self.types.push(TypeStat {
            name: name.clone(),
            count: 0,
            bytes: 0,
        });
        self.type_index.insert(name, id);
        Ok(id)
    }

    /// Adds statistics for a type (formats without an object graph).
    pub fn type_stat(&mut self, name: &str, count: u64, bytes: u64) -> Result<()> {
        let id = self.type_id(name)? as usize;
        self.types[id].count = self.types[id].count.saturating_add(count);
        self.types[id].bytes = self.types[id].bytes.saturating_add(bytes);
        Ok(())
    }

    /// Starts the next object (ids are consecutive from 0); its edges follow.
    pub fn object(&mut self, type_id: u32, size: u64) -> Result<u32> {
        if type_id as usize >= self.types.len() {
            return Err(ProfileError::malformed(self.source, "an object has an unknown type"));
        }
        if self.graph.type_of.len() as u64 >= u64::from(self.limits.max_objects) {
            return Err(ProfileError::LimitExceeded {
                what: "objects",
                limit: u64::from(self.limits.max_objects),
                hint: "the snapshot is too large to analyse here",
            });
        }
        self.has_graph = true;
        let id = self.graph.type_of.len() as u32;
        self.graph.type_of.push(type_id);
        self.graph.size.push(u32::try_from(size).unwrap_or(u32::MAX));
        self.graph.edge_start.push(self.graph.edge_to.len() as u64);
        let t = &mut self.types[type_id as usize];
        t.count += 1;
        t.bytes = t.bytes.saturating_add(size);
        Ok(id)
    }

    /// A reference from the last object added to object `to` (which may come later).
    pub fn edge(&mut self, to: u32, label: Option<&str>) -> Result<()> {
        if self.graph.type_of.is_empty() {
            return Err(ProfileError::malformed(self.source, "a reference comes before any object"));
        }
        if self.graph.edge_to.len() as u64 >= self.limits.max_edges {
            return Err(ProfileError::LimitExceeded {
                what: "references",
                limit: self.limits.max_edges,
                hint: "the snapshot is too large to analyse here",
            });
        }
        let label = match label {
            None => NONE,
            Some(l) => match self.label_index.get(l) {
                Some(&i) => i,
                None => {
                    let i = self.graph.labels.len() as u32;
                    self.graph.labels.push(clip(l, self.limits.max_name_len));
                    self.label_index.insert(l.to_string(), i);
                    i
                }
            },
        };
        self.graph.edge_to.push(to);
        self.graph.edge_label.push(label);
        if let Some(last) = self.graph.edge_start.last_mut() {
            *last = self.graph.edge_to.len() as u64;
        }
        Ok(())
    }

    /// Marks an object as referenced by a GC root.
    pub fn root(&mut self, object: u32) {
        self.graph.roots.push(object);
    }

    /// Finishes the snapshot; references to objects that do not exist are an error.
    pub fn finish(mut self) -> Result<HeapSnapshot> {
        let graph = if self.has_graph {
            let n = self.graph.type_of.len() as u32;
            if self.graph.edge_to.iter().chain(self.graph.roots.iter()).any(|&t| t >= n) {
                return Err(ProfileError::malformed(
                    self.source,
                    "a reference points to an object that does not exist",
                ));
            }
            self.graph.roots.sort_unstable();
            self.graph.roots.dedup();
            Some(self.graph)
        } else {
            None
        };
        if self.types.is_empty() {
            return Err(ProfileError::malformed(self.source, "the snapshot contains no objects"));
        }
        let name = if self.name.is_empty() { self.source.to_string() } else { self.name };
        Ok(HeapSnapshot {
            name,
            source: self.source.to_string(),
            taken_ms: self.taken_ms,
            types: self.types,
            graph,
            notes: self.notes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_graphs_and_stats() {
        let mut b = HeapBuilder::new("t", Limits::default());
        assert!(b.edge(0, None).is_err(), "edge before object");
        let a = b.type_id("A").unwrap();
        let s = b.type_id("S").unwrap();
        assert_eq!(b.type_id("A").unwrap(), a);
        assert!(b.object(9, 1).is_err());
        let o0 = b.object(a, 10).unwrap();
        b.edge(1, Some("field")).unwrap();
        b.edge(1, Some("field")).unwrap();
        let o1 = b.object(s, 5).unwrap();
        b.edge(0, None).unwrap();
        b.root(o0);
        b.root(o0);
        b.set_name("snap").set_taken_ms(Some(5)).note("n");
        let h = b.finish().unwrap();
        let g = h.graph.as_ref().unwrap();
        assert_eq!(g.len(), 2);
        assert!(!g.is_empty());
        assert_eq!(g.edges(o0).collect::<Vec<_>>(), [(1, 0), (1, 0)]);
        assert_eq!(g.edges(o1).collect::<Vec<_>>(), [(0, NONE)]);
        assert_eq!(g.label(0), Some("field"));
        assert_eq!(g.roots, [0]);
        assert_eq!(h.total_bytes(), 15);
        assert_eq!(h.total_count(), 2);
        assert_eq!(h.name, "snap");
    }

    #[test]
    fn rejects_dangling_and_empty_and_limits() {
        let mut b = HeapBuilder::new("t", Limits::default());
        let a = b.type_id("A").unwrap();
        b.object(a, 1).unwrap();
        b.edge(5, None).unwrap();
        assert!(b.finish().is_err());
        assert!(HeapBuilder::new("t", Limits::default()).finish().is_err());

        let mut b = HeapBuilder::new("t", Limits::default());
        b.type_stat("X", 2, 20).unwrap();
        b.type_stat("X", 1, 5).unwrap();
        let h = b.finish().unwrap();
        assert!(h.graph.is_none());
        assert_eq!((h.types[0].count, h.types[0].bytes), (3, 25));
        assert_eq!(h.name, "t");

        let limits = Limits {
            max_objects: 1,
            max_edges: 1,
            max_frames: 1,
            ..Limits::default()
        };
        let mut b = HeapBuilder::new("t", limits);
        let a = b.type_id("A").unwrap();
        assert!(b.type_id("B").is_err());
        b.object(a, u64::MAX).unwrap();
        assert!(b.object(a, 1).is_err());
        b.edge(0, None).unwrap();
        assert!(b.edge(0, None).is_err());
        let h = b.finish().unwrap();
        assert_eq!(h.graph.unwrap().size[0], u32::MAX);
    }
}
