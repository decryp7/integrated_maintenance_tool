//! Hotspots: functions (or types, namespaces, modules, threads) with self and
//! total values. A group's total counts each sample once, even when the
//! group appears several times on one stack (recursion).

use crate::api::{GroupBy, HotspotRow, HotspotsPage, HotspotsRequest, SortSpec, ValueOf};
use crate::error::{ProfileError, Result};
use crate::frames::{namespace_key, type_key};
use crate::model::{MetricId, Profile};
use crate::ops::common::{check_page, frame_dto, page, pct, pick_metric, values, Search};
use crate::ops::tree::{CallTree, CompiledView};
use rustc_hash::FxHashMap;
use std::cmp::Ordering;

/// Aggregated values per group.
#[derive(Debug, Clone)]
pub struct Groups {
    pub metrics: Vec<MetricId>,
    pub keys: Vec<String>,
    pub names: Vec<String>,
    pub modules: Vec<Option<String>>,
    /// A frame of the group (function grouping) or the thread id (thread grouping).
    pub frame: Vec<Option<u32>>,
    pub thread: Vec<Option<u32>>,
    pub self_v: Vec<u64>,
    pub total_v: Vec<u64>,
    pub touched: Vec<bool>,
}

impl Groups {
    fn new(metrics: Vec<MetricId>) -> Groups {
        Groups {
            metrics,
            keys: Vec::new(),
            names: Vec::new(),
            modules: Vec::new(),
            frame: Vec::new(),
            thread: Vec::new(),
            self_v: Vec::new(),
            total_v: Vec::new(),
            touched: Vec::new(),
        }
    }

    fn add(&mut self, key: String, name: String, module: Option<String>, frame: Option<u32>, thread: Option<u32>) -> u32 {
        let m = self.metrics.len();
        self.keys.push(key);
        self.names.push(name);
        self.modules.push(module);
        self.frame.push(frame);
        self.thread.push(thread);
        self.self_v.extend(std::iter::repeat_n(0, m));
        self.total_v.extend(std::iter::repeat_n(0, m));
        self.touched.push(false);
        (self.keys.len() - 1) as u32
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn self_of(&self, g: usize, k: usize) -> u64 {
        self.self_v[g * self.metrics.len() + k]
    }

    pub fn total_of(&self, g: usize, k: usize) -> u64 {
        self.total_v[g * self.metrics.len() + k]
    }

    pub fn self_row(&self, g: usize) -> &[u64] {
        let m = self.metrics.len();
        &self.self_v[g * m..(g + 1) * m]
    }

    pub fn total_row(&self, g: usize) -> &[u64] {
        let m = self.metrics.len();
        &self.total_v[g * m..(g + 1) * m]
    }

    /// Index of a group by key.
    pub fn index(&self) -> FxHashMap<&str, usize> {
        self.keys.iter().enumerate().map(|(i, k)| (k.as_str(), i)).collect()
    }
}

/// Group id of every frame for `by` (not used for threads).
fn frame_groups(profile: &Profile, by: GroupBy, groups: &mut Groups) -> Vec<u32> {
    let mut ids: FxHashMap<String, u32> = FxHashMap::default();
    profile
        .frames
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let (key, name, module) = match by {
                GroupBy::Function | GroupBy::Thread => (crate::ops::common::function_key(f), f.function.clone(), f.module.clone()),
                GroupBy::Type => {
                    let k = type_key(&f.function).unwrap_or_else(|| "(no type)".into());
                    (k.clone(), k, None)
                }
                GroupBy::Namespace => {
                    let k = namespace_key(&f.function).unwrap_or_else(|| "(no namespace)".into());
                    (k.clone(), k, None)
                }
                GroupBy::Module => {
                    let k = f
                        .module
                        .as_deref()
                        .map(|m| crate::frames::basename(m).to_string())
                        .unwrap_or_else(|| "(unknown module)".into());
                    (k.clone(), k, None)
                }
            };
            *ids.entry(key.clone()).or_insert_with(|| {
                let frame = (by == GroupBy::Function).then_some(i as u32);
                groups.add(key, name, module, frame, None)
            })
        })
        .collect()
}

/// Self and total values per group under the tree's view.
pub fn group(profile: &Profile, tree: &CallTree, view: &CompiledView, by: GroupBy) -> Groups {
    let m = tree.m();
    let mut groups = Groups::new(tree.metrics.clone());
    if by == GroupBy::Thread {
        for t in &profile.threads {
            groups.add(format!("thread:{}", t.id), t.name.clone(), None, None, Some(t.id));
        }
        for i in view.sample_indices(profile) {
            let g = profile.samples.thread[i] as usize;
            let vals = &profile.samples.values[i * m..(i + 1) * m];
            for (k, &v) in vals.iter().enumerate() {
                let id = groups.metrics[k];
                id.fold(&mut groups.self_v[g * m + k], v);
                id.fold(&mut groups.total_v[g * m + k], v);
            }
            groups.touched[g] = true;
        }
        return groups;
    }
    let of_frame = frame_groups(profile, by, &mut groups);
    let mut on_stack = vec![0u32; groups.len()];
    let metrics = tree.metrics.clone();
    tree.walk(|n, entering| {
        let g = of_frame[tree.frame[n as usize] as usize] as usize;
        if !entering {
            on_stack[g] -= 1;
            return;
        }
        groups.touched[g] = true;
        for (k, id) in metrics.iter().enumerate() {
            id.fold(&mut groups.self_v[g * m + k], tree.self_of(n, k));
            if !id.inclusive() {
                id.fold(&mut groups.total_v[g * m + k], tree.self_of(n, k));
            } else if on_stack[g] == 0 {
                id.fold(&mut groups.total_v[g * m + k], tree.total_of(n, k));
            }
        }
        on_stack[g] += 1;
    });
    groups
}

/// Sorts group indices.
pub fn sort_groups(groups: &Groups, idx: &mut [usize], sort: &SortSpec) -> Result<()> {
    match sort.metric {
        None => {
            let desc = sort.descending.unwrap_or(false);
            idx.sort_by(|&a, &b| {
                let o = groups.names[a].to_lowercase().cmp(&groups.names[b].to_lowercase());
                if desc {
                    o.reverse()
                } else {
                    o
                }
            });
        }
        Some(metric) => {
            let k = groups
                .metrics
                .iter()
                .position(|m| *m == metric)
                .ok_or_else(|| ProfileError::NotAvailable(format!("this profile has no {} data", metric.label().to_lowercase())))?;
            let desc = sort.descending.unwrap_or(true);
            let directed = |o: Ordering| if desc { o.reverse() } else { o };
            match sort.of {
                ValueOf::AvgPerCall => {
                    let c = groups.metrics.iter().position(|m| *m == MetricId::Calls).ok_or_else(|| {
                        ProfileError::NotAvailable("this profile has no call counts, so there is no average per call".into())
                    })?;
                    let avg = |g: usize| (groups.total_of(g, c) > 0).then(|| groups.total_of(g, k) as f64 / groups.total_of(g, c) as f64);
                    idx.sort_by(|&a, &b| {
                        let o = match (avg(a), avg(b)) {
                            (Some(x), Some(y)) => directed(cmp_f64(x, y)),
                            // Rows without calls have no average: last in either direction.
                            (Some(_), None) => Ordering::Less,
                            (None, Some(_)) => Ordering::Greater,
                            (None, None) => Ordering::Equal,
                        };
                        o.then_with(|| groups.names[a].cmp(&groups.names[b]))
                    });
                }
                ValueOf::SelfValue | ValueOf::Total => {
                    let value = |g: usize| match sort.of {
                        ValueOf::SelfValue => groups.self_of(g, k),
                        _ => groups.total_of(g, k),
                    };
                    idx.sort_by(|&a, &b| directed(value(a).cmp(&value(b))).then_with(|| groups.names[a].cmp(&groups.names[b])));
                }
            }
        }
    }
    Ok(())
}

/// One row for group `g`, percentages of the tree's grand total of metric `k`.
pub fn row(profile: &Profile, groups: &Groups, g: usize, k: usize, grand: u64) -> HotspotRow {
    let mi = |id: MetricId| groups.metrics.iter().position(|m| *m == id);
    let avg = match (mi(MetricId::TimeNs), mi(MetricId::Calls)) {
        (Some(t), Some(c)) if groups.total_of(g, c) > 0 => Some(groups.total_of(g, t) as f64 / groups.total_of(g, c) as f64),
        _ => None,
    };
    let max = mi(MetricId::MaxCallNs).map(|x| groups.total_of(g, x)).filter(|&v| v > 0);
    HotspotRow {
        key: groups.keys[g].clone(),
        name: groups.names[g].clone(),
        module: groups.modules[g].clone(),
        frame: groups.frame[g].map(|f| frame_dto(profile, f)),
        thread: groups.thread[g],
        self_values: values(&groups.metrics, groups.self_row(g)),
        total_values: values(&groups.metrics, groups.total_row(g)),
        self_pct: pct(groups.self_of(g, k), grand),
        total_pct: pct(groups.total_of(g, k), grand),
        avg_per_call_ns: avg,
        max_per_call_ns: max,
    }
}

/// The hotspots table.
pub fn hotspots(profile: &Profile, tree: &CallTree, view: &CompiledView, req: &HotspotsRequest) -> Result<HotspotsPage> {
    check_page(req.limit)?;
    let (metric, k, measurement) = pick_metric(profile, tree, req.metric)?;
    let search = Search::new(req.search.as_deref(), false)?;
    let groups = group(profile, tree, view, req.group_by);
    let mut idx: Vec<usize> = (0..groups.len())
        .filter(|&g| groups.touched[g])
        .filter(|&g| match &search {
            None => true,
            Some(s) => s.is_match(&groups.names[g]) || groups.modules[g].as_deref().is_some_and(|m| s.is_match(m)),
        })
        .collect();
    let sort = req.sort.unwrap_or(SortSpec {
        metric: Some(metric),
        of: ValueOf::SelfValue,
        descending: Some(true),
    });
    sort_groups(&groups, &mut idx, &sort)?;
    let total_rows = idx.len();
    let rows = page(idx, req.offset, req.limit)
        .into_iter()
        .map(|g| row(profile, &groups, g, k, tree.grand[k]))
        .collect();
    Ok(HotspotsPage {
        rows,
        total_rows,
        grand_total: values(&tree.metrics, &tree.grand),
        metric,
        measurement,
        samples_in_view: tree.samples_in_view,
    })
}

/// Orders f64 values, NaN last.
pub fn cmp_f64(a: f64, b: f64) -> Ordering {
    a.partial_cmp(&b).unwrap_or(Ordering::Equal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ProfileView;
    use crate::builder::{FrameSpec, ProfileBuilder};
    use crate::limits::Limits;
    use crate::model::{Measurement, MetricInfo, ProfileKind};

    /// Instrumented profile: `(stack, time ns, calls)` per sample.
    fn instrumented(lines: &[(&str, u64, u64)]) -> Profile {
        let mut b = ProfileBuilder::new(
            "t",
            ProfileKind::Wall,
            &[
                MetricInfo::new(MetricId::TimeNs, Measurement::Exact),
                MetricInfo::new(MetricId::Calls, Measurement::Exact),
            ],
            Limits::default(),
        )
        .unwrap();
        let t = b.thread(None, "main").unwrap();
        for (stack, time, calls) in lines {
            let frames: Vec<u32> = stack.split(';').map(|f| b.frame(FrameSpec::new(f)).unwrap()).collect();
            let s = b.stack(&frames).unwrap();
            b.sample(t, 0, s, &[*time, *calls]).unwrap();
        }
        b.finish().unwrap()
    }

    fn request(sort: SortSpec) -> HotspotsRequest {
        HotspotsRequest {
            profile_id: 0,
            view: ProfileView::default(),
            group_by: GroupBy::Function,
            metric: Some(MetricId::TimeNs),
            sort: Some(sort),
            search: None,
            offset: 0,
            limit: 100,
        }
    }

    fn names(page: &HotspotsPage) -> Vec<&str> {
        page.rows.iter().map(|r| r.name.as_str()).collect()
    }

    #[test]
    fn sorts_by_average_per_call_with_rows_without_calls_last() {
        // fast: 100 ns over 10 calls (10 each); slow: 90 ns in 1 call; nocalls: time but no calls.
        let p = instrumented(&[("fast", 100, 10), ("slow", 90, 1), ("nocalls", 500, 0), ("mid", 40, 2)]);
        let view = CompiledView::all();
        let tree = CallTree::build(&p, &view);
        let avg = SortSpec {
            metric: Some(MetricId::TimeNs),
            of: ValueOf::AvgPerCall,
            descending: None,
        };
        let page = hotspots(&p, &tree, &view, &request(avg)).unwrap();
        assert_eq!(names(&page), ["slow", "mid", "fast", "nocalls"]);
        assert_eq!(page.rows[0].avg_per_call_ns, Some(90.0), "the sort key is the row's average");
        let asc = SortSpec {
            descending: Some(false),
            ..avg
        };
        let page = hotspots(&p, &tree, &view, &request(asc)).unwrap();
        assert_eq!(
            names(&page),
            ["fast", "mid", "slow", "nocalls"],
            "no average stays last ascending too"
        );
        // Ties fall back to the name.
        let p = instrumented(&[("b", 10, 1), ("a", 20, 2), ("x", 1, 0), ("w", 1, 0)]);
        let tree = CallTree::build(&p, &view);
        let page = hotspots(&p, &tree, &view, &request(avg)).unwrap();
        assert_eq!(names(&page), ["a", "b", "w", "x"]);
        // The ordinary keys still work.
        let total = SortSpec {
            metric: Some(MetricId::TimeNs),
            of: ValueOf::Total,
            descending: Some(false),
        };
        assert_eq!(names(&hotspots(&p, &tree, &view, &request(total)).unwrap())[0], "w");
    }

    #[test]
    fn average_per_call_needs_call_counts() {
        let p = crate::ops::tree::tests::profile(&[(0, 0, "main;a", 1)]);
        let view = CompiledView::all();
        let tree = CallTree::build(&p, &view);
        let sort = SortSpec {
            metric: Some(MetricId::TimeNs),
            of: ValueOf::AvgPerCall,
            descending: None,
        };
        let e = hotspots(&p, &tree, &view, &request(sort)).unwrap_err();
        assert!(e.to_string().contains("no call counts"), "{e}");
        assert_eq!(e.code(), crate::error::ErrorCode::Unsupported);
    }
}
