//! Automatic findings over a profile: what an expert would look at first,
//! each with a link to the evidence (a view and a selection). A finding is
//! only emitted when the data supports it; thresholds are configurable.

use crate::api::{
    DiffSort, Evidence, EvidenceView, FindingKind, FindingSeverity, FindingThresholds, FunctionDiffRequest, GroupBy, HotPathRequest,
    ProfileFinding, ProfileView, SortSpec, TimeRange, TreeDirection, ValueOf,
};
use crate::error::Result;
use crate::model::{EventKind, MetricId, Profile, NONE};
use crate::ops::common::{frame_dto, pct};
use crate::ops::diff::{function_diff, Side};
use crate::ops::hotspots::{group, sort_groups};
use crate::ops::nav::hot_path;
use crate::ops::tree::CallTree;
use rustc_hash::FxHashMap;

/// Garbage collector work (lower-case substrings of function names).
const GC: &[&str] = &[
    "(garbage collector)",
    "gc_heap::",
    "wks::gc",
    "svr::gc",
    "garbagecollect",
    "system.gc.collect",
    "runtime.gcbgmarkworker",
    "runtime.gcdrain",
    "runtime.gcstart",
    "runtime.gcassistalloc",
    "runtime.markroot",
    "v8::internal::heap::collectgarbage",
    "v8::internal::scavenger",
    "v8::internal::markcompactcollector",
    "jvm.gc",
];

/// Waiting for a lock.
const LOCKS: &[&str] = &[
    "monitor.enter",
    "monitor.reliableenter",
    "monitor.tryenter",
    "spinlock.enter",
    "semaphoreslim.wait",
    "mutex.waitone",
    "readerwriterlock",
    "awaitlock",
    "pthread_mutex_lock",
    "pthread_rwlock",
    "__lll_lock_wait",
    "futex_wait",
    "sync.(*mutex).lock",
    "sync.(*rwmutex)",
    "entercriticalsection",
    "acquiresrwlock",
    "std::mutex::lock",
    "parking_lot",
];

/// Throwing and unwinding exceptions.
const THROWS: &[&str] = &[
    "__cxa_throw",
    "_unwind_raiseexception",
    "raiseexception",
    "_cxxthrowexception",
    "il_throw",
    "il_rethrow",
    "dispatchmanagedexception",
    "exceptiondispatchinfo.throw",
    "throwhelper.",
    "exceptiontracker",
];

/// A thread blocked waiting for something (a pool thread doing this starves the pool).
const BLOCKING: &[&str] = &[
    "task.wait",
    "task.internalwait",
    "taskawaiter.getresult",
    "taskawaiter`1.getresult",
    "get_result",
    "monitor.wait",
    "waithandle.waitone",
    "waithandle.waitany",
    "waithandle.waitall",
    "manualreseteventslim.wait",
    "semaphoreslim.wait",
    "thread.sleep",
    "waitforsingleobject",
];

const SYNC_OVER_ASYNC: &[&str] = &["task.wait", "task.internalwait", "getresult", "get_result"];

const POOL_THREADS: &[&str] = &[
    ".net threadpool worker",
    ".net tp worker",
    "threadpool",
    "thread pool",
    "worker thread",
    "pool-",
];

/// Frames that are not work (excluded from "top self time").
const IDLE: &[&str] = &["(idle)", "(program)", "(root)", "(no stack)"];

fn matches_any(name: &str, patterns: &[&str]) -> bool {
    let n = name.to_lowercase();
    patterns.iter().any(|p| n.contains(p))
}

fn frame_hits(profile: &Profile, patterns: &[&str]) -> Vec<bool> {
    profile.frames.iter().map(|f| matches_any(&f.function, patterns)).collect()
}

/// Value under matching frames (nested matches once) and the heaviest matching node.
fn under(tree: &CallTree, k: usize, hits: &[bool]) -> (u64, Option<u32>) {
    let mut sum = 0u64;
    let mut best: Option<u32> = None;
    let mut stack: Vec<u32> = tree.roots.clone();
    while let Some(n) = stack.pop() {
        if hits[tree.frame[n as usize] as usize] {
            let v = tree.total_of(n, k);
            sum = sum.saturating_add(v);
            if best.is_none_or(|b| tree.total_of(b, k) < v) {
                best = Some(n);
            }
            continue;
        }
        stack.extend_from_slice(tree.children(n));
    }
    (sum, best)
}

/// Everything a finding needs to know about where it came from.
pub struct Context<'a> {
    pub profile_id: u32,
    pub baseline_id: Option<u32>,
    pub view: &'a ProfileView,
    pub thresholds: &'a FindingThresholds,
}

struct Builder<'a> {
    ctx: &'a Context<'a>,
    metric: MetricId,
    out: Vec<ProfileFinding>,
}

impl Builder<'_> {
    fn evidence(&self, view: EvidenceView) -> Evidence {
        Evidence {
            view,
            profile_id: self.ctx.profile_id,
            baseline_id: None,
            frame: None,
            path: Vec::new(),
            metric: self.metric,
            range: self.ctx.view.range,
            threads: self.ctx.view.threads.clone(),
            event_kind: None,
        }
    }

    fn push(
        &mut self,
        kind: FindingKind,
        severity: FindingSeverity,
        title: String,
        detail: String,
        value_pct: Option<f64>,
        evidence: Evidence,
    ) {
        self.out.push(ProfileFinding {
            kind,
            severity,
            title,
            detail,
            value_pct,
            evidence,
        });
    }
}

fn ms(ns: u64) -> String {
    format!("{:.1} ms", ns as f64 / 1e6)
}

/// All findings for `target` (and regressions against `baseline`), most severe first.
pub fn findings(target: &Side<'_>, baseline: Option<&Side<'_>>, ctx: &Context<'_>) -> Result<Vec<ProfileFinding>> {
    let profile = target.profile;
    let tree = target.tree;
    let metric = profile.default_metric();
    let Some(k) = tree.metrics.iter().position(|m| *m == metric) else {
        return Ok(Vec::new());
    };
    let grand = tree.grand[k];
    let th = ctx.thresholds;
    let mut b = Builder {
        ctx,
        metric,
        out: Vec::new(),
    };
    if grand > 0 {
        top_self(&mut b, target, k, grand, th);
        gc(&mut b, target, k, grand, th);
        locks(&mut b, target, k, grand, th);
        exceptions(&mut b, target, k, grand, th);
        starvation(&mut b, target, th);
    }
    if let Some(base) = baseline {
        regressions(&mut b, base, target, th)?;
    }
    let mut out = b.out;
    out.sort_by(|a, b| {
        b.severity
            .cmp(&a.severity)
            .then_with(|| crate::ops::hotspots::cmp_f64(b.value_pct.unwrap_or(0.0), a.value_pct.unwrap_or(0.0)))
    });
    Ok(out)
}

fn top_self(b: &mut Builder<'_>, s: &Side<'_>, k: usize, grand: u64, th: &FindingThresholds) {
    let groups = group(s.profile, s.tree, s.view, GroupBy::Function);
    let mut idx: Vec<usize> = (0..groups.len())
        .filter(|&g| groups.touched[g] && !matches_any(&groups.names[g], IDLE))
        .collect();
    let sort = SortSpec {
        metric: Some(b.metric),
        of: ValueOf::SelfValue,
        descending: Some(true),
    };
    if sort_groups(&groups, &mut idx, &sort).is_err() {
        return;
    }
    for g in idx.into_iter().take(th.top_self_count) {
        let share = pct(groups.self_of(g, k), grand);
        if share < th.top_self_pct {
            break;
        }
        let frame = groups.frame[g];
        let path = frame
            .and_then(|f| s.tree.nodes_of_frame(f).iter().copied().max_by_key(|&n| s.tree.self_of(n, k)))
            .map(|n| s.tree.path_of(n))
            .unwrap_or_default();
        let mut e = b.evidence(EvidenceView::Butterfly);
        e.frame = frame.map(|f| frame_dto(s.profile, f));
        e.path = path;
        let severity = if share >= 15.0 {
            FindingSeverity::Warning
        } else {
            FindingSeverity::Info
        };
        b.push(
            FindingKind::TopSelf,
            severity,
            format!("{} spends {share:.1}% of the time in itself", groups.names[g]),
            format!(
                "Self time is time in the function's own code, not in what it calls. Optimise its body, or call it less often (see its callers).{}",
                if s.profile.metrics[k].measurement == crate::model::Measurement::Sampled {
                    " Values are estimates from samples."
                } else {
                    ""
                }
            ),
            Some(share),
            e,
        );
    }
}

fn gc(b: &mut Builder<'_>, s: &Side<'_>, k: usize, grand: u64, th: &FindingThresholds) {
    let gc_events: Vec<_> = s
        .profile
        .events
        .iter()
        .filter(|e| e.kind == EventKind::Gc && e.dur_ns > 0)
        .collect();
    if !gc_events.is_empty() && s.profile.duration_ns > 0 {
        let (from, to) = s.view.range.unwrap_or((0, s.profile.duration_ns));
        let paused: u64 = gc_events
            .iter()
            .map(|e| e.start_ns.saturating_add(e.dur_ns).min(to).saturating_sub(e.start_ns.max(from)))
            .sum();
        let share = pct(paused, (to - from).max(1));
        if share >= th.gc_pct {
            let mut e = b.evidence(EvidenceView::Timeline);
            e.event_kind = Some(EventKind::Gc);
            b.push(
                FindingKind::GcPressure,
                if share >= 2.0 * th.gc_pct { FindingSeverity::Critical } else { FindingSeverity::Warning },
                format!("Garbage collection took {share:.1}% of the time"),
                format!(
                    "{} collections paused for {} in total. Reduce allocations on hot paths (pool or reuse buffers, avoid boxing and LINQ in loops) or check for a leak growing the heap.",
                    gc_events.len(),
                    ms(paused)
                ),
                Some(share),
                e,
            );
        }
        return;
    }
    let hits = frame_hits(s.profile, GC);
    let (value, best) = under(s.tree, k, &hits);
    let share = pct(value, grand);
    if share >= th.gc_pct {
        let mut e = b.evidence(EvidenceView::Butterfly);
        if let Some(n) = best {
            e.frame = Some(frame_dto(s.profile, s.tree.frame[n as usize]));
            e.path = s.tree.path_of(n);
        }
        b.push(
            FindingKind::GcPressure,
            if share >= 2.0 * th.gc_pct { FindingSeverity::Critical } else { FindingSeverity::Warning },
            format!("Garbage collection takes {share:.1}% of the samples"),
            "The garbage collector's own frames are hot. Reduce allocations on hot paths (see an allocation profile) or check for a leak growing the heap.".into(),
            Some(share),
            e,
        );
    }
}

fn locks(b: &mut Builder<'_>, s: &Side<'_>, k: usize, grand: u64, th: &FindingThresholds) {
    let waits: Vec<_> = s.profile.events.iter().filter(|e| e.kind == EventKind::LockWait).collect();
    if !waits.is_empty() && s.profile.duration_ns > 0 {
        let total: u64 = waits.iter().map(|e| e.dur_ns).fold(0, u64::saturating_add);
        let threads = s.profile.threads.len().max(1) as u64;
        let share = pct(total, s.profile.duration_ns.saturating_mul(threads)).min(100.0);
        if share >= th.lock_pct || total >= s.profile.duration_ns / 20 {
            let mut e = b.evidence(EvidenceView::Timeline);
            e.event_kind = Some(EventKind::LockWait);
            b.push(
                FindingKind::LockContention,
                FindingSeverity::Warning,
                format!("Threads waited {} for locks", ms(total)),
                format!(
                    "{} lock waits ({share:.1}% of all threads' time). Shorten the locked sections, use finer-grained or lock-free structures, or avoid sharing the state.",
                    waits.len()
                ),
                Some(share),
                e,
            );
            return;
        }
    }
    let hits = frame_hits(s.profile, LOCKS);
    let (value, best) = under(s.tree, k, &hits);
    let share = pct(value, grand);
    if share >= th.lock_pct {
        let mut e = b.evidence(EvidenceView::Butterfly);
        if let Some(n) = best {
            e.frame = Some(frame_dto(s.profile, s.tree.frame[n as usize]));
            e.path = s.tree.path_of(n);
        }
        let name = best
            .map(|n| s.profile.frames[s.tree.frame[n as usize] as usize].function.clone())
            .unwrap_or_default();
        b.push(
            FindingKind::LockContention,
            if share >= 3.0 * th.lock_pct {
                FindingSeverity::Critical
            } else {
                FindingSeverity::Warning
            },
            format!("Lock contention: {share:.1}% of the samples wait for a lock"),
            format!("Most of it in {name}. The callers show which lock: shorten what runs under it or stop sharing that state."),
            Some(share),
            e,
        );
    }
}

fn exceptions(b: &mut Builder<'_>, s: &Side<'_>, k: usize, grand: u64, th: &FindingThresholds) {
    if let Some(ke) = s.tree.metrics.iter().position(|m| *m == MetricId::Exceptions) {
        let count = s.tree.grand[ke];
        if count >= th.exception_count.max(1) {
            let req = HotPathRequest {
                profile_id: b.ctx.profile_id,
                view: b.ctx.view.clone(),
                direction: TreeDirection::TopDown,
                path: Vec::new(),
                metric: Some(MetricId::Exceptions),
                min_fraction: Some(0.2),
            };
            let path = hot_path(s.profile, s.tree, &req).map(|h| h.path).unwrap_or_default();
            let thrower = path.last().map(|&f| frame_dto(s.profile, f));
            let mut e = b.evidence(EvidenceView::CallTree);
            e.metric = MetricId::Exceptions;
            e.frame = thrower.clone();
            e.path = path;
            let rate = if s.profile.duration_ns > 0 {
                format!(" ({:.1}/s)", count as f64 * 1e9 / s.profile.duration_ns as f64)
            } else {
                String::new()
            };
            b.push(
                FindingKind::ExceptionHotPath,
                if count >= 100 { FindingSeverity::Warning } else { FindingSeverity::Info },
                format!("{count} exceptions thrown{rate}"),
                format!(
                    "Most are thrown along the highlighted path{}. Exceptions are expensive: validate instead of catching, and fix the cause of recurring ones.",
                    thrower.map(|f| format!(", ending in {}", f.name)).unwrap_or_default()
                ),
                None,
                e,
            );
            return;
        }
    }
    let thrown: Vec<_> = s.profile.events.iter().filter(|e| e.kind == EventKind::Exception).collect();
    if thrown.len() as u64 >= th.exception_count.max(1) {
        let mut by_label: FxHashMap<&str, usize> = FxHashMap::default();
        for e in &thrown {
            *by_label.entry(e.label.as_str()).or_default() += 1;
        }
        let (top, n) = by_label
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0)))
            .unwrap_or(("", 0));
        let mut e = b.evidence(EvidenceView::Timeline);
        e.event_kind = Some(EventKind::Exception);
        b.push(
            FindingKind::ExceptionHotPath,
            FindingSeverity::Info,
            format!("{} exceptions thrown", thrown.len()),
            format!("Most often {top} ({n}×). Fix the cause of recurring exceptions; they are expensive."),
            None,
            e,
        );
        return;
    }
    let hits = frame_hits(s.profile, THROWS);
    let (value, best) = under(s.tree, k, &hits);
    let share = pct(value, grand);
    if share >= th.exception_pct {
        let mut e = b.evidence(EvidenceView::Butterfly);
        if let Some(n) = best {
            e.frame = Some(frame_dto(s.profile, s.tree.frame[n as usize]));
            e.path = s.tree.path_of(n);
        }
        b.push(
            FindingKind::ExceptionHotPath,
            FindingSeverity::Warning,
            format!("Throwing exceptions takes {share:.1}% of the samples"),
            "Exception machinery is on a hot path. The callers show who throws; avoid exceptions for control flow.".into(),
            Some(share),
            e,
        );
    }
}

fn starvation(b: &mut Builder<'_>, s: &Side<'_>, th: &FindingThresholds) {
    let p = s.profile;
    if !p.has_time {
        return;
    }
    let pool: Vec<bool> = p.threads.iter().map(|t| matches_any(&t.name, POOL_THREADS)).collect();
    if pool.iter().filter(|&&x| x).count() < 4 {
        return;
    }
    // Per stack node: a blocking frame is on the stack (parents come first).
    let block_hits = frame_hits(p, BLOCKING);
    let sync_hits = frame_hits(p, SYNC_OVER_ASYNC);
    let mut blocked = vec![false; p.stacks.len()];
    let mut sync = vec![false; p.stacks.len()];
    for (i, node) in p.stacks.iter().enumerate() {
        let (pb, ps) = if node.parent == NONE {
            (false, false)
        } else {
            (blocked[node.parent as usize], sync[node.parent as usize])
        };
        blocked[i] = pb || block_hits[node.frame as usize];
        sync[i] = ps || sync_hits[node.frame as usize];
    }
    let (mut on_pool, mut on_pool_blocked, mut sync_blocked) = (0u64, 0u64, 0u64);
    let mut first_seen: FxHashMap<u32, u64> = FxHashMap::default();
    for i in s.view.sample_indices(p) {
        let t = p.samples.thread[i];
        if !pool[t as usize] {
            continue;
        }
        first_seen.entry(t).or_insert(p.samples.time[i]);
        on_pool += 1;
        let st = p.samples.stack[i] as usize;
        if blocked[st] {
            on_pool_blocked += 1;
            if sync[st] {
                sync_blocked += 1;
            }
        }
    }
    let share = pct(on_pool_blocked, on_pool);
    if first_seen.len() < 4 || share < th.starvation_blocked_pct {
        return;
    }
    // Thread injection: pool threads that only appear well after the start.
    let start = s.view.range.map(|r| r.0).unwrap_or(0);
    let span = s.view.range.map(|r| r.1 - r.0).unwrap_or(p.duration_ns).max(1);
    let late = first_seen.values().filter(|&&t| t - start.min(t) > span / 10).count();
    let mut detail = format!(
        "{share:.0}% of the samples on {} pool threads are blocked waiting.",
        first_seen.len()
    );
    if late >= 3 {
        detail.push_str(&format!(
            " {late} pool threads were added during the capture, as the pool does when its threads are blocked."
        ));
    }
    if sync_blocked * 2 >= on_pool_blocked && sync_blocked > 0 {
        detail.push_str(" Most waits are on tasks (.Wait(), .Result, GetResult()): sync over async. Await them instead.");
    } else {
        detail.push_str(" Avoid blocking calls on pool threads; await asynchronous work instead.");
    }
    let mut e = b.evidence(EvidenceView::Timeline);
    e.threads = Some(
        first_seen
            .keys()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect(),
    );
    b.push(
        FindingKind::ThreadPoolStarvation,
        if late >= 3 {
            FindingSeverity::Critical
        } else {
            FindingSeverity::Warning
        },
        "Thread pool starvation likely".into(),
        detail,
        Some(share),
        e,
    );
}

fn regressions(b: &mut Builder<'_>, base: &Side<'_>, target: &Side<'_>, th: &FindingThresholds) -> Result<()> {
    let req = FunctionDiffRequest {
        baseline_id: b.ctx.baseline_id.unwrap_or(0),
        target_id: b.ctx.profile_id,
        view: b.ctx.view.clone(),
        group_by: GroupBy::Function,
        metric: None,
        sort: DiffSort::DeltaSelf,
        search: None,
        offset: 0,
        limit: 5,
    };
    let diff = function_diff(base, target, &req)?;
    for row in diff.rows.iter().filter(|r| r.delta_self_pct >= th.regression_pct) {
        let mut e = b.evidence(EvidenceView::Diff);
        e.baseline_id = b.ctx.baseline_id;
        e.metric = diff.metric;
        e.frame = row.target_frame.map(|f| frame_dto(target.profile, f));
        b.push(
            FindingKind::Regression,
            if row.delta_self_pct >= 3.0 * th.regression_pct {
                FindingSeverity::Critical
            } else {
                FindingSeverity::Warning
            },
            format!("{} regressed by {:.1} points", row.name, row.delta_self_pct),
            format!(
                "Its self share grew from {:.1}% to {:.1}% of the total{}.",
                row.baseline.self_pct,
                row.target.self_pct,
                if row.status == crate::api::DiffStatus::New {
                    " (new in this profile)"
                } else {
                    ""
                }
            ),
            Some(row.delta_self_pct),
            e,
        );
    }
    Ok(())
}

/// The range a finding refers to (for callers that only have a profile).
pub fn whole(profile: &Profile) -> TimeRange {
    TimeRange {
        from_ns: 0,
        to_ns: profile.duration_ns,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::{FrameSpec, ProfileBuilder};
    use crate::limits::Limits;
    use crate::model::{Measurement, MetricInfo, ProfileEvent, ProfileKind};
    use crate::ops::tree::tests::profile;
    use crate::ops::tree::CompiledView;

    fn run(p: &Profile, base: Option<&Profile>) -> Vec<ProfileFinding> {
        let v = CompiledView::all();
        let t = CallTree::build(p, &v);
        let side = Side {
            profile: p,
            tree: &t,
            view: &v,
        };
        let bt = base.map(|b| CallTree::build(b, &v));
        let bside = base.map(|b| Side {
            profile: b,
            tree: bt.as_ref().unwrap(),
            view: &v,
        });
        let view = ProfileView::default();
        let th = FindingThresholds::default();
        let ctx = Context {
            profile_id: 1,
            baseline_id: base.map(|_| 0),
            view: &view,
            thresholds: &th,
        };
        findings(&side, bside.as_ref(), &ctx).unwrap()
    }

    fn kinds(f: &[ProfileFinding]) -> Vec<FindingKind> {
        f.iter().map(|f| f.kind).collect()
    }

    #[test]
    fn top_self_gc_locks_and_throws_from_frames() {
        let p = profile(&[
            (0, 0, "main;work", 50),
            (0, 1, "main;(garbage collector)", 25),
            (0, 2, "main;save;System.Threading.Monitor.Enter", 15),
            (0, 3, "main;check;__cxa_throw", 5),
            (0, 4, "(idle)", 40),
        ]);
        let f = run(&p, None);
        let k = kinds(&f);
        assert!(k.contains(&FindingKind::GcPressure), "{k:?}");
        assert!(k.contains(&FindingKind::LockContention));
        assert!(k.contains(&FindingKind::ExceptionHotPath));
        let top: Vec<&str> = f
            .iter()
            .filter(|f| f.kind == FindingKind::TopSelf)
            .map(|f| f.evidence.frame.as_ref().unwrap().name.as_str())
            .collect();
        assert!(top.contains(&"work") && !top.contains(&"(idle)"), "{top:?}");
        let gc = f.iter().find(|f| f.kind == FindingKind::GcPressure).unwrap();
        assert_eq!(gc.evidence.view, EvidenceView::Butterfly);
        assert_eq!(gc.evidence.path.len(), 2);
        assert!(f.windows(2).all(|w| w[0].severity >= w[1].severity), "most severe first");
        // A clean profile has nothing but its top functions.
        let clean = profile(&[(0, 0, "main;a", 1), (0, 1, "main;b", 1)]);
        assert!(kinds(&run(&clean, None)).iter().all(|k| *k == FindingKind::TopSelf));
    }

    fn evented(kinds: &[(EventKind, u64)], exceptions: u64, pool_names: bool) -> Profile {
        let mut metrics = vec![MetricInfo::new(MetricId::Samples, Measurement::Sampled)];
        if exceptions > 0 {
            metrics.push(MetricInfo::new(MetricId::Exceptions, Measurement::Exact));
        }
        let mut b = ProfileBuilder::new("t", ProfileKind::Cpu, &metrics, Limits::default()).unwrap();
        b.set_has_time(true).set_duration_ns(1_000_000);
        let main = b.frame(FrameSpec::new("main")).unwrap();
        let wait = b.frame(FrameSpec::new("System.Threading.Tasks.Task.Wait")).unwrap();
        let work = b.frame(FrameSpec::new("work")).unwrap();
        let throw = b.frame(FrameSpec::new("Shop.Validate")).unwrap();
        let blocked = b.stack(&[main, wait]).unwrap();
        let busy = b.stack(&[main, work]).unwrap();
        let thrower = b.stack(&[main, throw]).unwrap();
        for t in 0..6u64 {
            let name = if pool_names { ".NET ThreadPool Worker" } else { "" };
            let th = b.thread(Some(t), name).unwrap();
            for i in 0..10u64 {
                let time = t * 150_000 + i * 1000;
                let stack = if i < 8 { blocked } else { busy };
                let mut v = vec![1];
                if exceptions > 0 {
                    v.push(0);
                }
                b.sample(th, time, stack, &v).unwrap();
            }
        }
        if exceptions > 0 {
            b.sample_with(0, 10, thrower, &[(MetricId::Exceptions, exceptions)]).unwrap();
        }
        for (i, (kind, dur)) in kinds.iter().enumerate() {
            b.event(ProfileEvent {
                kind: *kind,
                thread: Some(0),
                start_ns: i as u64 * 1000,
                dur_ns: *dur,
                label: "Shop.InvalidOrderException".into(),
                stack: None,
            });
        }
        b.finish().unwrap()
    }

    #[test]
    fn events_drive_gc_locks_and_exceptions() {
        let p = evented(
            &[
                (EventKind::Gc, 150_000),
                (EventKind::LockWait, 400_000),
                (EventKind::Exception, 0),
                (EventKind::Exception, 0),
            ],
            0,
            false,
        );
        let f = run(&p, None);
        let gc = f.iter().find(|f| f.kind == FindingKind::GcPressure).unwrap();
        assert_eq!(gc.evidence.view, EvidenceView::Timeline);
        assert_eq!(gc.value_pct, Some(15.0));
        assert!(f
            .iter()
            .any(|f| f.kind == FindingKind::LockContention && f.evidence.event_kind == Some(EventKind::LockWait)));
        let ex = f.iter().find(|f| f.kind == FindingKind::ExceptionHotPath).unwrap();
        assert!(ex.detail.contains("Shop.InvalidOrderException (2×)"), "{}", ex.detail);
        assert!(!kinds(&f).contains(&FindingKind::ThreadPoolStarvation), "unnamed threads: no claim");
    }

    #[test]
    fn exact_exceptions_and_starvation() {
        let p = evented(&[], 120, true);
        let f = run(&p, None);
        let ex = f.iter().find(|f| f.kind == FindingKind::ExceptionHotPath).unwrap();
        assert_eq!(ex.evidence.view, EvidenceView::CallTree);
        assert_eq!(ex.evidence.frame.as_ref().unwrap().name, "Shop.Validate");
        assert_eq!(ex.severity, FindingSeverity::Warning);
        let st = f.iter().find(|f| f.kind == FindingKind::ThreadPoolStarvation).unwrap();
        assert!(st.detail.contains("sync over async"), "{}", st.detail);
        assert!(st.detail.contains("pool threads were added"), "{}", st.detail);
        assert_eq!(st.severity, FindingSeverity::Critical);
        assert_eq!(st.evidence.threads.as_ref().unwrap().len(), 6);
    }

    #[test]
    fn regressions_against_a_baseline() {
        let base = profile(&[(0, 0, "main;work", 9), (0, 1, "main;json", 1)]);
        let target = profile(&[(0, 0, "main;work", 6), (0, 1, "main;json", 4)]);
        let f = run(&target, Some(&base));
        let r = f.iter().find(|f| f.kind == FindingKind::Regression).unwrap();
        assert!(r.title.starts_with("json regressed by 30.0 points"), "{}", r.title);
        assert_eq!(r.severity, FindingSeverity::Critical);
        assert_eq!(r.evidence.baseline_id, Some(0));
        assert_eq!(whole(&target).to_ns, target.duration_ns);
    }
}
