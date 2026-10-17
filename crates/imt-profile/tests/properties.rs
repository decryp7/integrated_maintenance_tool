//! Property-based tests of the profile model and its views: totals add up,
//! comparisons of a profile with itself are zero, time filters are monotonic,
//! transforms keep the total, and no view panics on any profile.
//!
//! Run more cases with `PROPTEST_CASES=5000 cargo test -p imt-profile --test properties`.

use imt_profile::api::*;
use imt_profile::ops::nav::{children, level_at};
use imt_profile::ops::tree::{CallTree, CompiledView};
use imt_profile::*;
use proptest::prelude::*;

/// `(thread, time, stack of frame indices, weight)`.
type Line = (u8, u16, Vec<u8>, u16);

fn lines() -> impl Strategy<Value = Vec<Line>> {
    prop::collection::vec((0u8..3, any::<u16>(), prop::collection::vec(0u8..12, 1..9), 1u16..100), 1..60)
}

fn build(lines: &[Line]) -> Profile {
    let mut b = ProfileBuilder::new(
        "prop",
        ProfileKind::Cpu,
        &[
            MetricInfo::new(MetricId::Samples, Measurement::Sampled),
            MetricInfo::new(MetricId::TimeNs, Measurement::Sampled),
            MetricInfo::new(MetricId::Calls, Measurement::Exact),
            MetricInfo::new(MetricId::MaxCallNs, Measurement::Exact),
        ],
        Limits::default(),
    )
    .unwrap();
    b.set_has_time(true);
    for t in 0..3u64 {
        b.thread(Some(t), &format!("T{t}")).unwrap();
    }
    for (t, time, stack, w) in lines {
        let frames: Vec<u32> = stack
            .iter()
            .map(|f| {
                let name = if f % 4 == 0 { format!("fw.F{f}") } else { format!("app.F{f}") };
                b.frame(FrameSpec::new(&name).framework(f % 4 == 0)).unwrap()
            })
            .collect();
        let s = b.stack(&frames).unwrap();
        let w = u64::from(*w);
        b.sample(u32::from(*t), u64::from(*time), s, &[1, w * 10, w % 3, w * 7]).unwrap();
    }
    b.finish().unwrap()
}

fn views(p: &Profile) -> Vec<ProfileView> {
    vec![
        ProfileView::default(),
        ProfileView {
            recursion: RecursionMode::Direct,
            ..ProfileView::default()
        },
        ProfileView {
            recursion: RecursionMode::All,
            just_my_code: Some(JustMyCode::default()),
            ..ProfileView::default()
        },
        ProfileView {
            threads: Some(vec![1]),
            range: Some(TimeRange {
                from_ns: p.duration_ns / 3,
                to_ns: p.duration_ns,
            }),
            ..ProfileView::default()
        },
    ]
}

/// Sum of a metric over the samples a view selects.
fn sample_sum(p: &Profile, view: &CompiledView, k: usize) -> u64 {
    let m = p.metrics.len();
    let id = p.metrics[k].id;
    let mut acc = 0;
    for i in view.sample_indices(p) {
        id.fold(&mut acc, p.samples.values[i * m + k]);
    }
    acc
}

fn tree(p: &Profile, view: &ProfileView) -> (CompiledView, CallTree) {
    let cv = CompiledView::compile(p, view).unwrap();
    let t = CallTree::build(p, &cv);
    (cv, t)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn tree_totals_equal_the_sum_of_samples(lines in lines()) {
        let p = build(&lines);
        for view in views(&p) {
            let (cv, t) = tree(&p, &view);
            for k in 0..p.metrics.len() {
                prop_assert_eq!(t.grand[k], sample_sum(&p, &cv, k), "metric {:?} view {:?}", p.metrics[k].id, view);
            }
            // Every node: total = self + children's totals (inclusive metrics).
            for n in 0..t.len() as u32 {
                for k in [0usize, 1] {
                    let kids: u64 = t.children(n).iter().map(|&c| t.total_of(c, k)).sum();
                    prop_assert_eq!(t.total_of(n, k), t.self_of(n, k) + kids);
                }
            }
            // Self values of all nodes partition the total.
            let selfs: u64 = (0..t.len() as u32).map(|n| t.self_of(n, 0)).sum();
            prop_assert_eq!(selfs, t.grand[0]);
        }
    }

    #[test]
    fn hotspots_and_inverted_tree_partition_the_total(lines in lines()) {
        let p = build(&lines);
        for view in views(&p) {
            let (cv, t) = tree(&p, &view);
            let page = imt_profile::ops::hotspots::hotspots(&p, &t, &cv, &HotspotsRequest {
                profile_id: 0, view: view.clone(), group_by: GroupBy::Function, metric: Some(MetricId::Samples),
                sort: None, search: None, offset: 0, limit: 10_000,
            }).unwrap();
            let selfs: u64 = page.rows.iter().map(|r| r.self_values[&MetricId::Samples]).sum();
            prop_assert_eq!(selfs, t.grand[0]);
            for r in &page.rows {
                prop_assert!(r.total_values[&MetricId::Samples] <= t.grand[0], "a function's total never exceeds the grand total");
                prop_assert!(r.self_values[&MetricId::Samples] <= r.total_values[&MetricId::Samples]);
            }
            let top = children(&t, &level_at(&t, TreeDirection::BottomUp, &[]).unwrap());
            let sum: u64 = top.iter().map(|c| c.total_row[0]).sum();
            prop_assert_eq!(sum, t.grand[0]);
            for group_by in [GroupBy::Type, GroupBy::Namespace, GroupBy::Module, GroupBy::Thread] {
                let g = imt_profile::ops::hotspots::hotspots(&p, &t, &cv, &HotspotsRequest {
                    profile_id: 0, view: view.clone(), group_by, metric: Some(MetricId::Samples),
                    sort: None, search: None, offset: 0, limit: 10_000,
                }).unwrap();
                let s: u64 = g.rows.iter().map(|r| r.self_values[&MetricId::Samples]).sum();
                prop_assert_eq!(s, t.grand[0], "{:?}", group_by);
            }
        }
    }

    #[test]
    fn diff_of_identical_profiles_is_zero(lines in lines()) {
        let p = build(&lines);
        let mut s = Captures::new(Vec::new(), Limits::default());
        let a = s.insert(Capture::Profile(p.clone()), None).unwrap().id();
        let b = s.insert(Capture::Profile(p), None).unwrap().id();
        let d = s.function_diff(&FunctionDiffRequest {
            baseline_id: a, target_id: b, view: ProfileView::default(), group_by: GroupBy::Function,
            metric: None, sort: DiffSort::AbsDeltaSelf, search: None, offset: 0, limit: 10_000,
        }).unwrap();
        for r in &d.rows {
            prop_assert_eq!(r.delta_self_pct, 0.0);
            prop_assert_eq!(r.delta_total_pct, 0.0);
        }
        let g = s.diff_flame(&DiffFlameRequest {
            baseline_id: a, target_id: b, view: ProfileView::default(), focus: vec![], viewport: Viewport::default(),
            width_px: 1000.0, min_width_px: Some(0.0), max_depth: None, metric: None, order: FlameOrder::Name,
            search: None, search_regex: false, max_rects: None,
        }).unwrap();
        prop_assert!(g.rects.iter().all(|r| r.delta == Some(0.0)));
        let f = s.findings(&FindingsRequest { profile_id: b, baseline_id: Some(a), view: ProfileView::default(), thresholds: FindingThresholds::default() }).unwrap();
        prop_assert!(f.iter().all(|f| f.kind != FindingKind::Regression));
    }

    #[test]
    fn time_filter_is_monotonic_and_additive(lines in lines(), a in any::<u16>(), b in any::<u16>(), c in any::<u16>()) {
        let p = build(&lines);
        let mut cuts = [u64::from(a), u64::from(b), u64::from(c)];
        cuts.sort_unstable();
        let total = |from: u64, to: u64| {
            let v = ProfileView { range: Some(TimeRange { from_ns: from, to_ns: to }), ..ProfileView::default() };
            tree(&p, &v).1.grand[0]
        };
        let inner = total(cuts[1], cuts[2]);
        let outer = total(cuts[0], cuts[2]);
        prop_assert!(inner <= outer, "a narrower range never has more");
        if cuts[1] > cuts[0] {
            prop_assert_eq!(total(cuts[0], cuts[1] - 1) + inner, outer, "adjacent ranges add up");
        }
        prop_assert!(outer <= tree(&p, &ProfileView::default()).1.grand[0]);
    }

    #[test]
    fn flame_rectangles_nest_inside_their_parents(lines in lines(), start in 0.0f64..0.5, len in 0.1f64..0.5) {
        let p = build(&lines);
        let (_, t) = tree(&p, &ProfileView::default());
        for direction in [TreeDirection::TopDown, TreeDirection::BottomUp] {
            let g = imt_profile::ops::flame::flame(&p, &t, &FlameRequest {
                profile_id: 0, view: ProfileView::default(), direction, focus: vec![],
                viewport: Viewport { start, end: start + len }, width_px: 500.0, min_width_px: Some(0.0),
                max_depth: None, metric: Some(MetricId::Samples), order: FlameOrder::Name, search: Some("F1".into()),
                search_regex: false, max_rects: None,
            }).unwrap();
            prop_assert!(g.matched_value <= g.focus_value);
            for r in &g.rects {
                prop_assert!(r.x < 1.0 && r.x + r.w > 0.0, "visible");
                prop_assert!(r.self_value <= r.value);
            }
            // Depth-first order: a rectangle's parent is the nearest earlier one a level up.
            let mut chain: Vec<&FlameRect> = Vec::new();
            for r in &g.rects {
                chain.truncate(r.depth as usize);
                if let Some(parent) = chain.last() {
                    prop_assert!(r.x >= parent.x - 1e-9 && r.x + r.w <= parent.x + parent.w + 1e-9);
                }
                chain.push(r);
            }
        }
    }

    #[test]
    fn views_never_panic(lines in lines()) {
        fuzz_views(Capture::Profile(build(&lines)));
    }
}
