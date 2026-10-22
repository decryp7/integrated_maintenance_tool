//! Performance: a synthetic profile with 1M samples over 50k distinct
//! functions (deep, skewed stacks), timing the operations the UI calls most.
//!
//! Optimised builds get the full size and strict targets:
//! `cargo test --release -p debrief-profile --test perf -- --nocapture`.
//! Debug builds (the normal `cargo test`) run a fifth of the samples with
//! looser targets, so the test stays fast and still catches blow-ups.

use debrief_profile::api::*;
use debrief_profile::*;
use std::time::{Duration, Instant};

struct Lcg(u64);

impl Lcg {
    fn next(&mut self, m: u64) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 33) % m
    }
}

fn synthetic(samples: usize, frames: u32) -> Profile {
    let mut b = ProfileBuilder::new(
        "perf",
        ProfileKind::Cpu,
        &[
            MetricInfo::new(MetricId::Samples, Measurement::Sampled),
            MetricInfo::new(MetricId::TimeNs, Measurement::Sampled),
        ],
        Limits::default(),
    )
    .unwrap();
    b.set_has_time(true);
    let mut rng = Lcg(42);
    let ids: Vec<u32> = (0..frames)
        .map(|i| {
            let name = format!("App.Module{}.Type{}.Method{i}", i % 97, i % 1013);
            let module = format!("App.Module{}.dll", i % 97);
            b.frame(
                FrameSpec::new(&name)
                    .module(Some(&module))
                    .kind(FrameKind::Managed)
                    .framework(i % 5 == 0),
            )
            .unwrap()
        })
        .collect();
    let threads: Vec<u32> = (0..16).map(|t| b.thread(Some(t), &format!("Worker {t}")).unwrap()).collect();
    // 60k distinct stacks: a shared trunk, then a random walk biased to a hot set of frames.
    let mut stacks = Vec::new();
    for _ in 0..60_000 {
        let depth = 8 + rng.next(30) as usize;
        let mut frames = vec![ids[0], ids[1 + rng.next(4) as usize]];
        for d in 2..depth {
            let f = if rng.next(4) == 0 {
                ids[rng.next(ids.len() as u64) as usize]
            } else {
                ids[(d as u64 * 131 + rng.next(400)) as usize % ids.len()]
            };
            frames.push(f);
        }
        stacks.push(b.stack(&frames).unwrap());
    }
    for i in 0..samples {
        // Skewed: low stack ids are hot.
        let r = rng.next(1 << 20) as f64 / (1u64 << 20) as f64;
        let s = stacks[((r * r * r) * stacks.len() as f64) as usize % stacks.len()];
        let t = threads[rng.next(16) as usize];
        b.sample(t, i as u64 * 1_000_000, s, &[1, 1_000_000]).unwrap();
    }
    b.finish().unwrap()
}

fn time<T>(label: &str, budget: Duration, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let out = f();
    let took = start.elapsed();
    println!(
        "{label:<36} {:>9.1} ms (budget {} ms)",
        took.as_secs_f64() * 1e3,
        budget.as_millis()
    );
    assert!(took <= budget, "{label} took {took:?}, budget {budget:?}");
    out
}

#[test]
fn one_million_samples_fifty_thousand_frames() {
    let release = !cfg!(debug_assertions);
    let (samples, scale) = if release { (1_000_000, 1) } else { (200_000, 20) };
    let ms = |n: u64| Duration::from_millis(n * scale);

    let p = time("build profile (import cost)", ms(1000), || synthetic(samples, 50_000));
    println!(
        "samples {}  frames {}  stack nodes {}  threads {}",
        p.samples.len(),
        p.frames.len(),
        p.stacks.len(),
        p.threads.len()
    );
    let mut store = Captures::new(Vec::new(), Limits::default());
    let id = store.insert(Capture::Profile(p.clone()), None).unwrap().id();
    let base = store.insert(Capture::Profile(p), None).unwrap().id();

    let hot = HotspotsRequest {
        profile_id: id,
        view: ProfileView::default(),
        group_by: GroupBy::Function,
        metric: None,
        sort: None,
        search: None,
        offset: 0,
        limit: 200,
    };
    let page = time("hotspots (cold: builds the tree)", ms(1000), || store.hotspots(&hot).unwrap());
    assert_eq!(page.rows.len(), 200);
    time("hotspots (warm, cached tree)", ms(400), || store.hotspots(&hot).unwrap());
    time("hotspots by type", ms(400), || {
        store
            .hotspots(&HotspotsRequest {
                group_by: GroupBy::Type,
                ..hot.clone()
            })
            .unwrap()
    });
    let flame_req = FlameRequest {
        profile_id: id,
        view: ProfileView::default(),
        direction: TreeDirection::TopDown,
        focus: vec![],
        viewport: Viewport::default(),
        width_px: 1600.0,
        min_width_px: None,
        max_depth: None,
        metric: None,
        order: FlameOrder::Name,
        search: Some("Method7".into()),
        search_regex: false,
        max_rects: None,
    };
    let g = time("flame graph layout (1600 px, search)", ms(300), || store.flame(&flame_req).unwrap());
    println!(
        "  rects {}  max depth {}  matched {:.1}%",
        g.rects.len(),
        g.max_depth,
        g.matched_pct
    );
    time("inverted flame graph layout", ms(500), || {
        store
            .flame(&FlameRequest {
                direction: TreeDirection::BottomUp,
                ..flame_req.clone()
            })
            .unwrap()
    });
    time("call tree top level + hot path", ms(100), || {
        store
            .hot_path(&HotPathRequest {
                profile_id: id,
                view: ProfileView::default(),
                direction: TreeDirection::TopDown,
                path: vec![],
                metric: None,
                min_fraction: None,
            })
            .unwrap()
    });
    let ranged = ProfileView {
        range: Some(TimeRange {
            from_ns: 0,
            to_ns: samples as u64 * 500_000,
        }),
        just_my_code: Some(JustMyCode::default()),
        ..ProfileView::default()
    };
    time("time range + just my code (rebuild)", ms(1000), || {
        store
            .hotspots(&HotspotsRequest {
                view: ranged.clone(),
                ..hot.clone()
            })
            .unwrap()
    });
    time("thread timeline (400 buckets)", ms(100), || {
        store
            .thread_timeline(&ThreadTimelineRequest {
                profile_id: id,
                view: ProfileView::default(),
                buckets: 400,
                metric: None,
                event_kinds: None,
                max_events: None,
            })
            .unwrap()
    });
    time("function diff vs baseline", ms(1500), || {
        store
            .function_diff(&FunctionDiffRequest {
                baseline_id: base,
                target_id: id,
                view: ProfileView::default(),
                group_by: GroupBy::Function,
                metric: None,
                sort: DiffSort::AbsDeltaSelf,
                search: None,
                offset: 0,
                limit: 200,
            })
            .unwrap()
    });
    time("findings", ms(2000), || {
        store
            .findings(&FindingsRequest {
                profile_id: id,
                baseline_id: Some(base),
                view: ProfileView::default(),
                thresholds: FindingThresholds::default(),
            })
            .unwrap()
    });
}
