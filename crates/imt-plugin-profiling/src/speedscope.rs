//! speedscope JSON importer.
//!
//! Format: the speedscope file format, schema
//! <https://www.speedscope.app/file-format-schema.json> (defined in
//! `src/lib/file-format-spec.ts` of <https://github.com/jlfwong/speedscope>,
//! MIT). A file has shared frames (`shared.frames[i] = {name, file?, line?,
//! col?}`) and one or more profiles, each `evented` (open/close events at
//! times) or `sampled` (stacks of frame indices, root first, with weights).
//!
//! Every speedscope profile becomes one thread (named after the profile) of a
//! single IMT [`Profile`](imt_profile::Profile):
//!
//! | speedscope | IMT metrics |
//! |---|---|
//! | sampled, time unit | `TimeNs` (sampled) = weight in ns, `Samples` = 1 |
//! | sampled, `bytes` | `AllocBytes` = weight, `Samples` = 1 |
//! | sampled, `none` | `Samples` = weight |
//! | evented | `TimeNs` (exact self time), `Calls`, `MaxCallNs` (see [`Evented`]) |
//!
//! Sampled times are the cumulative weights from the profile's `startValue`
//! (real timestamps only when every profile uses a time unit). Evented
//! profiles with unit `none` are read as nanoseconds; `bytes` is refused.
//! Unbalanced events are tolerated: a close of a frame below the top closes
//! the frames above it first, a close of a frame that is not open is
//! ignored, frames still open at the end close at `endValue`; each case is
//! reported in the profile's notes.

use crate::util;
use imt_profile::{
    Capture, CaptureImporter, FrameKind, FrameSpec, Limits, Measurement, MetricId, MetricInfo, ProfileBuilder, ProfileError, ProfileKind,
    Result, NONE,
};
use serde::Deserialize;
use std::collections::HashMap;

const ID: &str = "speedscope";

/// Imports speedscope JSON files.
pub struct SpeedscopeImporter;

#[derive(Deserialize)]
struct File {
    #[serde(default)]
    shared: Shared,
    #[serde(default)]
    profiles: Vec<SsProfile>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize, Default)]
struct Shared {
    #[serde(default)]
    frames: Vec<SsFrame>,
}

#[derive(Deserialize)]
struct SsFrame {
    name: String,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    line: Option<f64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SsProfile {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    unit: String,
    #[serde(default)]
    start_value: f64,
    #[serde(default)]
    end_value: f64,
    #[serde(default)]
    events: Vec<SsEvent>,
    #[serde(default)]
    samples: Vec<Vec<u32>>,
    #[serde(default)]
    weights: Vec<f64>,
}

#[derive(Deserialize)]
struct SsEvent {
    #[serde(rename = "type")]
    kind: String,
    frame: u32,
    at: f64,
}

/// A speedscope value unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValueUnit {
    None,
    /// Nanoseconds per unit.
    Time(u64),
    Bytes,
}

impl ValueUnit {
    fn parse(unit: &str) -> Option<ValueUnit> {
        Some(match unit {
            "none" | "" => ValueUnit::None,
            "nanoseconds" => ValueUnit::Time(1),
            "microseconds" => ValueUnit::Time(1_000),
            "milliseconds" => ValueUnit::Time(1_000_000),
            "seconds" => ValueUnit::Time(1_000_000_000),
            "bytes" => ValueUnit::Bytes,
            _ => return None,
        })
    }
}

/// Converts a value in some unit to whole nanoseconds (0 for negative or non-finite values).
pub(crate) fn to_ns(value: f64, ns_per_unit: f64) -> u64 {
    let x = value * ns_per_unit;
    if x.is_finite() && x > 0.0 {
        // `as` saturates at u64::MAX.
        x.round() as u64
    } else {
        0
    }
}

/// Whether a source file name looks like managed (JIT/interpreted) code.
fn managed_file(file: &str) -> bool {
    const EXT: &[&str] = &[".js", ".mjs", ".cjs", ".jsx", ".ts", ".tsx", ".mts", ".cts", ".cs", ".java", ".kt"];
    let lower = file.to_ascii_lowercase();
    EXT.iter().any(|e| lower.ends_with(e))
}

impl CaptureImporter for SpeedscopeImporter {
    fn id(&self) -> &'static str {
        ID
    }

    fn name(&self) -> &'static str {
        "speedscope JSON"
    }

    fn sniff(&self, file_name: &str, head: &[u8]) -> u8 {
        let text = util::head_text(head);
        if !text.starts_with('{') {
            return 0;
        }
        let window: String = text.chars().take(4096).collect();
        if window.contains("\"$schema\"") && window.contains("speedscope") {
            return 100;
        }
        let lower = file_name.to_ascii_lowercase();
        if lower.ends_with(".speedscope.json") || lower.ends_with(".speedscope") {
            return 90;
        }
        if text.contains("\"shared\"") && text.contains("\"profiles\"") && text.contains("\"frames\"") {
            return 60;
        }
        0
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
        build(file, file_name, limits).map(Capture::Profile)
    }
}

/// Validated per-profile plan, decided before building (the metric set depends on all profiles).
struct Plan {
    unit: ValueUnit,
    evented: bool,
}

fn plan(p: &SsProfile, index: usize) -> Result<Plan> {
    let unit = ValueUnit::parse(&p.unit)
        .ok_or_else(|| ProfileError::unsupported(ID, format!("profile {index} uses the unknown unit \"{}\"", p.unit)))?;
    let evented = match p.kind.as_str() {
        "evented" => true,
        "sampled" => false,
        other => {
            return Err(ProfileError::unsupported(
                ID,
                format!("profile {index} has the unknown type \"{other}\""),
            ))
        }
    };
    if evented && unit == ValueUnit::Bytes {
        return Err(ProfileError::unsupported(
            ID,
            format!("evented profile {index} measures bytes; only time is supported"),
        ));
    }
    if !evented && !p.weights.is_empty() && p.weights.len() != p.samples.len() {
        return Err(ProfileError::malformed(
            ID,
            format!("profile {index} has {} samples but {} weights", p.samples.len(), p.weights.len()),
        ));
    }
    Ok(Plan { unit, evented })
}

fn build(file: File, file_name: &str, limits: &Limits) -> Result<imt_profile::Profile> {
    if file.profiles.is_empty() {
        return Err(ProfileError::malformed(ID, "the file contains no profiles"));
    }
    let plans: Vec<Plan> = file.profiles.iter().enumerate().map(|(i, p)| plan(p, i)).collect::<Result<_>>()?;
    let any_sampled_time = plans.iter().any(|p| !p.evented && matches!(p.unit, ValueUnit::Time(_)));
    let mut metrics = Vec::new();
    let mut kinds = Vec::new();
    for p in &plans {
        let (kind, ms): (ProfileKind, &[MetricId]) = match (p.evented, p.unit) {
            (true, _) => (ProfileKind::Wall, &[MetricId::TimeNs, MetricId::Calls, MetricId::MaxCallNs]),
            (false, ValueUnit::Time(_)) => (ProfileKind::Cpu, &[MetricId::TimeNs, MetricId::Samples]),
            (false, ValueUnit::Bytes) => (ProfileKind::Alloc, &[MetricId::AllocBytes, MetricId::Samples]),
            (false, ValueUnit::None) => (ProfileKind::Cpu, &[MetricId::Samples]),
        };
        kinds.push(kind);
        for &m in ms {
            let measurement = if p.evented && !(m == MetricId::TimeNs && any_sampled_time) {
                Measurement::Exact
            } else {
                Measurement::Sampled
            };
            if !metrics.iter().any(|x: &MetricInfo| x.id == m) {
                metrics.push(MetricInfo::new(m, measurement));
            }
        }
    }
    let kind = if kinds.iter().all(|k| *k == kinds[0]) {
        kinds[0]
    } else {
        ProfileKind::Mixed
    };
    let mut b = ProfileBuilder::new(ID, kind, &metrics, *limits)?;
    let title = file
        .name
        .as_deref()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| imt_profile::frames::basename(file_name));
    b.set_name(title);
    let all_time = plans
        .iter()
        .all(|p| p.unit != ValueUnit::Bytes && (p.evented || matches!(p.unit, ValueUnit::Time(_))));
    b.set_has_time(all_time);

    // Common time origin: the earliest start of any profile (they share a clock).
    let origin_ns = file
        .profiles
        .iter()
        .zip(&plans)
        .map(|(p, plan)| signed_ns(p.start_value, ns_factor(plan)))
        .min()
        .unwrap_or(0);
    let mut end_ns = 0u64;
    let mut frames = FrameMap::new(&file.shared.frames);
    for (index, (p, plan)) in file.profiles.iter().zip(&plans).enumerate() {
        let name = if p.name.trim().is_empty() {
            format!("Profile {}", index + 1)
        } else {
            p.name.clone()
        };
        let thread = b.thread(None, &format!("{name}\u{0}{index}"))?;
        b.rename_thread(thread, &name);
        let factor = ns_factor(plan);
        let start = rel(signed_ns(p.start_value, factor), origin_ns);
        let end = rel(signed_ns(p.end_value, factor), origin_ns).max(start);
        if plan.evented {
            let mut ev = Evented::new(thread, b.limits().max_stack_depth);
            for e in &p.events {
                let frame = frames.get(&mut b, e.frame)?;
                let at = rel(signed_ns(e.at, factor), origin_ns);
                match e.kind.as_str() {
                    "O" => ev.open(&mut b, frame, at)?,
                    "C" => ev.close(&mut b, frame, at)?,
                    other => {
                        return Err(ProfileError::malformed(
                            ID,
                            format!("profile {index} has an event of unknown type \"{other}\""),
                        ))
                    }
                }
            }
            ev.finish(&mut b, end)?;
            ev.report(&mut b, &name);
            end_ns = end_ns.max(end).max(ev.last());
        } else {
            let mut t = start;
            let mut stack = Vec::new();
            for (i, sample) in p.samples.iter().enumerate() {
                stack.clear();
                for &f in sample {
                    stack.push(frames.get(&mut b, f)?);
                }
                let node = b.stack(&stack)?;
                let w = p.weights.get(i).copied().unwrap_or(1.0);
                let values: Vec<(MetricId, u64)> = match plan.unit {
                    ValueUnit::Time(f) => vec![(MetricId::TimeNs, to_ns(w, f as f64)), (MetricId::Samples, 1)],
                    ValueUnit::Bytes => vec![(MetricId::AllocBytes, to_ns(w, 1.0)), (MetricId::Samples, 1)],
                    ValueUnit::None => vec![(MetricId::Samples, to_ns(w, 1.0))],
                };
                b.sample_with(thread, t, node, &values)?;
                t = t.saturating_add(to_ns(w, factor));
            }
            end_ns = end_ns.max(end).max(t);
        }
    }
    if all_time {
        b.set_duration_ns(end_ns);
    }
    b.finish()
}

/// Nanoseconds per unit used for times; `none` is read as nanoseconds, bytes as a plain count.
fn ns_factor(plan: &Plan) -> f64 {
    match plan.unit {
        ValueUnit::Time(f) => f as f64,
        _ => 1.0,
    }
}

/// A value converted to signed nanoseconds (saturating; non-finite is 0).
fn signed_ns(value: f64, factor: f64) -> i128 {
    let x = (value * factor).round();
    if x.is_finite() {
        // `as` saturates; i64 range is ample for nanosecond clocks.
        i128::from(x as i64)
    } else {
        0
    }
}

/// `ns - origin` clamped to u64.
fn rel(ns: i128, origin: i128) -> u64 {
    u64::try_from((ns - origin).max(0)).unwrap_or(u64::MAX)
}

/// Lazily interns speedscope frames (only those used).
struct FrameMap<'a> {
    frames: &'a [SsFrame],
    ids: Vec<u32>,
}

impl<'a> FrameMap<'a> {
    fn new(frames: &'a [SsFrame]) -> FrameMap<'a> {
        FrameMap {
            frames,
            ids: vec![NONE; frames.len()],
        }
    }

    fn get(&mut self, b: &mut ProfileBuilder, index: u32) -> Result<u32> {
        let i = index as usize;
        let Some(f) = self.frames.get(i) else {
            return Err(ProfileError::malformed(
                ID,
                format!(
                    "a profile refers to frame {index}, but the file has only {} frames",
                    self.frames.len()
                ),
            ));
        };
        if self.ids[i] != NONE {
            return Ok(self.ids[i]);
        }
        let file = f.file.as_deref().filter(|s| !s.is_empty());
        let kind = if file.is_some_and(managed_file) {
            FrameKind::Managed
        } else {
            FrameKind::Unknown
        };
        let line = f
            .line
            .filter(|l| l.is_finite() && *l >= 0.0 && *l <= f64::from(u32::MAX))
            .map(|l| l as u32);
        let id = b.frame(
            FrameSpec::new(&f.name)
                .module(file.map(imt_profile::frames::basename))
                .file(file)
                .line(line)
                .kind(kind),
        )?;
        self.ids[i] = id;
        Ok(id)
    }
}

/// Turns open/close events of one thread into exact metrics:
///
/// - `TimeNs`: self time, the time between consecutive events attributed to
///   the stack open at that moment;
/// - `Calls`: 1 per call, on the called frame's node;
/// - `MaxCallNs`: the call's duration (close − open), on its node.
///
/// Each close emits one sample (self time of the last segment, the call and
/// its duration); each open emits a sample with the caller's self time so
/// far. Times before the previous event are clamped to it. Calls nested
/// deeper than `max_stack_depth` are attributed to their outermost kept
/// ancestor.
pub(crate) struct Evented {
    thread: u32,
    max_depth: usize,
    stack: Vec<Open>,
    /// Open calls per frame, so a close of a frame that is not open is O(1)
    /// (searching the stack each time would be quadratic on hostile input).
    open_count: HashMap<u32, u32>,
    last: u64,
    mismatched: u64,
    stray_closes: u64,
    unclosed: u64,
    too_deep: u64,
    backwards: u64,
}

struct Open {
    frame: u32,
    node: u32,
    at: u64,
}

impl Evented {
    pub(crate) fn new(thread: u32, max_depth: u32) -> Evented {
        Evented {
            thread,
            max_depth: (max_depth as usize).max(1),
            stack: Vec::new(),
            open_count: HashMap::new(),
            last: 0,
            mismatched: 0,
            stray_closes: 0,
            unclosed: 0,
            too_deep: 0,
            backwards: 0,
        }
    }

    /// Time of the latest event.
    pub(crate) fn last(&self) -> u64 {
        self.last
    }

    fn clamp(&mut self, at: u64) -> u64 {
        if at < self.last {
            self.backwards += 1;
            self.last
        } else {
            at
        }
    }

    /// `frame` is called at `at`.
    pub(crate) fn open(&mut self, b: &mut ProfileBuilder, frame: u32, at: u64) -> Result<()> {
        let at = self.clamp(at);
        let parent = self.stack.last().map_or(NONE, |o| o.node);
        if parent != NONE && at > self.last {
            b.sample_with(self.thread, self.last, parent, &[(MetricId::TimeNs, at - self.last)])?;
        }
        self.last = at;
        let node = if self.stack.len() >= self.max_depth {
            self.too_deep += 1;
            parent
        } else {
            b.push(parent, frame)?
        };
        self.stack.push(Open { frame, node, at });
        *self.open_count.entry(frame).or_default() += 1;
        Ok(())
    }

    /// The innermost open call returns at `at`; returns its frame (None when nothing is open).
    pub(crate) fn close_top(&mut self, b: &mut ProfileBuilder, at: u64) -> Result<Option<u32>> {
        let Some(top) = self.stack.pop() else {
            self.stray_closes += 1;
            return Ok(None);
        };
        if let Some(c) = self.open_count.get_mut(&top.frame) {
            *c = c.saturating_sub(1);
        }
        let at = self.clamp(at);
        b.sample_with(
            self.thread,
            self.last,
            top.node,
            &[
                (MetricId::TimeNs, at - self.last),
                (MetricId::Calls, 1),
                (MetricId::MaxCallNs, at.saturating_sub(top.at)),
            ],
        )?;
        self.last = at;
        Ok(Some(top.frame))
    }

    /// `frame` returns at `at`. Frames opened after it that are still open close first;
    /// a close of a frame that is not open is ignored.
    pub(crate) fn close(&mut self, b: &mut ProfileBuilder, frame: u32, at: u64) -> Result<()> {
        // The search below only runs when the frame is open, and everything it
        // passes is popped, so its cost is amortised over the opens.
        let open = self.open_count.get(&frame).is_some_and(|&c| c > 0);
        let Some(pos) = open.then(|| self.stack.iter().rposition(|o| o.frame == frame)).flatten() else {
            self.stray_closes += 1;
            return Ok(());
        };
        let above = self.stack.len() - pos - 1;
        if above > 0 {
            self.mismatched += 1;
        }
        for _ in 0..=above {
            self.close_top(b, at)?;
        }
        Ok(())
    }

    /// Closes every call still open at `end`.
    pub(crate) fn finish(&mut self, b: &mut ProfileBuilder, end: u64) -> Result<()> {
        self.unclosed += self.stack.len() as u64;
        while !self.stack.is_empty() {
            self.close_top(b, end)?;
        }
        Ok(())
    }

    /// Adds notes about anything that had to be repaired.
    pub(crate) fn report(&self, b: &mut ProfileBuilder, thread_name: &str) {
        if self.mismatched > 0 {
            b.note(format!(
                "{thread_name}: {} closes did not match the innermost open call; the calls above them were closed first",
                self.mismatched
            ));
        }
        if self.stray_closes > 0 {
            b.note(format!(
                "{thread_name}: {} closes of calls that were not open were ignored",
                self.stray_closes
            ));
        }
        if self.unclosed > 0 {
            b.note(format!(
                "{thread_name}: {} calls still open at the end were closed there",
                self.unclosed
            ));
        }
        if self.too_deep > 0 {
            b.note(format!(
                "{thread_name}: {} calls nested deeper than {} levels were attributed to their caller",
                self.too_deep, self.max_depth
            ));
        }
        if self.backwards > 0 {
            b.note(format!(
                "{thread_name}: {} events went back in time and were moved to the previous event",
                self.backwards
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_profile::Profile;

    fn import(json: &str) -> Result<Profile> {
        import_with(json, &Limits::default())
    }

    fn import_with(json: &str, limits: &Limits) -> Result<Profile> {
        match SpeedscopeImporter.import("x.speedscope.json", json.as_bytes(), limits)? {
            Capture::Profile(p) => Ok(p),
            Capture::Heap(_) => panic!("not a profile"),
        }
    }

    const FRAMES: &str =
        r#""shared":{"frames":[{"name":"main","file":"/app/main.js","line":3},{"name":"work"},{"name":"leaf","file":"lib.rs"}]}"#;

    /// Sum of a metric per function name (self values).
    fn self_by_name(p: &Profile, m: MetricId) -> std::collections::BTreeMap<String, u64> {
        let mi = p.metric_index(m).unwrap();
        let mut out = std::collections::BTreeMap::new();
        for i in 0..p.samples.len() {
            let node = p.samples.stack[i];
            let f = &p.frames[p.stacks[node as usize].frame as usize].function;
            m.fold(out.entry(f.clone()).or_insert(0), p.samples.values[i * p.metrics.len() + mi]);
        }
        out
    }

    #[test]
    fn sampled_time_units() {
        let json = format!(
            r#"{{"$schema":"https://www.speedscope.app/file-format-schema.json","name":"orders",{FRAMES},
            "profiles":[{{"type":"sampled","name":"T1","unit":"milliseconds","startValue":10,"endValue":20,
              "samples":[[0,1],[0,1,2],[0]],"weights":[2,3,1.5]}}]}}"#
        );
        let p = import(&json).unwrap();
        assert_eq!(p.name, "orders");
        assert_eq!(p.kind, ProfileKind::Cpu);
        assert!(p.has_time);
        assert_eq!(p.threads[0].name, "T1");
        assert_eq!(p.samples.time, vec![0, 2_000_000, 5_000_000]);
        let t = self_by_name(&p, MetricId::TimeNs);
        assert_eq!(t["work"], 2_000_000);
        assert_eq!(t["leaf"], 3_000_000);
        assert_eq!(t["main"], 1_500_000);
        assert_eq!(self_by_name(&p, MetricId::Samples)["main"], 1);
        assert_eq!(p.duration_ns, 10_000_000);
        let main = p.frames.iter().find(|f| f.function == "main").unwrap();
        assert_eq!(main.kind, FrameKind::Managed);
        assert_eq!(main.module.as_deref(), Some("main.js"));
        assert_eq!(main.line, Some(3));
        assert_eq!(p.frames.iter().find(|f| f.function == "leaf").unwrap().kind, FrameKind::Unknown);
        assert_eq!(p.metrics[0].measurement, Measurement::Sampled);
    }

    #[test]
    fn units_and_mixed_profiles() {
        for (unit, f) in [("nanoseconds", 1), ("microseconds", 1_000), ("seconds", 1_000_000_000)] {
            let json = format!(
                r#"{{{FRAMES},"profiles":[{{"type":"sampled","unit":"{unit}","startValue":0,"endValue":1,"samples":[[0]],"weights":[2]}}]}}"#
            );
            let p = import(&json).unwrap();
            assert_eq!(self_by_name(&p, MetricId::TimeNs)["main"], 2 * f, "{unit}");
            assert_eq!(p.threads[0].name, "Profile 1");
        }
        let json = format!(
            r#"{{{FRAMES},"profiles":[{{"type":"sampled","name":"heap","unit":"bytes","startValue":0,"endValue":0,"samples":[[0],[1]],"weights":[100,28]}}]}}"#
        );
        let p = import(&json).unwrap();
        assert_eq!(p.kind, ProfileKind::Alloc);
        assert!(!p.has_time);
        assert_eq!(self_by_name(&p, MetricId::AllocBytes)["main"], 100);
        let json = format!(
            r#"{{{FRAMES},"profiles":[{{"type":"sampled","name":"n","unit":"none","startValue":0,"endValue":0,"samples":[[0],[1]],"weights":[7,1]}},
            {{"type":"sampled","name":"heap","unit":"bytes","startValue":0,"endValue":0,"samples":[[2]],"weights":[64]}}]}}"#
        );
        let p = import(&json).unwrap();
        assert_eq!(p.kind, ProfileKind::Mixed);
        assert_eq!(self_by_name(&p, MetricId::Samples)["main"], 7);
        assert_eq!(p.threads.len(), 2);
        // Missing weights count 1 each.
        let json = format!(r#"{{{FRAMES},"profiles":[{{"type":"sampled","unit":"none","samples":[[0],[0]]}}]}}"#);
        assert_eq!(self_by_name(&import(&json).unwrap(), MetricId::Samples)["main"], 2);
        // Two profiles with the same name stay two threads.
        let json = format!(
            r#"{{{FRAMES},"profiles":[{{"type":"sampled","name":"a","unit":"none","samples":[[0]]}},{{"type":"sampled","name":"a","unit":"none","samples":[[0]]}}]}}"#
        );
        let p = import(&json).unwrap();
        assert_eq!(p.threads.len(), 2);
        assert!(p.threads.iter().all(|t| t.name == "a"));
    }

    #[test]
    fn evented_nesting_gives_exact_self_time_calls_and_max() {
        // main [0,100]: work [10,40] (leaf [20,30]), work [50,60]
        let json = format!(
            r#"{{{FRAMES},"profiles":[{{"type":"evented","name":"main thread","unit":"microseconds","startValue":0,"endValue":100,"events":[
            {{"type":"O","frame":0,"at":0}},{{"type":"O","frame":1,"at":10}},{{"type":"O","frame":2,"at":20}},{{"type":"C","frame":2,"at":30}},
            {{"type":"C","frame":1,"at":40}},{{"type":"O","frame":1,"at":50}},{{"type":"C","frame":1,"at":60}},{{"type":"C","frame":0,"at":100}}]}}]}}"#
        );
        let p = import(&json).unwrap();
        assert_eq!(p.kind, ProfileKind::Wall);
        assert!(p.has_time);
        assert!(p.metrics.iter().all(|m| m.measurement == Measurement::Exact));
        let t = self_by_name(&p, MetricId::TimeNs);
        assert_eq!(t["main"], 60_000);
        assert_eq!(t["work"], 30_000);
        assert_eq!(t["leaf"], 10_000);
        let c = self_by_name(&p, MetricId::Calls);
        assert_eq!((c["main"], c["work"], c["leaf"]), (1, 2, 1));
        let m = self_by_name(&p, MetricId::MaxCallNs);
        assert_eq!((m["main"], m["work"], m["leaf"]), (100_000, 30_000, 10_000));
        assert_eq!(p.duration_ns, 100_000);
        assert!(p.notes.is_empty(), "{:?}", p.notes);
        // leaf sits under work under main.
        let leaf_node = (0..p.samples.len())
            .map(|i| p.samples.stack[i])
            .find(|&s| p.frames[p.stacks[s as usize].frame as usize].function == "leaf")
            .unwrap();
        let names: Vec<&str> = p
            .stack_frames(leaf_node)
            .iter()
            .map(|&f| p.frames[f as usize].function.as_str())
            .collect();
        assert_eq!(names, ["main", "work", "leaf"]);
    }

    #[test]
    fn unbalanced_events_are_repaired_and_noted() {
        // Close of main while work is open; a stray close; an unclosed frame; time going backwards.
        let json = format!(
            r#"{{{FRAMES},"profiles":[{{"type":"evented","name":"t","unit":"none","startValue":0,"endValue":50,"events":[
            {{"type":"O","frame":0,"at":0}},{{"type":"O","frame":1,"at":10}},{{"type":"C","frame":0,"at":20}},
            {{"type":"C","frame":2,"at":21}},{{"type":"O","frame":2,"at":30}},{{"type":"O","frame":1,"at":25}}]}}]}}"#
        );
        let p = import(&json).unwrap();
        let notes = p.notes.join("\n");
        assert!(notes.contains("did not match"), "{notes}");
        assert!(notes.contains("not open were ignored"), "{notes}");
        assert!(notes.contains("still open at the end"), "{notes}");
        assert!(notes.contains("back in time"), "{notes}");
        let c = self_by_name(&p, MetricId::Calls);
        assert_eq!(c["work"], 2);
        assert_eq!(self_by_name(&p, MetricId::MaxCallNs)["leaf"], 20);
        assert_eq!(p.duration_ns, 50);
    }

    #[test]
    fn deep_evented_nesting_is_capped() {
        let mut events = String::new();
        for i in 0..10 {
            events.push_str(&format!(r#"{{"type":"O","frame":0,"at":{i}}},"#));
        }
        events.push_str(r#"{"type":"C","frame":0,"at":20}"#);
        let json = format!(r#"{{{FRAMES},"profiles":[{{"type":"evented","unit":"seconds","endValue":30,"events":[{events}]}}]}}"#);
        let limits = Limits {
            max_stack_depth: 3,
            ..Limits::default()
        };
        let p = import_with(&json, &limits).unwrap();
        assert!(p.notes.iter().any(|n| n.contains("deeper than 3")), "{:?}", p.notes);
        assert!(p.stacks.len() <= 3);
        assert_eq!(self_by_name(&p, MetricId::Calls)["main"], 10);
    }

    #[test]
    fn mixed_evented_and_sampled_time_is_labelled_sampled() {
        let json = format!(
            r#"{{{FRAMES},"profiles":[{{"type":"evented","unit":"milliseconds","endValue":2,"events":[{{"type":"O","frame":0,"at":0}}]}},
            {{"type":"sampled","unit":"milliseconds","samples":[[1]],"weights":[1]}}]}}"#
        );
        let p = import(&json).unwrap();
        assert_eq!(p.kind, ProfileKind::Mixed);
        let time = p.metrics.iter().find(|m| m.id == MetricId::TimeNs).unwrap();
        assert_eq!(time.measurement, Measurement::Sampled);
        assert_eq!(
            p.metrics.iter().find(|m| m.id == MetricId::Calls).unwrap().measurement,
            Measurement::Exact
        );
    }

    #[test]
    fn errors_say_what_is_wrong() {
        let err = |json: &str| import(json).unwrap_err().to_string();
        assert!(err("{").contains("invalid JSON"));
        assert!(err(r#"{"profiles":[]}"#).contains("no profiles"));
        assert!(err(&format!(r#"{{{FRAMES},"profiles":[{{"type":"weird","unit":"none"}}]}}"#)).contains("unknown type"));
        assert!(err(&format!(r#"{{{FRAMES},"profiles":[{{"type":"sampled","unit":"furlongs"}}]}}"#)).contains("unknown unit"));
        assert!(err(&format!(r#"{{{FRAMES},"profiles":[{{"type":"evented","unit":"bytes"}}]}}"#)).contains("bytes"));
        assert!(err(&format!(
            r#"{{{FRAMES},"profiles":[{{"type":"sampled","unit":"none","samples":[[0]],"weights":[1,2]}}]}}"#
        ))
        .contains("1 samples but 2 weights"));
        assert!(err(&format!(
            r#"{{{FRAMES},"profiles":[{{"type":"sampled","unit":"none","samples":[[7]]}}]}}"#
        ))
        .contains("refers to frame 7"));
        assert!(err(&format!(
            r#"{{{FRAMES},"profiles":[{{"type":"evented","unit":"none","events":[{{"type":"X","frame":0,"at":0}}]}}]}}"#
        ))
        .contains("unknown type \"X\""));
        assert!(err(&format!(r#"{{{FRAMES},"profiles":[{{"type":"sampled","unit":"none"}}]}}"#)).contains("no samples"));
        let small = Limits {
            max_input_bytes: 4,
            ..Limits::default()
        };
        assert!(matches!(
            import_with("{\"profiles\":[]}", &small),
            Err(ProfileError::LimitExceeded { .. })
        ));
        let few = Limits {
            max_samples: 1,
            ..Limits::default()
        };
        let json = format!(r#"{{{FRAMES},"profiles":[{{"type":"sampled","unit":"none","samples":[[0],[1]]}}]}}"#);
        assert!(matches!(
            import_with(&json, &few),
            Err(ProfileError::LimitExceeded { what: "samples", .. })
        ));
    }

    #[test]
    fn values_convert_safely() {
        assert_eq!(to_ns(-1.0, 1.0), 0);
        assert_eq!(to_ns(f64::NAN, 1.0), 0);
        assert_eq!(to_ns(1e300, 1.0), u64::MAX);
        assert_eq!(to_ns(1e300, 1e300), 0, "infinite");
        assert_eq!(to_ns(1.4, 1.0), 1);
        assert_eq!(signed_ns(f64::INFINITY, 1.0), 0);
        assert_eq!(rel(5, 10), 0);
        assert!(managed_file("a/B.TS"));
        assert!(!managed_file("a.rs"));
    }

    #[test]
    fn sniffing() {
        let s = SpeedscopeImporter;
        assert_eq!(
            s.sniff("p.json", br#"  {"$schema": "https://www.speedscope.app/file-format-schema.json","#),
            100
        );
        assert_eq!(s.sniff("p.speedscope.json", br#"{"version":"1"}"#), 90);
        assert_eq!(s.sniff("p.json", br#"{"shared":{"frames":[]},"profiles":[]}"#), 60);
        assert_eq!(s.sniff("p.json", br#"{"nodes":[{"callFrame":{}}],"samples":[]}"#), 0);
        assert_eq!(s.sniff("p.speedscope.json", b"a;b 1\n"), 0);
        assert_eq!(s.sniff("p.json", b""), 0);
    }
}
