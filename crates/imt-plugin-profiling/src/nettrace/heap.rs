//! .NET GC heap walks in a trace (what `dotnet-gcdump` and PerfView record):
//! the runtime induces a GC and describes the heap with `GCBulkType`,
//! `GCBulkNode` (objects), `GCBulkEdge` (references, in node order),
//! `GCBulkRootEdge`, `GCBulkRootStaticVar`, `GCBulkRootCCW` and
//! `GCBulkRootConditionalWeakTableElementEdge` events. They become a
//! [`HeapSnapshot`] with the object graph.
//!
//! Roots are grouped under synthetic objects (`[Stack roots]`, `[Finalizer
//! queue]`, `[Handles]`, `[Static variables]`, ...) so paths to roots name the
//! kind of root, as PerfView does. Weak handles do not keep objects alive and
//! are left out. Pointer-sized fields (`win:Pointer`) follow the trace's
//! pointer size, i.e. the target's bitness. Only the first heap walk of a
//! trace is used (a note says when there were more).

use super::cursor::Cursor;
use imt_profile::heap::{HeapBuilder, HeapSnapshot};
use imt_profile::{Limits, ProfileError, Result};
use std::collections::HashMap;

/// Runtime event ids of the heap walk.
pub mod ids {
    pub const BULK_TYPE: u32 = 15;
    pub const BULK_ROOT_EDGE: u32 = 16;
    pub const BULK_ROOT_CWT: u32 = 17;
    pub const BULK_NODE: u32 = 18;
    pub const BULK_EDGE: u32 = 19;
    pub const BULK_ROOT_CCW: u32 = 36;
    pub const BULK_ROOT_STATIC: u32 = 38;
}

/// Kinds of heap-walk events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HeapEvent {
    Type,
    Node,
    Edge,
    RootEdge,
    RootCwt,
    RootCcw,
    RootStatic,
}

impl HeapEvent {
    pub(super) fn from_id(id: u32) -> Option<HeapEvent> {
        Some(match id {
            ids::BULK_TYPE => HeapEvent::Type,
            ids::BULK_NODE => HeapEvent::Node,
            ids::BULK_EDGE => HeapEvent::Edge,
            ids::BULK_ROOT_EDGE => HeapEvent::RootEdge,
            ids::BULK_ROOT_CWT => HeapEvent::RootCwt,
            ids::BULK_ROOT_CCW => HeapEvent::RootCcw,
            ids::BULK_ROOT_STATIC => HeapEvent::RootStatic,
            _ => return None,
        })
    }
}

/// Root groups (synthetic objects at the top of the graph).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) enum RootGroup {
    Stack,
    Finalizer,
    Handles,
    Pinned,
    Statics,
    Ccw,
    Other,
}

impl RootGroup {
    const ALL: [RootGroup; 7] = [
        RootGroup::Stack,
        RootGroup::Finalizer,
        RootGroup::Handles,
        RootGroup::Pinned,
        RootGroup::Statics,
        RootGroup::Ccw,
        RootGroup::Other,
    ];

    fn name(self) -> &'static str {
        match self {
            RootGroup::Stack => "[Stack roots]",
            RootGroup::Finalizer => "[Finalizer queue]",
            RootGroup::Handles => "[Handles]",
            RootGroup::Pinned => "[Pinned handles]",
            RootGroup::Statics => "[Static variables]",
            RootGroup::Ccw => "[COM callable wrappers]",
            RootGroup::Other => "[Other roots]",
        }
    }

    /// From `GCRootKind` and `GCRootFlag`; `None` for weak references.
    fn of(kind: u8, flags: u32) -> Option<RootGroup> {
        const PINNING: u32 = 1;
        const WEAK: u32 = 2;
        if flags & WEAK != 0 {
            return None;
        }
        Some(match kind {
            0 => RootGroup::Stack,
            1 => RootGroup::Finalizer,
            2 if flags & PINNING != 0 => RootGroup::Pinned,
            2 => RootGroup::Handles,
            _ => RootGroup::Other,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TypeDef {
    name: String,
    params: Vec<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Node {
    addr: u64,
    size: u64,
    type_id: u64,
    edges: u64,
}

/// The heap walk collected from a trace.
#[derive(Debug, Default)]
pub(super) struct HeapParts {
    types: HashMap<u64, TypeDef>,
    nodes: Vec<Node>,
    /// Edge targets in node order.
    edges: Vec<u64>,
    roots: Vec<(RootGroup, u64, Option<String>)>,
    /// Conditional weak table entries: key keeps value alive.
    dependents: Vec<(u64, u64)>,
    /// A second heap walk started (ignored).
    more_walks: bool,
    pub damaged: u64,
}

impl HeapParts {
    /// Whether the trace contains a heap walk.
    pub(super) fn has_walk(&self) -> bool {
        !self.nodes.is_empty()
    }

    /// Adds one heap-walk event; `false` when the payload is damaged (counted).
    pub(super) fn add(&mut self, kind: HeapEvent, payload: &[u8], ptr: usize, limits: &Limits) -> Result<()> {
        let ok = match kind {
            HeapEvent::Type => self.bulk_type(payload, limits),
            HeapEvent::Node => self.bulk_node(payload, ptr, limits)?,
            HeapEvent::Edge => self.bulk_edge(payload, ptr, limits)?,
            HeapEvent::RootEdge => self.root_edge(payload, ptr),
            HeapEvent::RootCwt => self.root_cwt(payload, ptr),
            HeapEvent::RootCcw => self.root_ccw(payload, ptr),
            HeapEvent::RootStatic => self.root_static(payload, limits),
        };
        if ok.is_none() {
            self.damaged += 1;
        }
        Ok(())
    }

    fn bulk_type(&mut self, payload: &[u8], limits: &Limits) -> Option<()> {
        let mut c = Cursor::new(payload, 0);
        let count = c.u32().ok()?;
        let _clr = c.u16().ok()?;
        for _ in 0..count {
            // Each entry has at least 33 bytes; a short payload ends the loop.
            let id = c.u64().ok()?;
            let _module = c.u64().ok()?;
            let _name_id = c.u32().ok()?;
            let _flags = c.u32().ok()?;
            let _cor_element = c.u8().ok()?;
            let name = c.utf16z(limits.max_name_len).ok()?;
            let n = c.u32().ok()?;
            if u64::from(n) * 8 > c.remaining() as u64 {
                return None;
            }
            let mut params = Vec::with_capacity(n as usize);
            for _ in 0..n {
                params.push(c.u64().ok()?);
            }
            if self.types.len() < limits.max_frames as usize {
                self.types.insert(id, TypeDef { name, params });
            }
        }
        Some(())
    }

    /// Index field restarting at 0 after nodes were read means a second walk.
    fn second_walk(&mut self, index: u32, have: bool) -> bool {
        if index == 0 && have {
            self.more_walks = true;
        }
        self.more_walks
    }

    fn bulk_node(&mut self, payload: &[u8], ptr: usize, limits: &Limits) -> Result<Option<()>> {
        let mut c = Cursor::new(payload, 0);
        let (Ok(index), Ok(count), Ok(_clr)) = (c.u32(), c.u32(), c.u16()) else {
            return Ok(None);
        };
        if self.second_walk(index, !self.nodes.is_empty()) {
            return Ok(Some(()));
        }
        for _ in 0..count {
            let (Some(addr), Ok(size), Ok(type_id), Ok(edges)) = (read_ptr(&mut c, ptr), c.u64(), c.u64(), c.u64()) else {
                return Ok(None);
            };
            if self.nodes.len() as u64 >= u64::from(limits.max_objects) {
                return Err(ProfileError::LimitExceeded {
                    what: "objects",
                    limit: u64::from(limits.max_objects),
                    hint: "the heap is too large to analyse here",
                });
            }
            self.nodes.push(Node {
                addr,
                size,
                type_id,
                edges,
            });
        }
        Ok(Some(()))
    }

    fn bulk_edge(&mut self, payload: &[u8], ptr: usize, limits: &Limits) -> Result<Option<()>> {
        let mut c = Cursor::new(payload, 0);
        let (Ok(_index), Ok(count), Ok(_clr)) = (c.u32(), c.u32(), c.u16()) else {
            return Ok(None);
        };
        if self.more_walks {
            return Ok(Some(()));
        }
        for _ in 0..count {
            let (Some(to), Ok(_field)) = (read_ptr(&mut c, ptr), c.u32()) else {
                return Ok(None);
            };
            if self.edges.len() as u64 >= limits.max_edges {
                return Err(ProfileError::LimitExceeded {
                    what: "references",
                    limit: limits.max_edges,
                    hint: "the heap is too large to analyse here",
                });
            }
            self.edges.push(to);
        }
        Ok(Some(()))
    }

    fn push_root(&mut self, group: RootGroup, addr: u64, label: Option<String>) {
        // Roots are bounded by the objects they point at (unknown ones are dropped when building).
        if self.roots.len() < self.nodes.len().max(1 << 16) * 4 {
            self.roots.push((group, addr, label));
        }
    }

    fn root_edge(&mut self, payload: &[u8], ptr: usize) -> Option<()> {
        let mut c = Cursor::new(payload, 0);
        let _index = c.u32().ok()?;
        let count = c.u32().ok()?;
        let _clr = c.u16().ok()?;
        if self.more_walks {
            return Some(());
        }
        for _ in 0..count {
            let addr = read_ptr(&mut c, ptr)?;
            let kind = c.u8().ok()?;
            let flags = c.u32().ok()?;
            let _root_id = read_ptr(&mut c, ptr)?;
            if let Some(group) = RootGroup::of(kind, flags) {
                self.push_root(group, addr, None);
            }
        }
        Some(())
    }

    fn root_cwt(&mut self, payload: &[u8], ptr: usize) -> Option<()> {
        let mut c = Cursor::new(payload, 0);
        let _index = c.u32().ok()?;
        let count = c.u32().ok()?;
        let _clr = c.u16().ok()?;
        for _ in 0..count {
            let key = read_ptr(&mut c, ptr)?;
            let value = read_ptr(&mut c, ptr)?;
            let _root = read_ptr(&mut c, ptr)?;
            if !self.more_walks && self.dependents.len() < 1 << 24 {
                self.dependents.push((key, value));
            }
        }
        Some(())
    }

    fn root_ccw(&mut self, payload: &[u8], ptr: usize) -> Option<()> {
        let mut c = Cursor::new(payload, 0);
        let count = c.u32().ok()?;
        let _clr = c.u16().ok()?;
        for _ in 0..count {
            let _root = read_ptr(&mut c, ptr)?;
            let object = read_ptr(&mut c, ptr)?;
            let _type = read_ptr(&mut c, ptr)?;
            let _iunknown = read_ptr(&mut c, ptr)?;
            c.take(12).ok()?;
            if !self.more_walks {
                self.push_root(RootGroup::Ccw, object, None);
            }
        }
        Some(())
    }

    fn root_static(&mut self, payload: &[u8], limits: &Limits) -> Option<()> {
        let mut c = Cursor::new(payload, 0);
        let count = c.u32().ok()?;
        let _domain = c.u64().ok()?;
        let _clr = c.u16().ok()?;
        for _ in 0..count {
            let _root = c.u64().ok()?;
            let object = c.u64().ok()?;
            let type_id = c.u64().ok()?;
            let _flags = c.u32().ok()?;
            let field = c.utf16z(limits.max_name_len).ok()?;
            if self.more_walks {
                continue;
            }
            let owner = self.types.get(&type_id).map(|t| t.name.clone());
            let label = match owner {
                Some(t) if !t.is_empty() => format!("{t}.{field}"),
                _ => field,
            };
            self.push_root(RootGroup::Statics, object, Some(label));
        }
        Some(())
    }

    fn type_name(&self, id: u64, depth: u32) -> String {
        let Some(t) = self.types.get(&id) else {
            return "(unknown type)".to_string();
        };
        if t.params.is_empty() || depth >= 3 || t.name.contains('<') {
            return t.name.clone();
        }
        let base = t.name.split('`').next().unwrap_or(&t.name);
        let args: Vec<String> = t.params.iter().map(|&p| self.type_name(p, depth + 1)).collect();
        format!("{base}<{}>", args.join(", "))
    }

    /// The snapshot: root groups first, then the objects in walk order.
    pub(super) fn build(self, name: &str, taken_ms: Option<i64>, limits: &Limits) -> Result<HeapSnapshot> {
        let mut b = HeapBuilder::new(super::FORMAT, *limits);
        b.set_name(name).set_taken_ms(taken_ms);
        let mut index: HashMap<u64, u32> = HashMap::with_capacity(self.nodes.len());
        let groups: Vec<RootGroup> = RootGroup::ALL
            .into_iter()
            .filter(|g| self.roots.iter().any(|r| r.0 == *g))
            .collect();
        let first = groups.len() as u32;
        for (i, n) in self.nodes.iter().enumerate() {
            index.entry(n.addr).or_insert(first + i as u32);
        }
        let mut dependents: HashMap<u64, Vec<u32>> = HashMap::new();
        for (key, value) in &self.dependents {
            if let Some(&v) = index.get(value) {
                dependents.entry(*key).or_default().push(v);
            }
        }
        let mut unresolved = 0u64;
        for g in &groups {
            let t = b.type_id(g.name())?;
            let root = b.object(t, 0)?;
            b.root(root);
            for (_, addr, label) in self.roots.iter().filter(|r| r.0 == *g) {
                match index.get(addr) {
                    Some(&o) => b.edge(o, label.as_deref())?,
                    None => unresolved += 1,
                }
            }
        }
        let mut type_ids: HashMap<u64, u32> = HashMap::new();
        let mut at = 0usize;
        let mut short = false;
        for n in &self.nodes {
            let t = match type_ids.get(&n.type_id) {
                Some(&t) => t,
                None => {
                    let t = b.type_id(&self.type_name(n.type_id, 0))?;
                    type_ids.insert(n.type_id, t);
                    t
                }
            };
            b.object(t, n.size)?;
            let want = usize::try_from(n.edges).unwrap_or(usize::MAX);
            let end = at.saturating_add(want).min(self.edges.len());
            if end - at < want {
                short = true;
            }
            for to in &self.edges[at..end] {
                match index.get(to) {
                    Some(&o) => b.edge(o, None)?,
                    None if *to == 0 => {}
                    None => unresolved += 1,
                }
            }
            at = end;
            if let Some(values) = dependents.get(&n.addr) {
                for &v in values {
                    b.edge(v, Some("(conditional weak table value)"))?;
                }
            }
        }
        if unresolved > 0 {
            b.note(format!("{unresolved} references point outside the walked heap and were left out"));
        }
        if short {
            b.note("The trace ended before every reference of the heap walk was written; some references are missing");
        }
        if self.more_walks {
            b.note("The trace contains more than one heap walk; the first one is shown");
        }
        if self.damaged > 0 {
            b.note(format!("{} heap-walk events had damaged payloads and were skipped", self.damaged));
        }
        b.finish()
    }
}

/// A pointer-sized field (4 or 8 bytes).
fn read_ptr(c: &mut Cursor<'_>, ptr: usize) -> Option<u64> {
    if ptr == 4 {
        c.u32().ok().map(u64::from)
    } else {
        c.u64().ok()
    }
}

/// Payload encoders for the heap-walk events (tests, fuzz seeds and the ETW tests).
#[doc(hidden)]
pub mod payload {
    fn ptr(p: &mut Vec<u8>, v: u64, size: usize) {
        p.extend_from_slice(&v.to_le_bytes()[..size]);
    }

    fn utf16z(p: &mut Vec<u8>, s: &str) {
        for u in s.encode_utf16().chain([0]) {
            p.extend_from_slice(&u.to_le_bytes());
        }
    }

    /// `GCBulkType`: (type id, name, type parameter ids).
    pub fn bulk_type(types: &[(u64, &str, &[u64])]) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&(types.len() as u32).to_le_bytes());
        p.extend_from_slice(&0u16.to_le_bytes());
        for (id, name, params) in types {
            p.extend_from_slice(&id.to_le_bytes());
            p.extend_from_slice(&0u64.to_le_bytes());
            p.extend_from_slice(&0u32.to_le_bytes());
            p.extend_from_slice(&0u32.to_le_bytes());
            p.push(0x12);
            utf16z(&mut p, name);
            p.extend_from_slice(&(params.len() as u32).to_le_bytes());
            for t in *params {
                p.extend_from_slice(&t.to_le_bytes());
            }
        }
        p
    }

    /// `GCBulkNode`: (address, size, type id, edge count).
    pub fn bulk_node(index: u32, nodes: &[(u64, u64, u64, u64)], ptr_size: usize) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&index.to_le_bytes());
        p.extend_from_slice(&(nodes.len() as u32).to_le_bytes());
        p.extend_from_slice(&0u16.to_le_bytes());
        for (addr, size, ty, edges) in nodes {
            ptr(&mut p, *addr, ptr_size);
            p.extend_from_slice(&size.to_le_bytes());
            p.extend_from_slice(&ty.to_le_bytes());
            p.extend_from_slice(&edges.to_le_bytes());
        }
        p
    }

    /// `GCBulkEdge`: target addresses.
    pub fn bulk_edge(index: u32, targets: &[u64], ptr_size: usize) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&index.to_le_bytes());
        p.extend_from_slice(&(targets.len() as u32).to_le_bytes());
        p.extend_from_slice(&0u16.to_le_bytes());
        for t in targets {
            ptr(&mut p, *t, ptr_size);
            p.extend_from_slice(&0u32.to_le_bytes());
        }
        p
    }

    /// `GCBulkRootEdge`: (address, root kind, flags).
    pub fn bulk_root_edge(index: u32, roots: &[(u64, u8, u32)], ptr_size: usize) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&index.to_le_bytes());
        p.extend_from_slice(&(roots.len() as u32).to_le_bytes());
        p.extend_from_slice(&0u16.to_le_bytes());
        for (addr, kind, flags) in roots {
            ptr(&mut p, *addr, ptr_size);
            p.push(*kind);
            p.extend_from_slice(&flags.to_le_bytes());
            ptr(&mut p, 0x10, ptr_size);
        }
        p
    }

    /// `GCBulkRootStaticVar`: (object, owner type id, field name).
    pub fn bulk_root_static(statics: &[(u64, u64, &str)]) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&(statics.len() as u32).to_le_bytes());
        p.extend_from_slice(&1u64.to_le_bytes());
        p.extend_from_slice(&0u16.to_le_bytes());
        for (object, ty, field) in statics {
            p.extend_from_slice(&0x99u64.to_le_bytes());
            p.extend_from_slice(&object.to_le_bytes());
            p.extend_from_slice(&ty.to_le_bytes());
            p.extend_from_slice(&0u32.to_le_bytes());
            utf16z(&mut p, field);
        }
        p
    }

    /// `GCBulkRootCCW`: rooted objects.
    pub fn bulk_root_ccw(objects: &[u64], ptr_size: usize) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&(objects.len() as u32).to_le_bytes());
        p.extend_from_slice(&0u16.to_le_bytes());
        for o in objects {
            ptr(&mut p, 1, ptr_size);
            ptr(&mut p, *o, ptr_size);
            ptr(&mut p, 2, ptr_size);
            ptr(&mut p, 3, ptr_size);
            p.extend_from_slice(&[0u8; 12]);
        }
        p
    }

    /// `GCBulkRootConditionalWeakTableElementEdge`: (key, value).
    pub fn bulk_root_cwt(index: u32, pairs: &[(u64, u64)], ptr_size: usize) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&index.to_le_bytes());
        p.extend_from_slice(&(pairs.len() as u32).to_le_bytes());
        p.extend_from_slice(&0u16.to_le_bytes());
        for (k, v) in pairs {
            ptr(&mut p, *k, ptr_size);
            ptr(&mut p, *v, ptr_size);
            ptr(&mut p, 7, ptr_size);
        }
        p
    }
}

#[cfg(test)]
mod tests {
    use super::payload as p;
    use super::*;

    fn walk(ptr: usize) -> HeapParts {
        let l = Limits::default();
        let mut h = HeapParts::default();
        let list: &[u64] = &[2];
        h.add(
            HeapEvent::Type,
            &p::bulk_type(&[
                (1, "App.Cache", &[]),
                (2, "System.String", &[]),
                (3, "System.Collections.Generic.List`1", list),
            ]),
            ptr,
            &l,
        )
        .unwrap();
        h.add(
            HeapEvent::Node,
            &p::bulk_node(0, &[(0x100, 24, 1, 2), (0x200, 30, 2, 0), (0x300, 40, 3, 1)], ptr),
            ptr,
            &l,
        )
        .unwrap();
        h.add(HeapEvent::Edge, &p::bulk_edge(0, &[0x200, 0x300, 0x200], ptr), ptr, &l)
            .unwrap();
        h.add(
            HeapEvent::RootEdge,
            &p::bulk_root_edge(0, &[(0x100, 0, 0), (0x200, 2, 2), (0x300, 1, 0), (0x999, 2, 1)], ptr),
            ptr,
            &l,
        )
        .unwrap();
        h.add(HeapEvent::RootStatic, &p::bulk_root_static(&[(0x100, 1, "Instance")]), ptr, &l)
            .unwrap();
        h.add(HeapEvent::RootCcw, &p::bulk_root_ccw(&[0x300], ptr), ptr, &l).unwrap();
        h.add(HeapEvent::RootCwt, &p::bulk_root_cwt(0, &[(0x100, 0x300)], ptr), ptr, &l)
            .unwrap();
        h
    }

    #[test]
    fn builds_graph_with_root_groups_for_both_bitnesses() {
        for ptr in [4, 8] {
            let h = walk(ptr);
            assert!(h.has_walk());
            let s = h.build("heap", Some(5), &Limits::default()).unwrap();
            let g = s.graph.as_ref().unwrap();
            let names: Vec<&str> = s.types.iter().map(|t| t.name.as_str()).collect();
            assert!(names.contains(&"[Stack roots]"), "{names:?}");
            assert!(names.contains(&"[Finalizer queue]"));
            assert!(names.contains(&"[Static variables]"));
            assert!(names.contains(&"[COM callable wrappers]"));
            assert!(
                names.contains(&"[Pinned handles]"),
                "pinned handle to an unknown object still makes a group"
            );
            assert!(!names.contains(&"[Handles]"), "weak handles are not roots");
            assert!(names.contains(&"System.Collections.Generic.List<System.String>"), "{names:?}");
            // 5 root groups + 3 objects.
            assert_eq!(g.len(), 8);
            assert_eq!(g.roots.len(), 5);
            let cache = (0..g.len() as u32)
                .find(|&o| s.types[g.type_of[o as usize] as usize].name == "App.Cache")
                .unwrap();
            let refs: Vec<u32> = g.edges(cache).map(|e| e.0).collect();
            assert_eq!(refs.len(), 3, "two fields and the weak-table value");
            assert!(s.notes.iter().any(|n| n.contains("outside the walked heap")), "{:?}", s.notes);
            let statics = (0..g.len() as u32)
                .find(|&o| s.types[g.type_of[o as usize] as usize].name == "[Static variables]")
                .unwrap();
            let (_, label) = g.edges(statics).next().unwrap();
            assert_eq!(g.label(label), Some("App.Cache.Instance"));
            assert_eq!(s.taken_ms, Some(5));
        }
    }

    #[test]
    fn damaged_short_and_second_walks_are_noted() {
        let l = Limits::default();
        let mut h = walk(8);
        for kind in [
            HeapEvent::Type,
            HeapEvent::Node,
            HeapEvent::Edge,
            HeapEvent::RootEdge,
            HeapEvent::RootCwt,
            HeapEvent::RootCcw,
            HeapEvent::RootStatic,
        ] {
            h.add(kind, &[1, 0, 0, 0, 9], 8, &l).unwrap();
        }
        assert_eq!(h.damaged, 7);
        // A second walk (index 0 again) is ignored.
        h.add(HeapEvent::Node, &p::bulk_node(0, &[(0x500, 8, 2, 5)], 8), 8, &l).unwrap();
        h.add(HeapEvent::Edge, &p::bulk_edge(0, &[0x100], 8), 8, &l).unwrap();
        let s = h.build("x", None, &l).unwrap();
        assert!(s.notes.iter().any(|n| n.contains("more than one heap walk")));
        assert!(s.notes.iter().any(|n| n.contains("damaged")));

        // Nodes declaring more edges than were written.
        let mut h = HeapParts::default();
        h.add(HeapEvent::Node, &p::bulk_node(0, &[(0x1, 8, 9, 3)], 8), 8, &l).unwrap();
        let s = h.build("x", None, &l).unwrap();
        assert!(s.notes.iter().any(|n| n.contains("ended before")));
        assert_eq!(s.types[0].name, "(unknown type)");
    }

    #[test]
    fn limits_are_errors() {
        let l = Limits {
            max_objects: 1,
            max_edges: 1,
            ..Limits::default()
        };
        let mut h = HeapParts::default();
        assert!(h
            .add(HeapEvent::Node, &p::bulk_node(0, &[(1, 1, 1, 0), (2, 1, 1, 0)], 8), 8, &l)
            .is_err());
        let mut h = HeapParts::default();
        assert!(h.add(HeapEvent::Edge, &p::bulk_edge(0, &[1, 2], 8), 8, &l).is_err());
        assert_eq!(HeapEvent::from_id(18), Some(HeapEvent::Node));
        assert_eq!(HeapEvent::from_id(1), None);
        let huge = {
            let mut v = p::bulk_type(&[(1, "A", &[])]);
            let n = v.len();
            v[n - 4..].copy_from_slice(&u32::MAX.to_le_bytes());
            v
        };
        let mut h = HeapParts::default();
        h.add(HeapEvent::Type, &huge, 8, &Limits::default()).unwrap();
        assert_eq!(h.damaged, 1);
    }
}
