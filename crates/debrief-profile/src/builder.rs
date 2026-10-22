//! Builds a [`Profile`] incrementally, interning frames and stacks and
//! enforcing [`Limits`]. Importers use only this API, so every profile obeys
//! the same invariants (parents before children, values aligned to metrics,
//! samples sorted by time).

use crate::error::{ProfileError, Result};
use crate::frames;
use crate::limits::{clip, Limits};
use crate::model::*;
use rustc_hash::FxHashMap;

/// Describes a frame to intern. `is_framework: None` applies the built-in rules.
#[derive(Debug, Clone, Default)]
pub struct FrameSpec<'a> {
    pub function: &'a str,
    pub module: Option<&'a str>,
    pub file: Option<&'a str>,
    pub line: Option<u32>,
    pub kind: FrameKind,
    pub is_framework: Option<bool>,
}

impl<'a> FrameSpec<'a> {
    pub fn new(function: &'a str) -> FrameSpec<'a> {
        FrameSpec {
            function,
            ..FrameSpec::default()
        }
    }

    pub fn module(mut self, module: Option<&'a str>) -> Self {
        self.module = module.filter(|m| !m.is_empty());
        self
    }

    pub fn file(mut self, file: Option<&'a str>) -> Self {
        self.file = file.filter(|f| !f.is_empty());
        self
    }

    pub fn line(mut self, line: Option<u32>) -> Self {
        self.line = line;
        self
    }

    pub fn kind(mut self, kind: FrameKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn framework(mut self, is_framework: bool) -> Self {
        self.is_framework = Some(is_framework);
        self
    }
}

#[derive(Hash, PartialEq, Eq)]
struct FrameKey {
    function: String,
    module: Option<String>,
    file: Option<String>,
    line: Option<u32>,
    kind: FrameKind,
}

/// Incremental profile construction.
pub struct ProfileBuilder {
    limits: Limits,
    source: &'static str,
    name: String,
    kind: ProfileKind,
    started_ms: Option<i64>,
    duration_ns: Option<u64>,
    has_time: bool,
    frames: Vec<Frame>,
    frame_index: FxHashMap<FrameKey, u32>,
    stacks: Vec<StackNode>,
    stack_index: FxHashMap<(u32, u32), u32>,
    threads: Vec<Thread>,
    thread_by_os: FxHashMap<u64, u32>,
    thread_by_name: FxHashMap<String, u32>,
    metrics: Vec<MetricInfo>,
    samples: Samples,
    events: Vec<ProfileEvent>,
    notes: Vec<String>,
    truncated_stacks: u64,
    dropped_events: u64,
}

impl ProfileBuilder {
    /// A builder for a profile from `source` (the importer id) measuring `metrics`.
    pub fn new(source: &'static str, kind: ProfileKind, metrics: &[MetricInfo], limits: Limits) -> Result<ProfileBuilder> {
        let mut unique: Vec<MetricInfo> = Vec::new();
        for m in metrics {
            if !unique.iter().any(|u| u.id == m.id) {
                unique.push(*m);
            }
        }
        if unique.is_empty() {
            return Err(ProfileError::malformed(source, "the profile measures nothing"));
        }
        Ok(ProfileBuilder {
            limits,
            source,
            name: String::new(),
            kind,
            started_ms: None,
            duration_ns: None,
            has_time: false,
            frames: Vec::new(),
            frame_index: FxHashMap::default(),
            stacks: Vec::new(),
            stack_index: FxHashMap::default(),
            threads: Vec::new(),
            thread_by_os: FxHashMap::default(),
            thread_by_name: FxHashMap::default(),
            metrics: unique,
            samples: Samples::default(),
            events: Vec::new(),
            notes: Vec::new(),
            truncated_stacks: 0,
            dropped_events: 0,
        })
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    pub fn metrics(&self) -> &[MetricInfo] {
        &self.metrics
    }

    pub fn metric_index(&self, id: MetricId) -> Option<usize> {
        self.metrics.iter().position(|m| m.id == id)
    }

    pub fn set_name(&mut self, name: &str) -> &mut Self {
        self.name = clip(name, self.limits.max_name_len);
        self
    }

    pub fn set_kind(&mut self, kind: ProfileKind) -> &mut Self {
        self.kind = kind;
        self
    }

    pub fn set_started_ms(&mut self, ms: Option<i64>) -> &mut Self {
        self.started_ms = ms;
        self
    }

    /// The capture's length; derived from the samples and events when not set.
    pub fn set_duration_ns(&mut self, ns: u64) -> &mut Self {
        self.duration_ns = Some(ns);
        self
    }

    /// Whether sample times are real timestamps.
    pub fn set_has_time(&mut self, has_time: bool) -> &mut Self {
        self.has_time = has_time;
        self
    }

    /// Something the user should know about this capture.
    pub fn note(&mut self, note: impl Into<String>) -> &mut Self {
        if self.notes.len() < 100 {
            self.notes.push(note.into());
        }
        self
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }

    /// Interns a frame.
    pub fn frame(&mut self, spec: FrameSpec<'_>) -> Result<u32> {
        let max = self.limits.max_name_len;
        let function = if spec.function.trim().is_empty() {
            "(anonymous)".to_string()
        } else {
            clip(spec.function, max)
        };
        let key = FrameKey {
            function,
            module: spec.module.map(|m| clip(m, max)),
            file: spec.file.map(|f| clip(f, max)),
            line: spec.line,
            kind: spec.kind,
        };
        if let Some(&id) = self.frame_index.get(&key) {
            return Ok(id);
        }
        if self.frames.len() as u64 >= u64::from(self.limits.max_frames) {
            return Err(ProfileError::LimitExceeded {
                what: "distinct functions",
                limit: u64::from(self.limits.max_frames),
                hint: "split the capture or record fewer modules",
            });
        }
        let id = self.frames.len() as u32;
        let is_framework = spec
            .is_framework
            .unwrap_or_else(|| frames::is_framework(&key.function, key.module.as_deref(), key.kind));
        self.frames.push(Frame {
            function: key.function.clone(),
            module: key.module.clone(),
            file: key.file.clone(),
            line: key.line,
            kind: key.kind,
            is_framework,
        });
        self.frame_index.insert(key, id);
        Ok(id)
    }

    /// The stack `parent` (or [`NONE`] for a root) extended by `frame`.
    pub fn push(&mut self, parent: u32, frame: u32) -> Result<u32> {
        if frame as usize >= self.frames.len() || (parent != NONE && parent as usize >= self.stacks.len()) {
            return Err(ProfileError::malformed(self.source, "a stack refers to an unknown frame"));
        }
        if let Some(&id) = self.stack_index.get(&(parent, frame)) {
            return Ok(id);
        }
        if self.stacks.len() as u64 >= u64::from(self.limits.max_stack_nodes) {
            return Err(ProfileError::LimitExceeded {
                what: "call tree nodes",
                limit: u64::from(self.limits.max_stack_nodes),
                hint: "split the capture into shorter recordings",
            });
        }
        let id = self.stacks.len() as u32;
        self.stacks.push(StackNode { parent, frame });
        self.stack_index.insert((parent, frame), id);
        Ok(id)
    }

    /// Interns a whole stack given root first. Stacks deeper than the limit
    /// keep their outermost frames; an empty stack becomes `(no stack)`.
    pub fn stack(&mut self, frames_root_first: &[u32]) -> Result<u32> {
        if frames_root_first.is_empty() {
            return self.empty_stack();
        }
        let max = self.limits.max_stack_depth as usize;
        let frames = if frames_root_first.len() > max {
            self.truncated_stacks += 1;
            &frames_root_first[..max]
        } else {
            frames_root_first
        };
        let mut node = NONE;
        for &f in frames {
            node = self.push(node, f)?;
        }
        Ok(node)
    }

    /// Like [`ProfileBuilder::stack`] with the frames given leaf first.
    pub fn stack_leaf_first(&mut self, frames_leaf_first: &[u32]) -> Result<u32> {
        let root_first: Vec<u32> = frames_leaf_first.iter().rev().copied().collect();
        self.stack(&root_first)
    }

    /// The stack of samples that recorded none.
    pub fn empty_stack(&mut self) -> Result<u32> {
        let f = self.frame(FrameSpec::new("(no stack)").framework(false))?;
        self.push(NONE, f)
    }

    /// A thread by OS id (or by name when the id is unknown); created on first use.
    pub fn thread(&mut self, os_id: Option<u64>, name: &str) -> Result<u32> {
        if let Some(id) = os_id.and_then(|o| self.thread_by_os.get(&o)) {
            return Ok(*id);
        }
        if os_id.is_none() {
            if let Some(id) = self.thread_by_name.get(name) {
                return Ok(*id);
            }
        }
        if self.threads.len() as u64 >= u64::from(self.limits.max_threads) {
            return Err(ProfileError::LimitExceeded {
                what: "threads",
                limit: u64::from(self.limits.max_threads),
                hint: "the capture has an unusual number of threads",
            });
        }
        let id = self.threads.len() as u32;
        let name = if name.is_empty() {
            os_id.map(|o| format!("Thread {o}")).unwrap_or_else(|| format!("Thread #{id}"))
        } else {
            clip(name, self.limits.max_name_len)
        };
        match os_id {
            Some(o) => {
                self.thread_by_os.insert(o, id);
            }
            None => {
                self.thread_by_name.insert(name.clone(), id);
            }
        }
        self.threads.push(Thread { id, os_id, name });
        Ok(id)
    }

    /// Renames a thread (formats that name threads after their first sample).
    pub fn rename_thread(&mut self, thread: u32, name: &str) {
        if let Some(t) = self.threads.get_mut(thread as usize) {
            t.name = clip(name, self.limits.max_name_len);
        }
    }

    /// Adds a sample with one value per metric (in [`ProfileBuilder::metrics`] order).
    pub fn sample(&mut self, thread: u32, time_ns: u64, stack: u32, values: &[u64]) -> Result<()> {
        if values.len() != self.metrics.len() {
            return Err(ProfileError::malformed(self.source, "a sample has the wrong number of values"));
        }
        if thread as usize >= self.threads.len() || stack as usize >= self.stacks.len() {
            return Err(ProfileError::malformed(
                self.source,
                "a sample refers to an unknown thread or stack",
            ));
        }
        if self.samples.len() as u64 >= self.limits.max_samples {
            return Err(ProfileError::LimitExceeded {
                what: "samples",
                limit: self.limits.max_samples,
                hint: "open a shorter recording or a lower sampling rate",
            });
        }
        self.samples.stack.push(stack);
        self.samples.thread.push(thread);
        self.samples.time.push(time_ns);
        self.samples.values.extend_from_slice(values);
        Ok(())
    }

    /// Adds a sample that carries values for some metrics only (the rest are 0).
    pub fn sample_with(&mut self, thread: u32, time_ns: u64, stack: u32, values: &[(MetricId, u64)]) -> Result<()> {
        let mut row = vec![0u64; self.metrics.len()];
        for (m, v) in values {
            if let Some(i) = self.metric_index(*m) {
                m.fold(&mut row[i], *v);
            }
        }
        self.sample(thread, time_ns, stack, &row)
    }

    /// Adds a timeline event; beyond the limit events are counted and dropped.
    pub fn event(&mut self, mut event: ProfileEvent) {
        if self.events.len() as u64 >= u64::from(self.limits.max_events) {
            self.dropped_events += 1;
            return;
        }
        event.label = clip(&event.label, self.limits.max_name_len);
        if event.thread.is_some_and(|t| t as usize >= self.threads.len()) {
            event.thread = None;
        }
        if event.stack.is_some_and(|s| s as usize >= self.stacks.len()) {
            event.stack = None;
        }
        self.events.push(event);
    }

    /// Finishes the profile: sorts samples by time and derives the duration.
    pub fn finish(mut self) -> Result<Profile> {
        if self.samples.is_empty() && self.events.is_empty() {
            return Err(ProfileError::malformed(self.source, "the capture contains no samples"));
        }
        if self.truncated_stacks > 0 {
            let n = self.truncated_stacks;
            let d = self.limits.max_stack_depth;
            self.note(format!("{n} stacks deeper than {d} frames were cut to their outermost {d} frames"));
        }
        if self.dropped_events > 0 {
            let n = self.dropped_events;
            self.note(format!(
                "{n} timeline events beyond the limit of {} were dropped",
                self.limits.max_events
            ));
        }
        if self.has_time {
            sort_samples(&mut self.samples, self.metrics.len());
            self.events.sort_by_key(|e| e.start_ns);
        }
        let derived = self
            .samples
            .time
            .last()
            .copied()
            .unwrap_or(0)
            .max(self.events.iter().map(|e| e.start_ns.saturating_add(e.dur_ns)).max().unwrap_or(0));
        let name = if self.name.is_empty() { self.source.to_string() } else { self.name };
        Ok(Profile {
            name,
            kind: self.kind,
            source: self.source.to_string(),
            started_ms: self.started_ms,
            duration_ns: self.duration_ns.unwrap_or(derived).max(derived),
            has_time: self.has_time,
            frames: self.frames,
            stacks: self.stacks,
            threads: self.threads,
            metrics: self.metrics,
            samples: self.samples,
            events: self.events,
            notes: self.notes,
        })
    }
}

/// Sorts the columnar samples by time (stable), unless already sorted.
fn sort_samples(s: &mut Samples, stride: usize) {
    if s.time.windows(2).all(|w| w[0] <= w[1]) {
        return;
    }
    let mut order: Vec<u32> = (0..s.len() as u32).collect();
    order.sort_by_key(|&i| s.time[i as usize]);
    let mut out = Samples {
        stack: Vec::with_capacity(s.len()),
        thread: Vec::with_capacity(s.len()),
        time: Vec::with_capacity(s.len()),
        values: Vec::with_capacity(s.values.len()),
    };
    for i in order {
        let i = i as usize;
        out.stack.push(s.stack[i]);
        out.thread.push(s.thread[i]);
        out.time.push(s.time[i]);
        out.values.extend_from_slice(&s.values[i * stride..(i + 1) * stride]);
    }
    *s = out;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builder(limits: Limits) -> ProfileBuilder {
        ProfileBuilder::new(
            "test",
            ProfileKind::Cpu,
            &[
                MetricInfo::new(MetricId::Samples, Measurement::Sampled),
                MetricInfo::new(MetricId::Samples, Measurement::Sampled),
                MetricInfo::new(MetricId::TimeNs, Measurement::Sampled),
            ],
            limits,
        )
        .unwrap()
    }

    #[test]
    fn interns_frames_stacks_and_threads() {
        let mut b = builder(Limits::default());
        assert_eq!(b.metrics().len(), 2, "duplicate metrics are merged");
        let a = b.frame(FrameSpec::new("a").module(Some("app.dll"))).unwrap();
        let a2 = b.frame(FrameSpec::new("a").module(Some("app.dll"))).unwrap();
        let blank = b.frame(FrameSpec::new("  ").module(Some("")).file(Some(""))).unwrap();
        assert_eq!(a, a2);
        assert_ne!(a, blank);
        let s1 = b.stack(&[a, blank]).unwrap();
        let s2 = b.stack_leaf_first(&[blank, a]).unwrap();
        assert_eq!(s1, s2);
        let t = b.thread(Some(7), "").unwrap();
        assert_eq!(b.thread(Some(7), "other").unwrap(), t);
        let n = b.thread(None, "named").unwrap();
        assert_eq!(b.thread(None, "named").unwrap(), n);
        b.rename_thread(t, "main");
        b.rename_thread(99, "ignored");
        b.set_has_time(true)
            .set_name("p")
            .set_started_ms(Some(1000))
            .set_kind(ProfileKind::Mixed);
        b.sample(t, 30, s1, &[1, 10]).unwrap();
        b.sample_with(n, 10, s1, &[(MetricId::Samples, 2), (MetricId::AllocBytes, 9)])
            .unwrap();
        assert!(b.sample(t, 0, s1, &[1]).is_err(), "wrong arity");
        assert!(b.sample(99, 0, s1, &[1, 1]).is_err());
        assert!(b.push(NONE, 999).is_err());
        b.event(ProfileEvent {
            kind: EventKind::Gc,
            thread: Some(99),
            start_ns: 5,
            dur_ns: 100,
            label: "gen0".into(),
            stack: Some(999),
        });
        let p = b.finish().unwrap();
        assert_eq!(p.samples.time, [10, 30], "sorted by time");
        assert_eq!(p.samples.values, [2, 0, 1, 10]);
        assert_eq!(p.duration_ns, 105, "events count for the duration");
        assert_eq!(p.threads[0].name, "main");
        assert_eq!(p.frames[blank as usize].function, "(anonymous)");
        assert_eq!(p.events[0].thread, None);
        assert_eq!(p.events[0].stack, None);
        assert_eq!(p.stack_frames(s1), [a, blank]);
        assert_eq!(p.sample_range(0, 20), 0..1);
        assert_eq!(p.ended_ms(), Some(1000));
        assert_eq!(p.default_metric(), MetricId::TimeNs);
        assert_eq!(p.kind, ProfileKind::Mixed);
    }

    #[test]
    fn limits_are_enforced_with_clear_errors() {
        let limits = Limits {
            max_frames: 2,
            max_stack_nodes: 2,
            max_samples: 1,
            max_threads: 1,
            max_events: 1,
            max_stack_depth: 2,
            ..Limits::default()
        };
        let mut b = builder(limits);
        let a = b.frame(FrameSpec::new("a")).unwrap();
        let c = b.frame(FrameSpec::new("c")).unwrap();
        let e = b.frame(FrameSpec::new("e")).unwrap_err();
        assert!(e.to_string().contains("distinct functions limit exceeded (2)"), "{e}");
        let s = b.stack(&[a, c, a]).unwrap();
        assert!(b.push(s, c).is_err(), "node limit");
        let t = b.thread(Some(1), "x").unwrap();
        assert!(b.thread(Some(2), "y").is_err());
        b.sample(t, 0, s, &[1, 1]).unwrap();
        assert!(b.sample(t, 0, s, &[1, 1]).is_err());
        for _ in 0..3 {
            b.event(ProfileEvent {
                kind: EventKind::Marker,
                thread: None,
                start_ns: 0,
                dur_ns: 0,
                label: "m".into(),
                stack: None,
            });
        }
        let p = b.finish().unwrap();
        assert_eq!(p.events.len(), 1);
        assert_eq!(p.notes.len(), 2, "{:?}", p.notes);
        assert_eq!(p.name, "test");
    }

    #[test]
    fn empty_profiles_and_metrics_are_rejected() {
        assert!(ProfileBuilder::new("x", ProfileKind::Cpu, &[], Limits::default()).is_err());
        assert!(builder(Limits::default()).finish().is_err());
        let mut b = builder(Limits::default());
        let s = b.stack(&[]).unwrap();
        let t = b.thread(None, "").unwrap();
        b.sample(t, 0, s, &[1, 0]).unwrap();
        let p = b.finish().unwrap();
        assert_eq!(p.frames[0].function, "(no stack)");
        assert_eq!(p.threads[0].name, "Thread #0");
        assert_eq!(p.sample_range(5, 1), 0..1, "no time: everything");
    }
}
