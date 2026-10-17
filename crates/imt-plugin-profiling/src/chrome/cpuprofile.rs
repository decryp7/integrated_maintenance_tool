//! Chrome / Node `.cpuprofile` importer (DevTools protocol `Profiler.Profile`:
//! `{nodes, startTime, endTime, samples, timeDeltas}`, times in microseconds
//! of a monotonic clock, so the wall-clock start is unknown).
//!
//! Sample `i` is taken at `startTime + timeDeltas[0..=i]` and, like DevTools,
//! is attributed the time until the next sample (`TimeNs`, sampled; the last
//! sample gets the time to `endTime`, or the average interval); every sample
//! also counts 1 in `Samples`. Profiles without `samples` (very old ones with
//! only `hitCount`s) give `Samples` per node without timestamps.

use super::nodes::{Node, NodeStacks};
use super::{check_size, offset, sample_weights, us_to_ns};
use crate::util;
use imt_profile::{
    Capture, CaptureImporter, Limits, Measurement, MetricId, MetricInfo, Profile, ProfileBuilder, ProfileError, ProfileKind, Result,
};
use serde::Deserialize;

const ID: &str = "cpuprofile";

/// Imports `.cpuprofile` files.
pub struct CpuProfileImporter;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CpuProfile {
    nodes: Vec<Node>,
    #[serde(default)]
    start_time: f64,
    #[serde(default)]
    end_time: f64,
    #[serde(default)]
    samples: Vec<u64>,
    #[serde(default)]
    time_deltas: Vec<f64>,
}

impl CaptureImporter for CpuProfileImporter {
    fn id(&self) -> &'static str {
        ID
    }

    fn name(&self) -> &'static str {
        "Chrome .cpuprofile"
    }

    fn sniff(&self, file_name: &str, head: &[u8]) -> u8 {
        let text = util::head_text(head);
        if !text.starts_with('{') || text.contains("\"traceEvents\"") || text.contains("\"ph\"") || text.contains("\"snapshot\"") {
            return 0;
        }
        let nodes = text.contains("\"nodes\"");
        let call_frame = text.contains("\"callFrame\"");
        if util::extension(file_name) == "cpuprofile" {
            return if nodes { 95 } else { 50 };
        }
        match (nodes && call_frame, text.contains("\"samples\"") || text.contains("\"startTime\"")) {
            (true, true) => 80,
            (true, false) => 60,
            _ => 0,
        }
    }

    fn import(&self, file_name: &str, bytes: &[u8], limits: &Limits) -> Result<Capture> {
        check_size(bytes, limits)?;
        let bytes = util::maybe_gunzip(bytes, limits, ID)?;
        let profile: CpuProfile = util::json(&bytes, ID)?;
        build(profile, file_name, limits).map(Capture::Profile)
    }
}

fn build(p: CpuProfile, file_name: &str, limits: &Limits) -> Result<Profile> {
    if p.nodes.is_empty() {
        return Err(ProfileError::malformed(ID, "the profile has no nodes"));
    }
    let timed = !p.samples.is_empty();
    let metrics: &[MetricInfo] = if timed {
        &[
            MetricInfo::new(MetricId::TimeNs, Measurement::Sampled),
            MetricInfo::new(MetricId::Samples, Measurement::Sampled),
        ]
    } else {
        &[MetricInfo::new(MetricId::Samples, Measurement::Sampled)]
    };
    let mut b = ProfileBuilder::new(ID, ProfileKind::Cpu, metrics, *limits)?;
    b.set_name(imt_profile::frames::basename(file_name));
    let thread = b.thread(None, "main")?;
    let stacks = NodeStacks::resolve(ID, &p.nodes, &mut b)?;
    if stacks.truncated > 0 {
        b.note(format!(
            "{} call tree nodes deeper than the stack depth limit were merged into their caller",
            stacks.truncated
        ));
    }
    if timed {
        let times = sample_times(&p)?;
        let start = us_to_ns(p.start_time);
        let end = us_to_ns(p.end_time);
        let weights = sample_weights(&times, Some(end));
        for ((&id, &t), &w) in p.samples.iter().zip(&times).zip(&weights) {
            let stack = stacks.stack(ID, id, &mut b)?;
            b.sample(thread, offset(t, start), stack, &[w, 1])?;
        }
        b.set_has_time(true);
        b.set_duration_ns(offset(end, start));
    } else {
        for (id, hits) in NodeStacks::hit_counts(&p.nodes) {
            let stack = stacks.stack(ID, id, &mut b)?;
            b.sample(thread, 0, stack, &[hits])?;
        }
        b.note("the profile has no sample timeline (only hit counts per function)");
    }
    b.finish()
}

/// Absolute sample times in ns: `startTime` plus the cumulative deltas, or
/// evenly spread over the profile when there are no deltas.
fn sample_times(p: &CpuProfile) -> Result<Vec<i64>> {
    let start = us_to_ns(p.start_time);
    let n = p.samples.len();
    if p.time_deltas.is_empty() {
        let span = i128::from(us_to_ns(p.end_time)) - i128::from(start);
        let step = (span.max(0) / n as i128) as i64;
        return Ok((0..n).map(|i| start.saturating_add(step.saturating_mul(i as i64))).collect());
    }
    if p.time_deltas.len() != n {
        return Err(ProfileError::malformed(
            ID,
            format!("the profile has {n} samples but {} time deltas", p.time_deltas.len()),
        ));
    }
    let mut t = start;
    Ok(p.time_deltas
        .iter()
        .map(|&d| {
            t = t.saturating_add(us_to_ns(d));
            t
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn import(json: &str) -> Result<Profile> {
        match CpuProfileImporter.import("dir/x.cpuprofile", json.as_bytes(), &Limits::default())? {
            Capture::Profile(p) => Ok(p),
            Capture::Heap(_) => panic!("not a profile"),
        }
    }

    const NODES: &str = r#""nodes":[
        {"id":1,"callFrame":{"functionName":"(root)","scriptId":"0","url":"","lineNumber":-1,"columnNumber":-1},"children":[2,4]},
        {"id":2,"callFrame":{"functionName":"main","scriptId":"1","url":"file:///srv/app.js","lineNumber":0,"columnNumber":0},"children":[3]},
        {"id":3,"callFrame":{"functionName":"work","scriptId":"1","url":"file:///srv/app.js","lineNumber":10,"columnNumber":2}},
        {"id":4,"callFrame":{"functionName":"(garbage collector)","scriptId":"0","url":"","lineNumber":-1,"columnNumber":-1}}]"#;

    fn time_of(p: &Profile, name: &str) -> u64 {
        let ti = p.metric_index(MetricId::TimeNs).unwrap();
        (0..p.samples.len())
            .filter(|&i| p.frames[p.stacks[p.samples.stack[i] as usize].frame as usize].function == name)
            .map(|i| p.samples.values[i * p.metrics.len() + ti])
            .sum()
    }

    #[test]
    fn devtools_time_attribution() {
        let json = format!(r#"{{{NODES},"startTime":1000,"endTime":1100,"samples":[3,3,4,2,1],"timeDeltas":[10,20,30,10,5]}}"#);
        let p = import(&json).unwrap();
        assert_eq!(p.name, "x.cpuprofile");
        assert_eq!(p.kind, ProfileKind::Cpu);
        assert!(p.has_time);
        assert_eq!(p.started_ms, None);
        assert_eq!(p.threads[0].name, "main");
        assert_eq!(p.samples.time, vec![10_000, 30_000, 60_000, 70_000, 75_000]);
        assert_eq!(time_of(&p, "work"), 50_000);
        assert_eq!(time_of(&p, "(garbage collector)"), 10_000);
        assert_eq!(time_of(&p, "main"), 5_000);
        assert_eq!(time_of(&p, "(no stack)"), 25_000, "the root sample gets the rest up to endTime");
        assert_eq!(p.duration_ns, 100_000);
        assert!(p.frames.iter().all(|f| f.function != "(root)"));
        let work = p.samples.stack[0];
        let names: Vec<&str> = p
            .stack_frames(work)
            .iter()
            .map(|&f| p.frames[f as usize].function.as_str())
            .collect();
        assert_eq!(names, ["main", "work"]);
        let w = p.frames.iter().find(|f| f.function == "work").unwrap();
        assert_eq!((w.line, w.module.as_deref()), (Some(11), Some("app.js")));
    }

    #[test]
    fn negative_deltas_missing_deltas_and_hit_counts() {
        let json = format!(r#"{{{NODES},"startTime":0,"endTime":40,"samples":[3,4,3],"timeDeltas":[10,20,-5]}}"#);
        let p = import(&json).unwrap();
        assert_eq!(p.samples.time, vec![10_000, 25_000, 30_000], "sorted by time");
        assert_eq!(time_of(&p, "(garbage collector)"), 10_000);
        let json = format!(r#"{{{NODES},"startTime":0,"endTime":40,"samples":[3,4,3,2]}}"#);
        let p = import(&json).unwrap();
        assert_eq!(p.samples.time, vec![0, 10_000, 20_000, 30_000]);
        let json = r#"{"nodes":[{"id":1,"callFrame":{"functionName":"a"},"hitCount":4},{"id":2,"callFrame":{"functionName":"b"},"hitCount":0,"parent":1}]}"#;
        let p = import(json).unwrap();
        assert!(!p.has_time);
        assert_eq!(p.samples.values, vec![4]);
        assert!(p.notes[0].contains("hit counts"));
    }

    #[test]
    fn errors() {
        let err = |json: &str| import(json).unwrap_err().to_string();
        assert!(err(r#"{"nodes":[]}"#).contains("no nodes"));
        assert!(err(r#"{"samples":[]}"#).contains("invalid JSON"));
        assert!(err(&format!(r#"{{{NODES},"samples":[3],"timeDeltas":[1,2]}}"#)).contains("1 samples but 2 time deltas"));
        assert!(err(&format!(r#"{{{NODES},"samples":[9],"timeDeltas":[1]}}"#)).contains("node 9"));
        assert!(err(r#"{"nodes":[{"id":1,"callFrame":{"functionName":"a"}}]}"#).contains("no samples"));
        assert!(err(r#"{"nodes":[{"id":1,"children":[2]},{"id":2,"children":[1]}],"samples":[1]}"#).contains("ancestor"));
        let small = Limits {
            max_input_bytes: 1,
            ..Limits::default()
        };
        assert!(CpuProfileImporter.import("x", b"{}", &small).is_err());
    }

    #[test]
    fn deep_trees_are_noted() {
        let json = r#"{"nodes":[{"id":1,"callFrame":{"functionName":"a"},"children":[2]},{"id":2,"callFrame":{"functionName":"b"},"children":[3]},
            {"id":3,"callFrame":{"functionName":"c"}}],"samples":[3],"timeDeltas":[1]}"#;
        let limits = Limits {
            max_stack_depth: 2,
            ..Limits::default()
        };
        let Capture::Profile(p) = CpuProfileImporter.import("x", json.as_bytes(), &limits).unwrap() else {
            panic!()
        };
        assert!(p.notes[0].contains("merged"), "{:?}", p.notes);
    }

    #[test]
    fn sniffing() {
        let s = CpuProfileImporter;
        assert_eq!(s.sniff("a.cpuprofile", br#"{"nodes":[{"id":1,"callFrame":{}}]"#), 95);
        assert_eq!(s.sniff("a.cpuprofile", br#"{"startTime":1}"#), 50);
        assert_eq!(s.sniff("a.json", br#" {"nodes":[{"id":1,"callFrame":{}}],"startTime":1"#), 80);
        assert_eq!(s.sniff("a.json", br#"{"nodes":[{"id":1,"callFrame":{"#), 60);
        assert_eq!(s.sniff("a.json", br#"{"nodes":[]}"#), 0);
        assert_eq!(
            s.sniff(
                "a.json",
                br#"{"traceEvents":[{"ph":"P","args":{"data":{"cpuProfile":{"nodes":[{"callFrame":{}}]}}}}]}"#
            ),
            0
        );
        assert_eq!(s.sniff("a.heapsnapshot", br#"{"snapshot":{"meta":{}},"nodes":[]}"#), 0);
        assert_eq!(s.sniff("a.cpuprofile", b"[1]"), 0);
    }
}
