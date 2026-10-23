//! A capture on the wire (ADR 0017): profile importers run in the profiling plugin's process, which sends the
//! result (capture or typed error) to Debrief encoded with [`encode_result`]; Debrief decodes it with [`decode_result`],
//! which checks every index and size against [`Limits`] before the capture reaches a view (a plugin process is
//! not trusted to send a consistent model).

use crate::error::{ErrorCode, ProfileError, Result};
use crate::heap::{HeapSnapshot, ObjectGraph, TypeStat};
use crate::import::Capture;
use crate::limits::Limits;
use crate::model::{
    EventKind, Frame, FrameKind, Measurement, MetricId, MetricInfo, Profile, ProfileEvent, ProfileKind, Samples, StackNode, Thread, NONE,
};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct WFrame {
    function: String,
    module: Option<String>,
    file: Option<String>,
    line: Option<u32>,
    kind: FrameKind,
    is_framework: bool,
}

#[derive(Serialize, Deserialize)]
struct WEvent {
    kind: EventKind,
    thread: Option<u32>,
    start_ns: u64,
    dur_ns: u64,
    label: String,
    stack: Option<u32>,
}

#[derive(Serialize, Deserialize)]
struct WProfile {
    name: String,
    kind: ProfileKind,
    source: String,
    started_ms: Option<i64>,
    duration_ns: u64,
    has_time: bool,
    frames: Vec<WFrame>,
    stacks: Vec<(u32, u32)>,
    threads: Vec<(u32, Option<u64>, String)>,
    metrics: Vec<(MetricId, Measurement)>,
    sample_stack: Vec<u32>,
    sample_thread: Vec<u32>,
    sample_time: Vec<u64>,
    sample_values: Vec<u64>,
    events: Vec<WEvent>,
    notes: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct WGraph {
    type_of: Vec<u32>,
    size: Vec<u32>,
    edge_start: Vec<u64>,
    edge_to: Vec<u32>,
    edge_label: Vec<u32>,
    labels: Vec<String>,
    roots: Vec<u32>,
}

#[derive(Serialize, Deserialize)]
struct WHeap {
    name: String,
    source: String,
    taken_ms: Option<i64>,
    types: Vec<(String, u64, u64)>,
    graph: Option<WGraph>,
    notes: Vec<String>,
}

#[derive(Serialize, Deserialize)]
enum WResult {
    Profile(WProfile),
    Heap(WHeap),
    Error { code: ErrorCode, message: String },
}

fn profile_out(p: &Profile) -> WProfile {
    WProfile {
        name: p.name.clone(),
        kind: p.kind,
        source: p.source.clone(),
        started_ms: p.started_ms,
        duration_ns: p.duration_ns,
        has_time: p.has_time,
        frames: p
            .frames
            .iter()
            .map(|f| WFrame {
                function: f.function.clone(),
                module: f.module.clone(),
                file: f.file.clone(),
                line: f.line,
                kind: f.kind,
                is_framework: f.is_framework,
            })
            .collect(),
        stacks: p.stacks.iter().map(|s| (s.parent, s.frame)).collect(),
        threads: p.threads.iter().map(|t| (t.id, t.os_id, t.name.clone())).collect(),
        metrics: p.metrics.iter().map(|m| (m.id, m.measurement)).collect(),
        sample_stack: p.samples.stack.clone(),
        sample_thread: p.samples.thread.clone(),
        sample_time: p.samples.time.clone(),
        sample_values: p.samples.values.clone(),
        events: p
            .events
            .iter()
            .map(|e| WEvent {
                kind: e.kind,
                thread: e.thread,
                start_ns: e.start_ns,
                dur_ns: e.dur_ns,
                label: e.label.clone(),
                stack: e.stack,
            })
            .collect(),
        notes: p.notes.clone(),
    }
}

fn heap_out(h: &HeapSnapshot) -> WHeap {
    WHeap {
        name: h.name.clone(),
        source: h.source.clone(),
        taken_ms: h.taken_ms,
        types: h.types.iter().map(|t| (t.name.clone(), t.count, t.bytes)).collect(),
        graph: h.graph.as_ref().map(|g| WGraph {
            type_of: g.type_of.clone(),
            size: g.size.clone(),
            edge_start: g.edge_start.clone(),
            edge_to: g.edge_to.clone(),
            edge_label: g.edge_label.clone(),
            labels: g.labels.clone(),
            roots: g.roots.clone(),
        }),
        notes: h.notes.clone(),
    }
}

/// An importer's result as bytes for the host.
pub fn encode_result(r: &Result<Capture>) -> Vec<u8> {
    let w = match r {
        Ok(Capture::Profile(p)) => WResult::Profile(profile_out(p)),
        Ok(Capture::Heap(h)) => WResult::Heap(heap_out(h)),
        Err(e) => WResult::Error {
            code: e.code(),
            message: e.to_string(),
        },
    };
    postcard::to_allocvec(&w).unwrap_or_default()
}

fn bad(message: impl Into<String>) -> ProfileError {
    ProfileError::malformed("plugin capture", message)
}

fn within(what: &'static str, n: usize, max: u64) -> Result<()> {
    if n as u64 > max {
        return Err(ProfileError::LimitExceeded {
            what,
            limit: max,
            hint: "open a smaller capture",
        });
    }
    Ok(())
}

fn profile_in(w: WProfile, limits: &Limits) -> Result<Profile> {
    within("frames", w.frames.len(), u64::from(limits.max_frames))?;
    within("stack nodes", w.stacks.len(), u64::from(limits.max_stack_nodes))?;
    within("threads", w.threads.len(), u64::from(limits.max_threads))?;
    within("events", w.events.len(), u64::from(limits.max_events))?;
    within("samples", w.sample_stack.len(), limits.max_samples)?;
    let (frames, nodes, threads) = (w.frames.len() as u64, w.stacks.len() as u64, w.threads.len() as u64);
    for (i, (parent, frame)) in w.stacks.iter().enumerate() {
        // A parent always comes before its child, so the tree has no cycles.
        if u64::from(*frame) >= frames || (*parent != NONE && *parent as usize >= i) {
            return Err(bad(format!("stack node {i} refers to a missing frame or a later parent")));
        }
    }
    let n = w.sample_stack.len();
    let mut metrics = Vec::new();
    for (id, m) in &w.metrics {
        if metrics.iter().any(|x: &MetricInfo| x.id == *id) {
            return Err(bad("a metric is listed twice"));
        }
        metrics.push(MetricInfo::new(*id, *m));
    }
    if w.sample_thread.len() != n || w.sample_time.len() != n || Some(w.sample_values.len()) != n.checked_mul(metrics.len()) {
        return Err(bad("the sample columns differ in length"));
    }
    if w.sample_stack.iter().any(|s| u64::from(*s) >= nodes) || w.sample_thread.iter().any(|t| u64::from(*t) >= threads) {
        return Err(bad("a sample refers to a missing stack or thread"));
    }
    let mut events = Vec::with_capacity(w.events.len());
    for e in w.events {
        if e.thread.is_some_and(|t| u64::from(t) >= threads) || e.stack.is_some_and(|s| u64::from(s) >= nodes) {
            return Err(bad("an event refers to a missing thread or stack"));
        }
        events.push(ProfileEvent {
            kind: e.kind,
            thread: e.thread,
            start_ns: e.start_ns,
            dur_ns: e.dur_ns,
            label: e.label,
            stack: e.stack,
        });
    }
    Ok(Profile {
        name: w.name,
        kind: w.kind,
        source: w.source,
        started_ms: w.started_ms,
        duration_ns: w.duration_ns,
        has_time: w.has_time,
        frames: w
            .frames
            .into_iter()
            .map(|f| Frame {
                function: f.function,
                module: f.module,
                file: f.file,
                line: f.line,
                kind: f.kind,
                is_framework: f.is_framework,
            })
            .collect(),
        stacks: w.stacks.into_iter().map(|(parent, frame)| StackNode { parent, frame }).collect(),
        threads: w.threads.into_iter().map(|(id, os_id, name)| Thread { id, os_id, name }).collect(),
        metrics,
        samples: Samples {
            stack: w.sample_stack,
            thread: w.sample_thread,
            time: w.sample_time,
            values: w.sample_values,
        },
        events,
        notes: w.notes,
    })
}

fn heap_in(w: WHeap, limits: &Limits) -> Result<HeapSnapshot> {
    let types = w.types.len() as u64;
    let graph = match w.graph {
        None => None,
        Some(g) => {
            within("objects", g.type_of.len(), u64::from(limits.max_objects))?;
            within("references", g.edge_to.len(), limits.max_edges)?;
            let objects = g.type_of.len();
            if g.size.len() != objects || g.edge_start.len() != objects + 1 || g.edge_label.len() != g.edge_to.len() {
                return Err(bad("the object graph's columns differ in length"));
            }
            if g.edge_start.first() != Some(&0)
                || g.edge_start.last().copied() != Some(g.edge_to.len() as u64)
                || g.edge_start.windows(2).any(|w| w[0] > w[1])
            {
                return Err(bad("the object graph's edge offsets are not consistent"));
            }
            let labels = g.labels.len() as u64;
            if g.type_of.iter().any(|t| u64::from(*t) >= types)
                || g.edge_to.iter().chain(&g.roots).any(|o| *o as usize >= objects)
                || g.edge_label.iter().any(|l| *l != NONE && u64::from(*l) >= labels)
            {
                return Err(bad("the object graph refers to missing types, objects or labels"));
            }
            Some(ObjectGraph {
                type_of: g.type_of,
                size: g.size,
                edge_start: g.edge_start,
                edge_to: g.edge_to,
                edge_label: g.edge_label,
                labels: g.labels,
                roots: g.roots,
            })
        }
    };
    Ok(HeapSnapshot {
        name: w.name,
        source: w.source,
        taken_ms: w.taken_ms,
        types: w
            .types
            .into_iter()
            .map(|(name, count, bytes)| TypeStat { name, count, bytes })
            .collect(),
        graph,
        notes: w.notes,
    })
}

/// The result a plugin sent, checked against `limits`.
pub fn decode_result(bytes: &[u8], limits: &Limits) -> Result<Capture> {
    let w: WResult = postcard::from_bytes(bytes).map_err(|e| bad(format!("the capture could not be read ({e})")))?;
    match w {
        WResult::Profile(p) => profile_in(p, limits).map(Capture::Profile),
        WResult::Heap(h) => heap_in(h, limits).map(Capture::Heap),
        WResult::Error { code, message } => Err(ProfileError::Remote { code, message }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::{FrameSpec, ProfileBuilder};

    fn profile() -> Profile {
        let mut b = ProfileBuilder::new(
            "t",
            ProfileKind::Cpu,
            &[MetricInfo::new(MetricId::Samples, Measurement::Sampled)],
            Limits::default(),
        )
        .unwrap();
        let main = b.frame(FrameSpec::new("main")).unwrap();
        let work = b.frame(FrameSpec::new("work")).unwrap();
        let t = b.thread(Some(7), "main").unwrap();
        let s = b.stack(&[main, work]).unwrap();
        b.sample(t, 5, s, &[3]).unwrap();
        b.finish().unwrap()
    }

    #[test]
    fn captures_and_errors_roundtrip() {
        let p = profile();
        let back = decode_result(&encode_result(&Ok(Capture::Profile(p.clone()))), &Limits::default()).unwrap();
        assert_eq!(back, Capture::Profile(p));
        let h = HeapSnapshot {
            name: "h".into(),
            source: "s".into(),
            taken_ms: Some(1),
            types: vec![TypeStat {
                name: "T".into(),
                count: 2,
                bytes: 16,
            }],
            graph: Some(ObjectGraph {
                type_of: vec![0, 0],
                size: vec![8, 8],
                edge_start: vec![0, 1, 1],
                edge_to: vec![1],
                edge_label: vec![NONE],
                labels: Vec::new(),
                roots: vec![0],
            }),
            notes: Vec::new(),
        };
        let back = decode_result(&encode_result(&Ok(Capture::Heap(h.clone()))), &Limits::default()).unwrap();
        assert_eq!(back, Capture::Heap(h));
        let e = decode_result(
            &encode_result(&Err(ProfileError::Unrecognised("a".into(), "b".into()))),
            &Limits::default(),
        )
        .unwrap_err();
        assert_eq!(e.code(), ErrorCode::NotACapture);
        assert!(e.to_string().contains("not a recognised profile"));
    }

    #[test]
    fn inconsistent_or_oversized_captures_are_refused() {
        let mut w = profile_out(&profile());
        w.stacks.push((0, 99));
        let bytes = postcard::to_allocvec(&WResult::Profile(w)).unwrap();
        assert!(decode_result(&bytes, &Limits::default())
            .unwrap_err()
            .to_string()
            .contains("missing frame"));
        let mut w = profile_out(&profile());
        w.sample_values.pop();
        let bytes = postcard::to_allocvec(&WResult::Profile(w)).unwrap();
        assert!(decode_result(&bytes, &Limits::default()).is_err());
        let small = Limits {
            max_frames: 1,
            ..Limits::default()
        };
        let bytes = encode_result(&Ok(Capture::Profile(profile())));
        assert_eq!(decode_result(&bytes, &small).unwrap_err().code(), ErrorCode::Limit);
        assert!(decode_result(&[1, 2, 3], &Limits::default()).is_err());
        let mut g = heap_out(&HeapSnapshot {
            name: String::new(),
            source: String::new(),
            taken_ms: None,
            types: Vec::new(),
            graph: None,
            notes: Vec::new(),
        });
        g.graph = Some(WGraph {
            type_of: vec![0],
            size: vec![1],
            edge_start: vec![0, 0],
            edge_to: Vec::new(),
            edge_label: Vec::new(),
            labels: Vec::new(),
            roots: Vec::new(),
        });
        let bytes = postcard::to_allocvec(&WResult::Heap(g)).unwrap();
        assert!(decode_result(&bytes, &Limits::default()).is_err(), "type 0 does not exist");
    }
}
