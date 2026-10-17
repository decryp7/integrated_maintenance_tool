//! Chrome trace event JSON importer (Trace Event Format: an array of events
//! or `{"traceEvents": [...]}`; `ts`/`dur` in microseconds of the trace clock,
//! so the wall-clock start is unknown).
//!
//! - **CPU profile** (preferred): `Profile` events (`args.data.startTime`)
//!   and `ProfileChunk` events (`args.data.cpuProfile.{nodes, samples}`,
//!   `args.data.timeDeltas`) linked by `pid` + `id`, as written by Chrome
//!   DevTools' Performance panel and `node --cpu-prof`-style tracing. Node
//!   lists are merged across chunks; the profile's thread is the `tid` of its
//!   `Profile` event, named by `M` `thread_name` metadata. Metrics and time
//!   attribution are those of the `.cpuprofile` importer. Legacy
//!   `CpuProfile` events (a whole `cpuProfile` in one event) are read too.
//!   Duration events are then ignored (noted).
//! - **Duration events** (without CPU profile samples): `X` (`ts`, `dur`)
//!   and `B`/`E` pairs per (`pid`, `tid`) become an exact, evented profile per
//!   thread (`TimeNs` self time, `Calls`, `MaxCallNs`; see
//!   [`crate::speedscope::Evented`]). `E` closes the innermost open `B` of
//!   its thread; unclosed `B`s end at the thread's last timestamp; an event
//!   that overlaps its enclosing one without nesting is cut at the enclosing
//!   end.
//!
//! In both modes instant (`i`/`I`) and complete (`X`) events whose name
//! contains `GC` become garbage-collection timeline events.

use super::nodes::{Node, NodeStacks};
use super::{check_size, offset, sample_weights, us_to_ns};
use crate::speedscope::Evented;
use crate::util;
use imt_profile::{
    Capture, CaptureImporter, EventKind, FrameSpec, Limits, Measurement, MetricId, MetricInfo, Profile, ProfileBuilder, ProfileError,
    ProfileEvent, ProfileKind, Result,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use std::collections::HashMap;

const ID: &str = "chrome-trace";

/// Imports Chrome trace event JSON.
pub struct TraceEventImporter;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TraceObject {
    #[serde(default)]
    trace_events: Option<Vec<TraceEvent>>,
}

#[derive(Deserialize, Default)]
struct TraceEvent {
    #[serde(default, deserialize_with = "lenient")]
    name: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    ph: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    ts: Option<f64>,
    #[serde(default, deserialize_with = "lenient")]
    dur: Option<f64>,
    #[serde(default, deserialize_with = "key")]
    pid: String,
    #[serde(default, deserialize_with = "key")]
    tid: String,
    #[serde(default, deserialize_with = "key")]
    id: String,
    #[serde(default, deserialize_with = "lenient")]
    args: Option<Args>,
}

#[derive(Deserialize, Default)]
struct Args {
    #[serde(default, deserialize_with = "lenient")]
    name: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    data: Option<Data>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Data {
    #[serde(default, deserialize_with = "lenient")]
    start_time: Option<f64>,
    #[serde(default, deserialize_with = "lenient")]
    cpu_profile: Option<ChunkProfile>,
    #[serde(default, deserialize_with = "lenient")]
    time_deltas: Option<Vec<f64>>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ChunkProfile {
    #[serde(default)]
    nodes: Vec<Node>,
    #[serde(default)]
    samples: Vec<u64>,
    /// Legacy `CpuProfile` events carry the whole profile.
    #[serde(default)]
    start_time: Option<f64>,
    #[serde(default)]
    time_deltas: Option<Vec<f64>>,
}

/// A field of an unexpected type is treated as absent instead of failing the
/// whole file (traces mix events from many producers).
fn lenient<'de, D: Deserializer<'de>, T: DeserializeOwned>(d: D) -> std::result::Result<Option<T>, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(v).ok())
}

/// An id (number or string) as text; `""` when absent or of another type.
fn key<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<String, D::Error> {
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::String(s) => s,
        serde_json::Value::Number(n) => n.to_string(),
        _ => String::new(),
    })
}

impl CaptureImporter for TraceEventImporter {
    fn id(&self) -> &'static str {
        ID
    }

    fn name(&self) -> &'static str {
        "Chrome trace events"
    }

    fn sniff(&self, file_name: &str, head: &[u8]) -> u8 {
        let text = util::head_text(head);
        let score = if let Some(rest) = text.strip_prefix('{') {
            if rest.trim_start().starts_with("\"traceEvents\"") {
                85
            } else if text.contains("\"traceEvents\"") {
                70
            } else {
                0
            }
        } else if let Some(rest) = text.strip_prefix('[') {
            let looks =
                rest.trim_start().starts_with('{') && text.contains("\"ph\"") && (text.contains("\"ts\"") || text.contains("\"name\""));
            if looks {
                85
            } else {
                0
            }
        } else {
            0
        };
        if score > 0 && util::extension(file_name) == "cpuprofile" {
            10
        } else {
            score
        }
    }

    fn import(&self, file_name: &str, bytes: &[u8], limits: &Limits) -> Result<Capture> {
        check_size(bytes, limits)?;
        let bytes = util::maybe_gunzip(bytes, limits, ID)?;
        let events = parse(&bytes)?;
        build(events, file_name, limits).map(Capture::Profile)
    }
}

/// The events of either top-level shape. The JSON Array Format may lack its
/// closing `]` (the spec allows it, for traces cut off by a crash).
fn parse(bytes: &[u8]) -> Result<Vec<TraceEvent>> {
    let text = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let first = text.iter().find(|b| !b.is_ascii_whitespace()).copied();
    match first {
        Some(b'[') => match util::json::<Vec<TraceEvent>>(text, ID) {
            Ok(v) => Ok(v),
            Err(e) => {
                let trimmed = text.trim_ascii_end();
                let trimmed = trimmed.strip_suffix(b",").unwrap_or(trimmed);
                if trimmed.ends_with(b"]") {
                    return Err(e);
                }
                let mut closed = Vec::with_capacity(trimmed.len() + 1);
                closed.extend_from_slice(trimmed);
                closed.push(b']');
                util::json::<Vec<TraceEvent>>(&closed, ID).map_err(|_| e)
            }
        },
        Some(b'{') => util::json::<TraceObject>(text, ID)?
            .trace_events
            .ok_or_else(|| ProfileError::malformed(ID, "the JSON object has no \"traceEvents\" array")),
        _ => Err(ProfileError::malformed(
            ID,
            "a trace is a JSON array of events or an object with \"traceEvents\"",
        )),
    }
}

/// One V8 CPU profile assembled from its events.
#[derive(Default)]
struct CpuProf {
    pid: String,
    tid: String,
    /// µs on the trace clock.
    start_us: Option<f64>,
    nodes: Vec<Node>,
    samples: Vec<u64>,
    deltas: Vec<f64>,
    mismatched_chunks: u64,
}

impl CpuProf {
    fn add(&mut self, cpu: ChunkProfile, deltas: Option<Vec<f64>>) {
        self.nodes.extend(cpu.nodes);
        let deltas = deltas.or(cpu.time_deltas).unwrap_or_default();
        if deltas.len() != cpu.samples.len() {
            self.mismatched_chunks += 1;
        }
        let n = deltas.len().min(cpu.samples.len());
        self.samples.extend_from_slice(&cpu.samples[..n]);
        self.deltas.extend_from_slice(&deltas[..n]);
    }
}

fn is_gc(name: &str) -> bool {
    name.contains("GC")
}

fn build(events: Vec<TraceEvent>, file_name: &str, limits: &Limits) -> Result<Profile> {
    let mut thread_names: HashMap<(String, String), String> = HashMap::new();
    let mut profs: Vec<CpuProf> = Vec::new();
    let mut prof_index: HashMap<(String, String), usize> = HashMap::new();
    let mut origin: Option<f64> = None;
    let mut durations = 0usize;
    let mut take_origin = |t: f64| {
        if t.is_finite() {
            origin = Some(origin.map_or(t, |o: f64| o.min(t)));
        }
    };
    for e in &events {
        let ph = e.ph.as_deref().unwrap_or("");
        let name = e.name.as_deref().unwrap_or("");
        if ph == "M" {
            if name == "thread_name" {
                if let Some(n) = e.args.as_ref().and_then(|a| a.name.clone()) {
                    thread_names.insert((e.pid.clone(), e.tid.clone()), n);
                }
            }
            continue;
        }
        if let Some(ts) = e.ts {
            take_origin(ts);
        }
        if matches!(ph, "X" | "B" | "E") {
            durations += 1;
        }
    }
    // Which profile each Profile / ProfileChunk / CpuProfile event belongs to.
    let mut slot: Vec<Option<usize>> = vec![None; events.len()];
    for (ei, e) in events.iter().enumerate() {
        let name = e.name.as_deref().unwrap_or("");
        if !matches!(name, "Profile" | "ProfileChunk" | "CpuProfile") {
            continue;
        }
        let Some(data) = e.args.as_ref().and_then(|a| a.data.as_ref()) else {
            continue;
        };
        let fresh = || CpuProf {
            pid: e.pid.clone(),
            tid: e.tid.clone(),
            ..CpuProf::default()
        };
        let i = if name == "CpuProfile" {
            profs.push(fresh());
            profs.len() - 1
        } else {
            *prof_index.entry((e.pid.clone(), e.id.clone())).or_insert_with(|| {
                profs.push(fresh());
                profs.len() - 1
            })
        };
        slot[ei] = Some(i);
        let p = &mut profs[i];
        if name == "Profile" {
            p.tid = e.tid.clone();
            p.start_us = data.start_time.or(e.ts);
        }
        if p.start_us.is_none() {
            p.start_us = data.cpu_profile.as_ref().and_then(|c| c.start_time).or(e.ts);
        }
        if let Some(t) = p.start_us {
            take_origin(t);
        }
    }
    // Second pass moves the (large) chunk payloads instead of cloning them.
    let mut events = events;
    for (e, i) in events.iter_mut().zip(&slot) {
        let (Some(i), Some(data)) = (i, e.args.as_mut().and_then(|a| a.data.as_mut())) else {
            continue;
        };
        if let Some(cpu) = data.cpu_profile.take() {
            profs[*i].add(cpu, data.time_deltas.take());
        }
    }
    profs.retain(|p| !p.nodes.is_empty());

    let origin_ns = us_to_ns(origin.unwrap_or(0.0));
    let mut ctx = Ctx {
        thread_names,
        tid_owner: HashMap::new(),
        threads: HashMap::new(),
    };
    let mut b;
    if !profs.is_empty() {
        b = ProfileBuilder::new(
            ID,
            ProfileKind::Cpu,
            &[
                MetricInfo::new(MetricId::TimeNs, Measurement::Sampled),
                MetricInfo::new(MetricId::Samples, Measurement::Sampled),
            ],
            *limits,
        )?;
        let mut end = 0;
        for p in &profs {
            end = end.max(cpu_profile(&mut b, &mut ctx, p, origin_ns)?);
        }
        b.set_duration_ns(end);
        if durations > 0 {
            b.note(format!(
                "{durations} duration events were not used: the trace has CPU profile samples"
            ));
        }
    } else if durations > 0 {
        b = ProfileBuilder::new(
            ID,
            ProfileKind::Wall,
            &[
                MetricInfo::new(MetricId::TimeNs, Measurement::Exact),
                MetricInfo::new(MetricId::Calls, Measurement::Exact),
                MetricInfo::new(MetricId::MaxCallNs, Measurement::Exact),
            ],
            *limits,
        )?;
        let end = evented(&mut b, &mut ctx, &events, origin_ns)?;
        b.set_duration_ns(end);
    } else {
        return Err(ProfileError::malformed(
            ID,
            "the trace has neither CPU profile samples nor duration events",
        ));
    }
    b.set_name(imt_profile::frames::basename(file_name));
    b.set_has_time(true);
    for e in &events {
        let name = e.name.as_deref().unwrap_or("");
        let ph = e.ph.as_deref().unwrap_or("");
        if !matches!(ph, "i" | "I" | "X") || !is_gc(name) {
            continue;
        }
        let Some(ts) = e.ts else { continue };
        let thread = ctx.threads.get(&(e.pid.clone(), e.tid.clone())).copied();
        b.event(ProfileEvent {
            kind: EventKind::Gc,
            thread,
            start_ns: offset(us_to_ns(ts), origin_ns),
            dur_ns: if ph == "X" {
                e.dur.map_or(0, |d| offset(us_to_ns(d), 0))
            } else {
                0
            },
            label: name.to_string(),
            stack: None,
        });
    }
    b.finish()
}

/// Thread naming and creation by (pid, tid).
struct Ctx {
    thread_names: HashMap<(String, String), String>,
    /// tid → pid of the thread that used it as its OS id.
    tid_owner: HashMap<String, String>,
    threads: HashMap<(String, String), u32>,
}

impl Ctx {
    fn name(&self, pid: &str, tid: &str) -> String {
        self.thread_names
            .get(&(pid.to_string(), tid.to_string()))
            .cloned()
            .unwrap_or_else(|| format!("Thread {tid}"))
    }

    fn thread(&mut self, b: &mut ProfileBuilder, pid: &str, tid: &str) -> Result<u32> {
        let k = (pid.to_string(), tid.to_string());
        if let Some(&t) = self.threads.get(&k) {
            return Ok(t);
        }
        let base = self.name(pid, tid);
        let owner = self.tid_owner.entry(tid.to_string()).or_insert_with(|| pid.to_string());
        let os_id = tid.parse::<u64>().ok().filter(|_| owner == pid);
        let t = match os_id {
            Some(o) => b.thread(Some(o), &base)?,
            None => b.thread(None, &format!("{base} (pid {pid}, tid {tid})"))?,
        };
        self.threads.insert(k, t);
        Ok(t)
    }
}

/// Adds one CPU profile's samples; returns the end of its last sample (ns since the origin).
fn cpu_profile(b: &mut ProfileBuilder, ctx: &mut Ctx, p: &CpuProf, origin_ns: i64) -> Result<u64> {
    let thread = ctx.thread(b, &p.pid, &p.tid)?;
    let stacks = NodeStacks::resolve(ID, &p.nodes, b)?;
    if stacks.truncated > 0 {
        b.note(format!(
            "{} call tree nodes deeper than the stack depth limit were merged into their caller",
            stacks.truncated
        ));
    }
    if p.mismatched_chunks > 0 {
        b.note(format!(
            "{} profile chunks had different numbers of samples and time deltas; the extra entries were skipped",
            p.mismatched_chunks
        ));
    }
    let mut t = us_to_ns(p.start_us.unwrap_or(0.0));
    let times: Vec<i64> = p
        .deltas
        .iter()
        .map(|&d| {
            t = t.saturating_add(us_to_ns(d));
            t
        })
        .collect();
    let weights = sample_weights(&times, None);
    let mut end = 0;
    for ((&id, &t), &w) in p.samples.iter().zip(&times).zip(&weights) {
        let stack = stacks.stack(ID, id, b)?;
        let at = offset(t, origin_ns);
        b.sample(thread, at, stack, &[w, 1])?;
        end = end.max(at.saturating_add(w));
    }
    Ok(end)
}

/// A call interval of one thread, in ns since the origin.
struct Interval<'a> {
    start: u64,
    end: u64,
    name: &'a str,
}

/// The calls of one thread while pairing events.
struct ThreadCalls<'a> {
    pid: String,
    tid: String,
    intervals: Vec<Interval<'a>>,
    /// Open `B` events: start and name.
    open: Vec<(u64, &'a str)>,
    /// Latest timestamp seen.
    last: u64,
}

/// Adds the duration events as evented threads; returns the latest end (ns since the origin).
fn evented(b: &mut ProfileBuilder, ctx: &mut Ctx, events: &[TraceEvent], origin_ns: i64) -> Result<u64> {
    // Per thread, in first-seen order.
    let mut per_thread: Vec<ThreadCalls<'_>> = Vec::new();
    let mut index: HashMap<(&str, &str), usize> = HashMap::new();
    let mut ordered: Vec<&TraceEvent> = events.iter().filter(|e| matches!(e.ph.as_deref(), Some("X" | "B" | "E"))).collect();
    ordered.sort_by(|a, b| a.ts.unwrap_or(0.0).total_cmp(&b.ts.unwrap_or(0.0)));
    let (mut stray_ends, mut unclosed, mut overlaps, mut end_all) = (0u64, 0u64, 0u64, 0u64);
    for e in ordered {
        let Some(ts) = e.ts else { continue };
        let i = *index.entry((e.pid.as_str(), e.tid.as_str())).or_insert_with(|| {
            per_thread.push(ThreadCalls {
                pid: e.pid.clone(),
                tid: e.tid.clone(),
                intervals: Vec::new(),
                open: Vec::new(),
                last: 0,
            });
            per_thread.len() - 1
        });
        let ThreadCalls { intervals, open, last, .. } = &mut per_thread[i];
        let start = offset(us_to_ns(ts), origin_ns);
        let name = e.name.as_deref().unwrap_or("");
        match e.ph.as_deref() {
            Some("X") => {
                let end = start.saturating_add(offset(us_to_ns(e.dur.unwrap_or(0.0)), 0));
                *last = (*last).max(end);
                intervals.push(Interval { start, end, name });
            }
            Some("B") => {
                *last = (*last).max(start);
                open.push((start, name));
            }
            _ => {
                *last = (*last).max(start);
                match open.pop() {
                    Some((s, n)) => intervals.push(Interval {
                        start: s,
                        end: start,
                        name: n,
                    }),
                    None => stray_ends += 1,
                }
            }
        }
    }
    for ThreadCalls {
        pid,
        tid,
        mut intervals,
        open,
        last,
    } in per_thread
    {
        unclosed += open.len() as u64;
        end_all = end_all.max(last);
        intervals.extend(open.into_iter().map(|(start, name)| Interval { start, end: last, name }));
        // Outer calls first: by start, longer first.
        intervals.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
        let thread = ctx.thread(b, &pid, &tid)?;
        let mut ev = Evented::new(thread, b.limits().max_stack_depth);
        let mut ends: Vec<u64> = Vec::new();
        for iv in intervals {
            while let Some(&top) = ends.last() {
                if top > iv.start {
                    break;
                }
                ev.close_top(b, top)?;
                ends.pop();
            }
            let end = match ends.last() {
                Some(&outer) if iv.end > outer => {
                    overlaps += 1;
                    outer
                }
                _ => iv.end,
            };
            let frame = b.frame(FrameSpec::new(iv.name))?;
            ev.open(b, frame, iv.start)?;
            ends.push(end);
        }
        while let Some(top) = ends.pop() {
            ev.close_top(b, top)?;
        }
        let name = ctx.name(&pid, &tid);
        ev.report(b, &name);
    }
    if stray_ends > 0 {
        b.note(format!("{stray_ends} end (E) events without a matching begin were ignored"));
    }
    if unclosed > 0 {
        b.note(format!(
            "{unclosed} begin (B) events without an end were closed at their thread's last event"
        ));
    }
    if overlaps > 0 {
        b.note(format!(
            "{overlaps} events overlapped their enclosing event and were cut at its end"
        ));
    }
    Ok(end_all)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn import(json: &str) -> Result<Profile> {
        import_named("t.json", json)
    }

    fn import_named(name: &str, json: &str) -> Result<Profile> {
        match TraceEventImporter.import(name, json.as_bytes(), &Limits::default())? {
            Capture::Profile(p) => Ok(p),
            Capture::Heap(_) => panic!("not a profile"),
        }
    }

    fn metric_by_name(p: &Profile, m: MetricId) -> std::collections::BTreeMap<String, u64> {
        let mi = p.metric_index(m).unwrap();
        let mut out = std::collections::BTreeMap::new();
        for i in 0..p.samples.len() {
            let f = &p.frames[p.stacks[p.samples.stack[i] as usize].frame as usize].function;
            m.fold(out.entry(f.clone()).or_insert(0), p.samples.values[i * p.metrics.len() + mi]);
        }
        out
    }

    const CHUNKED: &str = r#"{"traceEvents":[
      {"ph":"M","name":"thread_name","pid":1,"tid":7,"ts":0,"args":{"name":"CrRendererMain"}},
      {"ph":"P","name":"Profile","id":"0x1","pid":1,"tid":7,"ts":1000,"args":{"data":{"startTime":1000}}},
      {"ph":"P","name":"ProfileChunk","id":"0x1","pid":1,"tid":9,"ts":1100,"args":{"data":{"cpuProfile":{
         "nodes":[{"id":1,"callFrame":{"functionName":"(root)","scriptId":0,"url":"","lineNumber":-1}},
                  {"id":2,"parent":1,"callFrame":{"functionName":"main","scriptId":1,"url":"https://x/app.js","lineNumber":3}}],
         "samples":[2,2]},"timeDeltas":[10,10]}}},
      {"ph":"P","name":"ProfileChunk","id":"0x1","pid":1,"tid":9,"ts":1200,"args":{"data":{"cpuProfile":{
         "nodes":[{"id":3,"parent":2,"callFrame":{"functionName":"work","scriptId":1,"url":"https://x/app.js","lineNumber":9}}],
         "samples":[3,3,2]},"timeDeltas":[10,10,10]}}},
      {"ph":"X","name":"RunTask","pid":1,"tid":8,"ts":1005,"dur":30},
      {"ph":"i","name":"MinorGC","pid":1,"tid":7,"ts":1020,"s":"t"},
      {"ph":"X","name":"MajorGC","pid":1,"tid":5,"ts":1030,"dur":4}
    ]}"#;

    #[test]
    fn merges_profile_chunks_into_a_sampled_thread() {
        let p = import(CHUNKED).unwrap();
        assert_eq!(p.kind, ProfileKind::Cpu);
        assert!(p.has_time);
        assert_eq!(p.threads.len(), 1);
        assert_eq!(p.threads[0].name, "CrRendererMain");
        assert_eq!(p.threads[0].os_id, Some(7));
        // Origin is the Profile start (1000 µs); samples at +10..+50 µs.
        assert_eq!(p.samples.time, vec![10_000, 20_000, 30_000, 40_000, 50_000]);
        let t = metric_by_name(&p, MetricId::TimeNs);
        assert_eq!(t["work"], 20_000);
        assert_eq!(t["main"], 30_000, "two gaps plus the average for the last sample");
        assert_eq!(metric_by_name(&p, MetricId::Samples)["work"], 2);
        assert!(p.notes.iter().any(|n| n.contains("duration events were not used")), "{:?}", p.notes);
        assert_eq!(p.events.len(), 2);
        assert_eq!(p.events[0].label, "MinorGC");
        assert_eq!(p.events[0].thread, Some(0));
        assert_eq!(
            (p.events[1].start_ns, p.events[1].dur_ns, p.events[1].thread),
            (30_000, 4_000, None)
        );
    }

    #[test]
    fn chunks_without_profile_event_and_legacy_cpu_profile() {
        let json = r#"[{"ph":"P","name":"ProfileChunk","id":2,"pid":3,"tid":4,"ts":50,"args":{"data":{"cpuProfile":{
            "nodes":[{"id":1,"callFrame":{"functionName":"a"}}],"samples":[1,1,1]},"timeDeltas":[1,1]}}},
          {"ph":"I","name":"CpuProfile","pid":3,"tid":6,"ts":40,"args":{"data":{"cpuProfile":{"startTime":40,
            "nodes":[{"id":1,"callFrame":{"functionName":"(root)"},"children":[2]},{"id":2,"callFrame":{"functionName":"b"}}],
            "samples":[2,2],"timeDeltas":[5,5]}}}}"#;
        let p = import(json).unwrap();
        assert_eq!(p.threads.len(), 2);
        assert!(p.notes.iter().any(|n| n.contains("different numbers")), "{:?}", p.notes);
        let s = metric_by_name(&p, MetricId::Samples);
        assert_eq!((s["a"], s["b"]), (2, 2));
        assert_eq!(p.samples.time[0], 5_000, "legacy profile starts at 40 µs, the origin");
    }

    #[test]
    fn duration_events_become_an_exact_evented_profile() {
        let json = r#"[
          {"ph":"M","name":"thread_name","pid":1,"tid":2,"args":{"name":"worker"}},
          {"ph":"X","name":"outer","pid":1,"tid":2,"ts":100,"dur":100},
          {"ph":"X","name":"inner","pid":1,"tid":2,"ts":120,"dur":30},
          {"ph":"B","name":"span","pid":1,"tid":2,"ts":160},
          {"ph":"E","pid":1,"tid":2,"ts":170},
          {"ph":"X","name":"inner","pid":1,"tid":2,"ts":180,"dur":50},
          {"ph":"B","name":"other","pid":1,"tid":3,"ts":100},
          {"ph":"X","name":"x","pid":1,"tid":3,"ts":150,"dur":10},
          {"ph":"E","pid":1,"tid":3,"ts":400},
          {"ph":"E","pid":1,"tid":3,"ts":401},
          {"ph":"B","name":"open","pid":2,"tid":3,"ts":300},
          {"ph":"X","name":"tail","pid":2,"tid":3,"ts":310,"dur":5},
          {"ph":"i","name":"V8.GC_SCAVENGER","pid":1,"tid":2,"ts":130}
        ]"#;
        let p = import(json).unwrap();
        assert_eq!(p.kind, ProfileKind::Wall);
        assert!(p.metrics.iter().all(|m| m.measurement == Measurement::Exact));
        assert_eq!(p.threads.len(), 3);
        assert_eq!(p.threads[0].name, "worker");
        assert!(
            p.threads.iter().any(|t| t.name.contains("pid 2") && t.os_id.is_none()),
            "{:?}",
            p.threads
        );
        let t = metric_by_name(&p, MetricId::TimeNs);
        // outer: 100 − inner 30 − span 10 − inner 20 (cut at outer's end) = 40 µs.
        assert_eq!(t["outer"], 40_000);
        assert_eq!(t["inner"], 50_000);
        assert_eq!(t["span"], 10_000);
        let c = metric_by_name(&p, MetricId::Calls);
        assert_eq!(c["inner"], 2);
        assert_eq!(metric_by_name(&p, MetricId::MaxCallNs)["inner"], 30_000);
        let notes = p.notes.join("\n");
        assert!(notes.contains("without a matching begin"), "{notes}");
        assert!(notes.contains("without an end"), "{notes}");
        assert!(notes.contains("overlapped"), "{notes}");
        assert_eq!(p.events.len(), 1);
        assert_eq!(p.events[0].kind, EventKind::Gc);
    }

    #[test]
    fn parse_shapes_and_errors() {
        // Unterminated array (allowed by the spec).
        let p = import(r#"[{"ph":"X","name":"a","pid":1,"tid":1,"ts":0,"dur":5},"#).unwrap();
        assert_eq!(p.duration_ns, 5_000);
        let err = |json: &str| import(json).unwrap_err().to_string();
        assert!(err(r#"{"other":1}"#).contains("traceEvents"));
        assert!(err("42").contains("JSON array"));
        assert!(err("[1,2]").contains("invalid JSON"), "{}", err("[1,2]"));
        assert!(err("[{\"ph\":").contains("invalid JSON"));
        assert!(err(r#"[{"ph":"i","name":"x","ts":1}]"#).contains("neither"));
        assert!(err(r#"{"traceEvents":[]}"#).contains("neither"));
        // Odd field types are ignored, not fatal.
        let p = import(r#"[{"ph":"X","name":5,"pid":"a","tid":null,"ts":"x","dur":1},{"ph":"X","name":"b","pid":[1],"tid":1,"ts":1,"dur":2,"args":7}]"#)
            .unwrap();
        assert!(p.frames.iter().any(|f| f.function == "b"));
        let bad_tree = r#"[{"ph":"P","name":"ProfileChunk","id":1,"pid":1,"tid":1,"args":{"data":{"cpuProfile":{
            "nodes":[{"id":1,"parent":1,"callFrame":{"functionName":"a"}}],"samples":[1]},"timeDeltas":[1]}}}]"#;
        assert!(err(bad_tree).contains("ancestor"));
        let small = Limits {
            max_input_bytes: 1,
            ..Limits::default()
        };
        assert!(TraceEventImporter.import("x", b"[]", &small).is_err());
    }

    #[test]
    fn sniffing() {
        let s = TraceEventImporter;
        assert_eq!(s.sniff("t.json", br#"{ "traceEvents": ["#), 85);
        assert_eq!(s.sniff("t.json", br#"{"metadata":{},"traceEvents":["#), 70);
        assert_eq!(s.sniff("t.json", br#"[ {"name":"a","ph":"X","ts":1}"#), 85);
        assert_eq!(s.sniff("t.json", br#"[{"level":"info"}]"#), 0);
        assert_eq!(s.sniff("t.json", br#"[1,2]"#), 0);
        assert_eq!(s.sniff("t.json", br#"{"nodes":[{"callFrame":{}}],"samples":[]}"#), 0);
        assert_eq!(s.sniff("t.cpuprofile", br#"{"traceEvents":[]}"#), 10);
        assert_eq!(s.sniff("t.json", b"a;b 1"), 0);
    }
}
