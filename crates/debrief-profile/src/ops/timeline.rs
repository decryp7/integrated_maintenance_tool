//! Thread timeline: per-thread activity buckets over a time range, plus the
//! runtime's events (GC, JIT, exceptions, lock waits) in that range.

use crate::api::{EventDto, ThreadLane, ThreadTimeline, ThreadTimelineRequest};
use crate::error::{ProfileError, Result};
use crate::limits::query::{MAX_BUCKETS, MAX_EVENTS};
use crate::model::Profile;
use crate::ops::tree::CompiledView;

pub fn thread_timeline(profile: &Profile, view: &CompiledView, req: &ThreadTimelineRequest) -> Result<ThreadTimeline> {
    if !profile.has_time {
        return Err(ProfileError::NotAvailable(format!(
            "this {} profile has no timestamps, so there is no timeline",
            profile.source
        )));
    }
    if req.buckets == 0 || req.buckets > MAX_BUCKETS {
        return Err(ProfileError::invalid(format!("buckets must be between 1 and {MAX_BUCKETS}")));
    }
    let metric = req.metric.unwrap_or(crate::model::MetricId::Samples);
    let metric = if profile.metric_index(metric).is_some() {
        metric
    } else if req.metric.is_none() {
        profile.default_metric()
    } else {
        return Err(ProfileError::NotAvailable(format!(
            "this profile has no {} data",
            metric.label().to_lowercase()
        )));
    };
    let k = profile.metric_index(metric).unwrap_or(0);
    let m = profile.metrics.len();
    let (from, to) = view.range.unwrap_or((0, profile.duration_ns));
    let span = (to - from).saturating_add(1);
    let n = req.buckets;
    let bucket_of = |t: u64| (((t.saturating_sub(from)) as u128 * n as u128 / span as u128) as usize).min(n - 1);

    let mut lanes: Vec<Option<ThreadLane>> = vec![None; profile.threads.len()];
    for i in view.sample_indices(profile) {
        let t = profile.samples.time[i];
        if t < from || t > to {
            continue;
        }
        let th = profile.samples.thread[i] as usize;
        let lane = lanes[th].get_or_insert_with(|| ThreadLane {
            thread: profile.threads[th].clone(),
            total: 0,
            buckets: vec![0; n],
        });
        let v = profile.samples.values[i * m + k];
        metric.fold(&mut lane.buckets[bucket_of(t)], v);
        metric.fold(&mut lane.total, v);
    }
    let mut threads: Vec<ThreadLane> = lanes.into_iter().flatten().filter(|l| l.total > 0).collect();
    threads.sort_by(|a, b| b.total.cmp(&a.total).then(a.thread.id.cmp(&b.thread.id)));

    let cap = req.max_events.unwrap_or(MAX_EVENTS).min(MAX_EVENTS);
    let mut events = Vec::new();
    let mut truncated = false;
    for e in &profile.events {
        let end = e.start_ns.saturating_add(e.dur_ns);
        if end < from || e.start_ns > to {
            continue;
        }
        if req.event_kinds.as_ref().is_some_and(|k| !k.contains(&e.kind)) {
            continue;
        }
        if let (Some(mask), Some(t)) = (&view.threads, e.thread) {
            if !mask.get(t as usize).copied().unwrap_or(false) {
                continue;
            }
        }
        if events.len() >= cap {
            truncated = true;
            break;
        }
        events.push(EventDto {
            kind: e.kind,
            thread: e.thread,
            start_ns: e.start_ns,
            dur_ns: e.dur_ns,
            label: e.label.clone(),
        });
    }
    Ok(ThreadTimeline {
        from_ns: from,
        to_ns: to,
        bucket_ns: span as f64 / n as f64,
        threads,
        events,
        events_truncated: truncated,
        metric,
        started_ms: profile.started_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{ProfileView, TimeRange};
    use crate::builder::{FrameSpec, ProfileBuilder};
    use crate::limits::Limits;
    use crate::model::{EventKind, Measurement, MetricId, MetricInfo, ProfileEvent, ProfileKind};
    use crate::ops::tree::tests::profile;

    fn req(buckets: usize) -> ThreadTimelineRequest {
        ThreadTimelineRequest {
            profile_id: 0,
            view: ProfileView::default(),
            buckets,
            metric: None,
            event_kinds: None,
            max_events: None,
        }
    }

    fn with_events() -> Profile {
        let mut b = ProfileBuilder::new(
            "t",
            ProfileKind::Cpu,
            &[MetricInfo::new(MetricId::Samples, Measurement::Sampled)],
            Limits::default(),
        )
        .unwrap();
        b.set_has_time(true);
        let t0 = b.thread(Some(1), "main").unwrap();
        let t1 = b.thread(Some(2), "worker").unwrap();
        let f = b.frame(FrameSpec::new("f")).unwrap();
        let s = b.stack(&[f]).unwrap();
        for i in 0..100 {
            b.sample(if i % 4 == 0 { t1 } else { t0 }, i * 10, s, &[1]).unwrap();
        }
        for (i, kind) in [EventKind::Gc, EventKind::Exception, EventKind::LockWait].into_iter().enumerate() {
            b.event(ProfileEvent {
                kind,
                thread: Some(if i == 2 { t1 } else { t0 }),
                start_ns: 100 + i as u64 * 300,
                dur_ns: 50,
                label: format!("{kind:?}"),
                stack: None,
            });
        }
        b.finish().unwrap()
    }

    #[test]
    fn buckets_threads_and_events() {
        let p = with_events();
        let tl = thread_timeline(&p, &CompiledView::all(), &req(10)).unwrap();
        assert_eq!(tl.threads.len(), 2);
        assert_eq!(tl.threads[0].thread.name, "main");
        assert_eq!(tl.threads[0].total, 75);
        let all: u64 = tl.threads.iter().flat_map(|t| t.buckets.iter()).sum();
        assert_eq!(all, 100, "every sample lands in a bucket");
        assert_eq!(tl.events.len(), 3);
        assert_eq!(tl.metric, MetricId::Samples);

        let mut r = req(4);
        r.event_kinds = Some(vec![EventKind::Gc]);
        r.max_events = Some(1);
        let tl = thread_timeline(&p, &CompiledView::all(), &r).unwrap();
        assert_eq!(tl.events.len(), 1);
        assert!(!tl.events_truncated);
        let mut r = req(4);
        r.max_events = Some(1);
        assert!(thread_timeline(&p, &CompiledView::all(), &r).unwrap().events_truncated);

        let view = CompiledView::compile(
            &p,
            &ProfileView {
                range: Some(TimeRange { from_ns: 0, to_ns: 499 }),
                threads: Some(vec![0]),
                ..ProfileView::default()
            },
        )
        .unwrap();
        let tl = thread_timeline(&p, &view, &req(5)).unwrap();
        assert_eq!(tl.threads.len(), 1);
        assert_eq!(tl.threads[0].total, 37);
        assert_eq!(tl.events.len(), 2, "the lock wait is later and on another thread");
        assert_eq!(tl.bucket_ns, 100.0);
    }

    #[test]
    fn errors() {
        let p = with_events();
        assert!(thread_timeline(&p, &CompiledView::all(), &req(0)).is_err());
        assert!(thread_timeline(&p, &CompiledView::all(), &req(MAX_BUCKETS + 1)).is_err());
        let mut r = req(10);
        r.metric = Some(MetricId::AllocBytes);
        assert!(thread_timeline(&p, &CompiledView::all(), &r).is_err());
        let mut r = req(10);
        r.metric = None;
        let q = profile(&[(0, 0, "a", 1)]);
        assert_eq!(thread_timeline(&q, &CompiledView::all(), &r).unwrap().metric, MetricId::Samples);
        let mut untimed = q.clone();
        untimed.has_time = false;
        assert!(thread_timeline(&untimed, &CompiledView::all(), &r).is_err());
        // Default metric when samples are absent.
        let mut time_only = q.clone();
        time_only.metrics.remove(0);
        time_only.samples.values = vec![10];
        assert_eq!(
            thread_timeline(&time_only, &CompiledView::all(), &r).unwrap().metric,
            MetricId::TimeNs
        );
    }
}
