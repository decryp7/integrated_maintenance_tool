//! The profiling port the app shell talks to, split by purpose (interface
//! segregation): [`CaptureCatalog`] opens and lists captures,
//! [`ProfileQueries`] answers the CPU/allocation views and
//! [`HeapQueries`] the memory views. [`Captures`] is the in-memory
//! implementation; queries never mutate (command–query separation), only
//! `open` and `close` do.

use crate::api::*;
use crate::error::{ProfileError, Result};
use crate::heap::ops::{self as heap_ops, Heap};
use crate::heap::{HeapAnalysis, HeapSnapshot};
use crate::import::{import_with, sniff_with, Capture, CaptureImporter};
use crate::limits::query::TREE_CACHE;
use crate::limits::Limits;
use crate::model::{MetricId, Profile};
use crate::ops::diff::Side;
use crate::ops::tree::{CallTree, CompiledView};
use crate::ops::{butterfly, diff, findings, flame, hotspots, nav, timeline};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// Captures open at once (each can be large).
pub const MAX_OPEN: usize = 32;

/// Opening, listing and closing captures.
pub trait CaptureCatalog: Send + Sync {
    /// Imports a capture file (bytes read by the caller from a path the user chose).
    fn open(&mut self, file_name: &str, path: Option<&str>, bytes: &[u8]) -> Result<CaptureSummary>;
    fn list(&self) -> Vec<CaptureSummary>;
    /// Forgets a capture; false when it was not open.
    fn close(&mut self, id: u32) -> bool;
    /// Wall-clock spans of the captures, to line them up with logs.
    fn spans(&self) -> Vec<ProfileSpan>;
    /// What a file is (profile, heap snapshot or neither) from its name and
    /// first bytes, without importing it. No side effects.
    fn sniff(&self, file_name: &str, head: &[u8]) -> SniffKind;
}

/// The CPU / allocation / wall-clock views of profiles.
pub trait ProfileQueries: Send + Sync {
    fn hotspots(&self, req: &HotspotsRequest) -> Result<HotspotsPage>;
    fn tree(&self, req: &TreeRequest) -> Result<TreePage>;
    fn hot_path(&self, req: &HotPathRequest) -> Result<HotPath>;
    /// The hottest top-down paths to a function ("show in tree", "zoom flame graph to it").
    fn frame_paths(&self, req: &FramePathsRequest) -> Result<FramePaths>;
    fn butterfly(&self, req: &ButterflyRequest) -> Result<Butterfly>;
    fn flame(&self, req: &FlameRequest) -> Result<FlameGraph>;
    fn diff_flame(&self, req: &DiffFlameRequest) -> Result<FlameGraph>;
    fn function_diff(&self, req: &FunctionDiffRequest) -> Result<FunctionDiffPage>;
    fn thread_timeline(&self, req: &ThreadTimelineRequest) -> Result<ThreadTimeline>;
    fn findings(&self, req: &FindingsRequest) -> Result<Vec<ProfileFinding>>;
    /// Profile samples in a wall-clock window (the Moment view).
    fn moment(&self, req: &ProfileMomentRequest) -> Result<Vec<ProfileMoment>>;
}

/// The memory views of heap snapshots.
pub trait HeapQueries: Send + Sync {
    fn heap_types(&self, req: &HeapTypesRequest) -> Result<HeapTypesPage>;
    fn heap_diff(&self, req: &HeapDiffRequest) -> Result<HeapDiffPage>;
    fn heap_growth(&self, req: &HeapGrowthRequest) -> Result<HeapGrowthPage>;
    fn heap_dominators(&self, req: &HeapDominatorsRequest) -> Result<HeapObjectsPage>;
    fn heap_instances(&self, req: &HeapInstancesRequest) -> Result<HeapObjectsPage>;
    fn heap_paths(&self, req: &HeapPathsRequest) -> Result<HeapPaths>;
    /// Automatic findings over a snapshot (growth against an optional baseline).
    fn heap_findings(&self, req: &HeapFindingsRequest) -> Result<Vec<HeapFinding>>;
}

/// Everything the shell needs: one value per window.
pub trait ProfileStore: CaptureCatalog + ProfileQueries + HeapQueries {}
impl<T: CaptureCatalog + ProfileQueries + HeapQueries> ProfileStore for T {}

enum Entry {
    Profile {
        profile: Arc<Profile>,
        path: Option<String>,
    },
    Heap {
        snapshot: Arc<HeapSnapshot>,
        analysis: Option<Arc<HeapAnalysis>>,
        path: Option<String>,
    },
}

type Built = Arc<(CompiledView, CallTree)>;

/// The in-memory profile store.
pub struct Captures {
    importers: Vec<Arc<dyn CaptureImporter>>,
    limits: Limits,
    next_id: u32,
    entries: BTreeMap<u32, Entry>,
    /// Recently built call trees, most recent last.
    cache: Mutex<Vec<(u32, ProfileView, Built)>>,
}

impl Captures {
    pub fn new(importers: Vec<Arc<dyn CaptureImporter>>, limits: Limits) -> Captures {
        Captures {
            importers,
            limits,
            next_id: 1,
            entries: BTreeMap::new(),
            cache: Mutex::new(Vec::new()),
        }
    }

    /// Adds an already imported capture.
    pub fn insert(&mut self, capture: Capture, path: Option<&str>) -> Result<CaptureSummary> {
        if self.entries.len() >= MAX_OPEN {
            return Err(ProfileError::LimitExceeded {
                what: "open captures",
                limit: MAX_OPEN as u64,
                hint: "close a capture first",
            });
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let path = path.map(str::to_string);
        let entry = match capture {
            Capture::Profile(p) => Entry::Profile {
                profile: Arc::new(p),
                path,
            },
            Capture::Heap(h) => {
                let analysis = h.graph.as_ref().map(|g| Arc::new(HeapAnalysis::new(g, h.types.len())));
                Entry::Heap {
                    snapshot: Arc::new(h),
                    analysis,
                    path,
                }
            }
        };
        let summary = summarize(id, &entry);
        self.entries.insert(id, entry);
        Ok(summary)
    }

    fn summary(&self, id: u32) -> Option<CaptureSummary> {
        self.entries.get(&id).map(|e| summarize(id, e))
    }

    fn profile(&self, id: u32) -> Result<Arc<Profile>> {
        match self.entries.get(&id) {
            Some(Entry::Profile { profile, .. }) => Ok(profile.clone()),
            Some(Entry::Heap { .. }) => Err(ProfileError::WrongCapture {
                id,
                expected: "profile",
                actual: "heap snapshot",
            }),
            None => Err(ProfileError::UnknownCapture(id)),
        }
    }

    fn heap(&self, id: u32) -> Result<(Arc<HeapSnapshot>, Option<Arc<HeapAnalysis>>)> {
        match self.entries.get(&id) {
            Some(Entry::Heap { snapshot, analysis, .. }) => Ok((snapshot.clone(), analysis.clone())),
            Some(Entry::Profile { .. }) => Err(ProfileError::WrongCapture {
                id,
                expected: "heap snapshot",
                actual: "profile",
            }),
            None => Err(ProfileError::UnknownCapture(id)),
        }
    }

    /// The profile and its call tree under `view` (cached).
    fn built(&self, id: u32, view: &ProfileView) -> Result<(Arc<Profile>, Built)> {
        let profile = self.profile(id)?;
        {
            let mut cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(i) = cache.iter().position(|(cid, v, _)| *cid == id && v == view) {
                let hit = cache.remove(i);
                let built = hit.2.clone();
                cache.push(hit);
                return Ok((profile, built));
            }
        }
        let compiled = CompiledView::compile(&profile, view)?;
        let tree = CallTree::build(&profile, &compiled);
        let built: Built = Arc::new((compiled, tree));
        let mut cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache.push((id, view.clone(), built.clone()));
        if cache.len() > TREE_CACHE {
            cache.remove(0);
        }
        Ok((profile, built))
    }

    fn importer_refs(&self) -> Vec<&dyn CaptureImporter> {
        self.importers.iter().map(|i| i.as_ref()).collect()
    }
}

fn summarize(id: u32, entry: &Entry) -> CaptureSummary {
    match entry {
        Entry::Profile { profile: p, path } => {
            let mut kinds: Vec<_> = p.events.iter().map(|e| e.kind).collect();
            kinds.sort_by_key(|k| *k as u8);
            kinds.dedup();
            CaptureSummary::Profile(ProfileSummary {
                id,
                name: p.name.clone(),
                path: path.clone(),
                format: p.source.clone(),
                kind: p.kind,
                started_ms: p.started_ms,
                duration_ns: p.duration_ns,
                has_time: p.has_time,
                sample_count: p.samples.len() as u64,
                frame_count: p.frames.len() as u64,
                event_count: p.events.len() as u64,
                metrics: p.metrics.clone(),
                default_metric: p.default_metric(),
                threads: p.threads.clone(),
                event_kinds: kinds,
                notes: p.notes.clone(),
            })
        }
        Entry::Heap { snapshot: h, path, .. } => CaptureSummary::Heap(HeapSummary {
            id,
            name: h.name.clone(),
            path: path.clone(),
            format: h.source.clone(),
            taken_ms: h.taken_ms,
            type_count: h.types.len() as u64,
            object_count: h.graph.as_ref().map(|g| g.len() as u64).unwrap_or_else(|| h.total_count()),
            total_bytes: h.total_bytes(),
            has_graph: h.graph.is_some(),
            notes: h.notes.clone(),
        }),
    }
}

impl CaptureCatalog for Captures {
    fn open(&mut self, file_name: &str, path: Option<&str>, bytes: &[u8]) -> Result<CaptureSummary> {
        if self.entries.len() >= MAX_OPEN {
            return Err(ProfileError::LimitExceeded {
                what: "open captures",
                limit: MAX_OPEN as u64,
                hint: "close a capture first",
            });
        }
        let mut capture = import_with(&self.importer_refs(), file_name, bytes, &self.limits)?;
        let base = crate::frames::basename(file_name).to_string();
        match &mut capture {
            Capture::Profile(p) if p.name.is_empty() || p.name == p.source => p.name = base,
            Capture::Heap(h) if h.name.is_empty() || h.name == h.source => h.name = base,
            _ => {}
        }
        self.insert(capture, path)
    }

    fn list(&self) -> Vec<CaptureSummary> {
        self.entries.keys().filter_map(|&id| self.summary(id)).collect()
    }

    fn close(&mut self, id: u32) -> bool {
        let removed = self.entries.remove(&id).is_some();
        if removed {
            self.cache
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .retain(|(cid, _, _)| *cid != id);
        }
        removed
    }

    fn sniff(&self, file_name: &str, head: &[u8]) -> SniffKind {
        sniff_with(&self.importer_refs(), file_name, head)
    }

    fn spans(&self) -> Vec<ProfileSpan> {
        self.entries
            .iter()
            .filter_map(|(&id, e)| match e {
                Entry::Profile { profile: p, .. } => p.started_ms.map(|s| ProfileSpan {
                    id,
                    name: p.name.clone(),
                    capture: CaptureKind::Profile,
                    kind: Some(p.kind),
                    from_ms: s,
                    to_ms: p.ended_ms().unwrap_or(s),
                }),
                Entry::Heap { snapshot: h, .. } => h.taken_ms.map(|t| ProfileSpan {
                    id,
                    name: h.name.clone(),
                    capture: CaptureKind::Heap,
                    kind: None,
                    from_ms: t,
                    to_ms: t,
                }),
            })
            .collect()
    }
}

impl ProfileQueries for Captures {
    fn hotspots(&self, req: &HotspotsRequest) -> Result<HotspotsPage> {
        let (p, b) = self.built(req.profile_id, &req.view)?;
        hotspots::hotspots(&p, &b.1, &b.0, req)
    }

    fn tree(&self, req: &TreeRequest) -> Result<TreePage> {
        let (p, b) = self.built(req.profile_id, &req.view)?;
        nav::tree_page(&p, &b.1, req)
    }

    fn hot_path(&self, req: &HotPathRequest) -> Result<HotPath> {
        let (p, b) = self.built(req.profile_id, &req.view)?;
        nav::hot_path(&p, &b.1, req)
    }

    fn frame_paths(&self, req: &FramePathsRequest) -> Result<FramePaths> {
        let (p, b) = self.built(req.profile_id, &req.view)?;
        nav::frame_paths(&p, &b.1, req)
    }

    fn butterfly(&self, req: &ButterflyRequest) -> Result<Butterfly> {
        let (p, b) = self.built(req.profile_id, &req.view)?;
        butterfly::butterfly(&p, &b.1, &b.0, req)
    }

    fn flame(&self, req: &FlameRequest) -> Result<FlameGraph> {
        let (p, b) = self.built(req.profile_id, &req.view)?;
        flame::flame(&p, &b.1, req)
    }

    fn diff_flame(&self, req: &DiffFlameRequest) -> Result<FlameGraph> {
        let (bp, bb) = self.built(req.baseline_id, &req.view)?;
        let (tp, tb) = self.built(req.target_id, &req.view)?;
        let base = Side {
            profile: &bp,
            tree: &bb.1,
            view: &bb.0,
        };
        let target = Side {
            profile: &tp,
            tree: &tb.1,
            view: &tb.0,
        };
        diff::diff_flame(&base, &target, req)
    }

    fn function_diff(&self, req: &FunctionDiffRequest) -> Result<FunctionDiffPage> {
        let (bp, bb) = self.built(req.baseline_id, &req.view)?;
        let (tp, tb) = self.built(req.target_id, &req.view)?;
        let base = Side {
            profile: &bp,
            tree: &bb.1,
            view: &bb.0,
        };
        let target = Side {
            profile: &tp,
            tree: &tb.1,
            view: &tb.0,
        };
        diff::function_diff(&base, &target, req)
    }

    fn thread_timeline(&self, req: &ThreadTimelineRequest) -> Result<ThreadTimeline> {
        let p = self.profile(req.profile_id)?;
        let view = CompiledView::compile(&p, &req.view)?;
        timeline::thread_timeline(&p, &view, req)
    }

    fn findings(&self, req: &FindingsRequest) -> Result<Vec<ProfileFinding>> {
        let (tp, tb) = self.built(req.profile_id, &req.view)?;
        let target = Side {
            profile: &tp,
            tree: &tb.1,
            view: &tb.0,
        };
        let base_built = match req.baseline_id {
            Some(id) => Some(self.built(id, &ProfileView::default())?),
            None => None,
        };
        let base = base_built.as_ref().map(|(p, b)| Side {
            profile: p,
            tree: &b.1,
            view: &b.0,
        });
        let ctx = findings::Context {
            profile_id: req.profile_id,
            baseline_id: req.baseline_id,
            view: &req.view,
            thresholds: &req.thresholds,
        };
        findings::findings(&target, base.as_ref(), &ctx)
    }

    fn moment(&self, req: &ProfileMomentRequest) -> Result<Vec<ProfileMoment>> {
        if req.from_ms > req.to_ms {
            return Err(ProfileError::invalid("the window ends before it starts"));
        }
        let top = req.top.unwrap_or(5).min(50);
        let mut out = Vec::new();
        for (&id, e) in &self.entries {
            let Entry::Profile { profile: p, .. } = e else { continue };
            let (Some(start), Some(end)) = (p.started_ms, p.ended_ms()) else {
                continue;
            };
            if !p.has_time || req.to_ms < start || req.from_ms > end {
                continue;
            }
            let to_ns = |ms: i64| u64::try_from(ms.saturating_sub(start)).unwrap_or(0).saturating_mul(1_000_000);
            let range = TimeRange {
                from_ns: to_ns(req.from_ms),
                to_ns: to_ns(req.to_ms).min(p.duration_ns),
            };
            let view = ProfileView {
                range: Some(range),
                ..ProfileView::default()
            };
            let (_, b) = self.built(id, &view)?;
            let metric = p.default_metric();
            let k = b.1.metric_index(metric)?;
            let page = hotspots::hotspots(
                p,
                &b.1,
                &b.0,
                &HotspotsRequest {
                    profile_id: id,
                    view: view.clone(),
                    group_by: GroupBy::Function,
                    metric: Some(metric),
                    sort: None,
                    search: None,
                    offset: 0,
                    limit: top.max(1),
                },
            )?;
            out.push(ProfileMoment {
                profile_id: id,
                name: p.name.clone(),
                range,
                samples: b.1.samples_in_view,
                metric,
                measurement: p.metrics[k].measurement,
                value: b.1.grand[k],
                top: if top == 0 { Vec::new() } else { page.rows },
            });
        }
        Ok(out)
    }
}

impl HeapQueries for Captures {
    fn heap_types(&self, req: &HeapTypesRequest) -> Result<HeapTypesPage> {
        let (s, a) = self.heap(req.snapshot_id)?;
        heap_ops::types(
            &Heap {
                snapshot: &s,
                analysis: a.as_deref(),
            },
            req,
        )
    }

    fn heap_diff(&self, req: &HeapDiffRequest) -> Result<HeapDiffPage> {
        let (b, _) = self.heap(req.baseline_id)?;
        let (t, _) = self.heap(req.target_id)?;
        heap_ops::diff(&b, &t, req)
    }

    fn heap_growth(&self, req: &HeapGrowthRequest) -> Result<HeapGrowthPage> {
        let snaps = req
            .snapshot_ids
            .iter()
            .map(|&id| self.heap(id).map(|h| h.0))
            .collect::<Result<Vec<_>>>()?;
        let refs: Vec<&HeapSnapshot> = snaps.iter().map(|s| s.as_ref()).collect();
        heap_ops::growth(&refs, req)
    }

    fn heap_dominators(&self, req: &HeapDominatorsRequest) -> Result<HeapObjectsPage> {
        let (s, a) = self.heap(req.snapshot_id)?;
        heap_ops::dominators(
            &Heap {
                snapshot: &s,
                analysis: a.as_deref(),
            },
            req,
        )
    }

    fn heap_instances(&self, req: &HeapInstancesRequest) -> Result<HeapObjectsPage> {
        let (s, a) = self.heap(req.snapshot_id)?;
        heap_ops::instances(
            &Heap {
                snapshot: &s,
                analysis: a.as_deref(),
            },
            req,
        )
    }

    fn heap_paths(&self, req: &HeapPathsRequest) -> Result<HeapPaths> {
        let (s, a) = self.heap(req.snapshot_id)?;
        heap_ops::paths(
            &Heap {
                snapshot: &s,
                analysis: a.as_deref(),
            },
            req,
        )
    }

    fn heap_findings(&self, req: &HeapFindingsRequest) -> Result<Vec<HeapFinding>> {
        let (s, a) = self.heap(req.snapshot_id)?;
        let baseline = match req.baseline_id.filter(|&b| b != req.snapshot_id) {
            Some(b) => Some(self.heap(b)?.0),
            None => None,
        };
        Ok(crate::heap::findings::findings(
            &Heap {
                snapshot: &s,
                analysis: a.as_deref(),
            },
            baseline.as_deref(),
        ))
    }
}

/// Runs every view with ordinary arguments over a capture and discards the
/// results: used by fuzz targets and property tests to check that whatever
/// an importer produced is safe to query. Errors are fine; panics are bugs.
#[doc(hidden)]
pub fn fuzz_views(capture: Capture) {
    let mut s = Captures::new(Vec::new(), Limits::default());
    let Ok(summary) = s.insert(capture, None) else { return };
    let id = summary.id();
    match summary {
        CaptureSummary::Profile(p) => {
            let views = [
                ProfileView::default(),
                ProfileView {
                    just_my_code: Some(JustMyCode::default()),
                    recursion: RecursionMode::All,
                    range: Some(TimeRange {
                        from_ns: p.duration_ns / 4,
                        to_ns: p.duration_ns / 2,
                    }),
                    threads: (!p.threads.is_empty()).then(|| vec![0]),
                },
            ];
            for view in views {
                for group_by in [
                    GroupBy::Function,
                    GroupBy::Type,
                    GroupBy::Namespace,
                    GroupBy::Module,
                    GroupBy::Thread,
                ] {
                    let _ = s.hotspots(&HotspotsRequest {
                        profile_id: id,
                        view: view.clone(),
                        group_by,
                        metric: None,
                        sort: None,
                        search: Some("a".into()),
                        offset: 0,
                        limit: 50,
                    });
                }
                for direction in [TreeDirection::TopDown, TreeDirection::BottomUp] {
                    if let Ok(h) = s.hot_path(&HotPathRequest {
                        profile_id: id,
                        view: view.clone(),
                        direction,
                        path: Vec::new(),
                        metric: None,
                        min_fraction: Some(0.0),
                    }) {
                        let _ = s.tree(&TreeRequest {
                            profile_id: id,
                            view: view.clone(),
                            direction,
                            path: h.path.iter().take(3).copied().collect(),
                            metric: None,
                            offset: 0,
                            limit: 50,
                        });
                        if let Some(&f) = h.path.last() {
                            let _ = s.butterfly(&ButterflyRequest {
                                profile_id: id,
                                view: view.clone(),
                                frame: f,
                                metric: None,
                                limit: 20,
                            });
                            let _ = s.frame_paths(&FramePathsRequest {
                                profile_id: id,
                                view: view.clone(),
                                frame: f,
                                metric: None,
                                limit: Some(3),
                            });
                        }
                    }
                    let _ = s.flame(&FlameRequest {
                        profile_id: id,
                        view: view.clone(),
                        direction,
                        focus: Vec::new(),
                        viewport: Viewport { start: 0.1, end: 0.9 },
                        width_px: 1200.0,
                        min_width_px: None,
                        max_depth: None,
                        metric: None,
                        order: FlameOrder::Value,
                        search: Some("e".into()),
                        search_regex: false,
                        max_rects: Some(5000),
                    });
                }
                let _ = s.thread_timeline(&ThreadTimelineRequest {
                    profile_id: id,
                    view: view.clone(),
                    buckets: 100,
                    metric: None,
                    event_kinds: None,
                    max_events: Some(100),
                });
                let _ = s.findings(&FindingsRequest {
                    profile_id: id,
                    baseline_id: Some(id),
                    view: view.clone(),
                    thresholds: FindingThresholds::default(),
                });
                let _ = s.function_diff(&FunctionDiffRequest {
                    baseline_id: id,
                    target_id: id,
                    view: view.clone(),
                    group_by: GroupBy::Function,
                    metric: Some(MetricId::Samples),
                    sort: DiffSort::AbsDeltaSelf,
                    search: None,
                    offset: 0,
                    limit: 20,
                });
                let _ = s.diff_flame(&DiffFlameRequest {
                    baseline_id: id,
                    target_id: id,
                    view,
                    focus: Vec::new(),
                    viewport: Viewport::default(),
                    width_px: 800.0,
                    min_width_px: None,
                    max_depth: Some(64),
                    metric: None,
                    order: FlameOrder::Name,
                    search: None,
                    search_regex: false,
                    max_rects: Some(2000),
                });
            }
            if let Some(start) = p.started_ms {
                let _ = s.moment(&ProfileMomentRequest {
                    from_ms: start,
                    to_ms: start.saturating_add(10),
                    top: None,
                });
            }
        }
        CaptureSummary::Heap(h) => {
            for sort in [HeapSort::Name, HeapSort::Count, HeapSort::Bytes, HeapSort::Retained] {
                let _ = s.heap_types(&HeapTypesRequest {
                    snapshot_id: id,
                    sort,
                    search: None,
                    offset: 0,
                    limit: 50,
                });
            }
            let _ = s.heap_diff(&HeapDiffRequest {
                baseline_id: id,
                target_id: id,
                search: None,
                sort: HeapDiffSort::BytesDelta,
                offset: 0,
                limit: 50,
            });
            let _ = s.heap_growth(&HeapGrowthRequest {
                snapshot_ids: vec![id, id, id],
                search: None,
                steady_only: false,
                offset: 0,
                limit: 50,
            });
            let _ = s.heap_findings(&HeapFindingsRequest {
                snapshot_id: id,
                baseline_id: Some(id),
            });
            if let Ok(top) = s.heap_dominators(&HeapDominatorsRequest {
                snapshot_id: id,
                parent: None,
                offset: 0,
                limit: 20,
            }) {
                for r in top.rows.iter().take(3) {
                    let _ = s.heap_paths(&HeapPathsRequest {
                        snapshot_id: id,
                        object: r.object,
                        max_paths: Some(3),
                        max_depth: Some(64),
                    });
                    let _ = s.heap_instances(&HeapInstancesRequest {
                        snapshot_id: id,
                        type_name: r.type_name.clone(),
                        offset: 0,
                        limit: 10,
                    });
                }
            }
            let _ = h;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heap::dominators::tests::graph;
    use crate::import::tests::Lines;
    use crate::model::MetricId;

    fn store() -> Captures {
        Captures::new(vec![Arc::new(Lines(90))], Limits::default())
    }

    #[test]
    fn sniffs_without_opening_and_finds_frame_paths() {
        let mut s = Captures::new(vec![Arc::new(Lines(90)), Arc::new(crate::import::tests::Heaps)], Limits::default());
        assert_eq!(s.sniff("a", b"#t\nmain 1\n"), SniffKind::Capture);
        assert_eq!(s.sniff("a", b"#h"), SniffKind::Heap);
        assert_eq!(s.sniff("a.log", b"INFO hi"), SniffKind::None);
        assert!(s.list().is_empty(), "sniffing opens nothing");
        let id = s.open("p", None, b"#t\nmain;a 3\nmain;b;a 1\n").unwrap().id();
        let tree = s
            .tree(&TreeRequest {
                profile_id: id,
                view: ProfileView::default(),
                direction: TreeDirection::TopDown,
                path: Vec::new(),
                metric: None,
                offset: 0,
                limit: 10,
            })
            .unwrap();
        let main = tree.nodes[0].frame.id;
        let kids = s
            .tree(&TreeRequest {
                profile_id: id,
                view: ProfileView::default(),
                direction: TreeDirection::TopDown,
                path: vec![main],
                metric: None,
                offset: 0,
                limit: 10,
            })
            .unwrap();
        let a = kids.nodes.iter().find(|n| n.frame.name == "a").unwrap().frame.id;
        let paths = s
            .frame_paths(&FramePathsRequest {
                profile_id: id,
                view: ProfileView::default(),
                frame: a,
                metric: None,
                limit: None,
            })
            .unwrap();
        assert_eq!(paths.total_paths, 2);
        assert_eq!(paths.paths[0].path, vec![main, a]);
        assert!(matches!(
            s.frame_paths(&FramePathsRequest {
                profile_id: 99,
                view: ProfileView::default(),
                frame: a,
                metric: None,
                limit: None,
            }),
            Err(ProfileError::UnknownCapture(99))
        ));
    }

    #[test]
    fn open_list_query_close() {
        let mut s = store();
        let sum = s.open("/tmp/x.lines", Some("/tmp/x.lines"), b"#t\nmain;a 3\nmain;b 1\n").unwrap();
        let id = sum.id();
        let CaptureSummary::Profile(p) = &sum else { panic!() };
        assert_eq!(p.name, "x.lines");
        assert_eq!(p.sample_count, 2);
        assert_eq!(p.default_metric, MetricId::Samples);
        assert_eq!(s.list().len(), 1);
        assert!(s.spans().is_empty(), "no wall-clock start");
        let req = HotspotsRequest {
            profile_id: id,
            view: ProfileView::default(),
            group_by: GroupBy::Function,
            metric: None,
            sort: None,
            search: None,
            offset: 0,
            limit: 10,
        };
        let h = s.hotspots(&req).unwrap();
        assert_eq!(h.rows[0].name, "a");
        assert_eq!(s.hotspots(&req).unwrap(), h, "cached tree, same answer");
        for i in 0..TREE_CACHE + 1 {
            let mut r = req.clone();
            r.view.threads = Some(vec![0; i + 1]);
            s.hotspots(&r).unwrap();
        }
        assert!(s.cache.lock().unwrap().len() <= TREE_CACHE);
        assert!(s.close(id));
        assert!(!s.close(id));
        assert!(s.cache.lock().unwrap().is_empty());
        assert!(matches!(s.hotspots(&req), Err(ProfileError::UnknownCapture(_))));
        assert!(s.open("y.bin", None, b"nope").is_err());
    }

    #[test]
    fn wrong_kinds_and_limits() {
        let mut s = store();
        let heap = s.insert(Capture::Heap(graph(&[("A", 1, &[])], &[0])), None).unwrap();
        let prof = s.open("p", None, b"#t\na 1\n").unwrap();
        let e = s
            .hotspots(&HotspotsRequest {
                profile_id: heap.id(),
                view: ProfileView::default(),
                group_by: GroupBy::Function,
                metric: None,
                sort: None,
                search: None,
                offset: 0,
                limit: 10,
            })
            .unwrap_err();
        assert!(e.to_string().contains("is a heap snapshot, not a profile"), "{e}");
        let e = s
            .heap_types(&HeapTypesRequest {
                snapshot_id: prof.id(),
                sort: HeapSort::Bytes,
                search: None,
                offset: 0,
                limit: 10,
            })
            .unwrap_err();
        assert!(e.to_string().contains("not a heap snapshot"), "{e}");
        assert!(s
            .heap_diff(&HeapDiffRequest {
                baseline_id: 77,
                target_id: heap.id(),
                search: None,
                sort: HeapDiffSort::Name,
                offset: 0,
                limit: 1
            })
            .is_err());
        for _ in 0..MAX_OPEN - 2 {
            s.open("p", None, b"#t\na 1\n").unwrap();
        }
        assert!(matches!(s.open("p", None, b"#t\na 1\n"), Err(ProfileError::LimitExceeded { .. })));
        assert!(s.insert(Capture::Heap(graph(&[("A", 1, &[])], &[0])), None).is_err());
    }

    #[test]
    fn spans_and_moment_line_up_with_wall_clock() {
        let mut p = crate::ops::tree::tests::profile(&[(0, 0, "main;a", 1), (0, 5_000_000, "main;b", 2), (1, 9_000_000, "main;b", 1)]);
        p.started_ms = Some(1_000);
        let mut h = graph(&[("A", 1, &[])], &[0]);
        h.taken_ms = Some(1_500);
        let mut s = store();
        let id = s.insert(Capture::Profile(p), None).unwrap().id();
        s.insert(Capture::Heap(h), None).unwrap();
        let spans = s.spans();
        assert_eq!(spans.len(), 2);
        assert_eq!(
            (spans[0].from_ms, spans[0].to_ms, spans[0].capture),
            (1_000, 1_009, CaptureKind::Profile)
        );
        assert_eq!((spans[1].from_ms, spans[1].to_ms), (1_500, 1_500));
        let m = s
            .moment(&ProfileMomentRequest {
                from_ms: 1_004,
                to_ms: 1_006,
                top: Some(3),
            })
            .unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].profile_id, id);
        assert_eq!(m[0].samples, 1);
        assert_eq!(m[0].top[0].name, "b");
        assert_eq!(m[0].range.from_ns, 4_000_000);
        assert!(s
            .moment(&ProfileMomentRequest {
                from_ms: 2_000,
                to_ms: 3_000,
                top: None
            })
            .unwrap()
            .is_empty());
        assert!(s
            .moment(&ProfileMomentRequest {
                from_ms: 5,
                to_ms: 1,
                top: None
            })
            .is_err());
        let none = s
            .moment(&ProfileMomentRequest {
                from_ms: 0,
                to_ms: 1_001,
                top: Some(0),
            })
            .unwrap();
        assert!(none[0].top.is_empty());
    }

    #[test]
    fn every_view_survives_odd_captures() {
        fuzz_views(Capture::Profile(crate::ops::tree::tests::profile(&[
            (0, 0, "a;a;b", 1),
            (2, 7, "fw.x;fw.y;c", 3),
        ])));
        fuzz_views(Capture::Heap(graph(&[("A", 1, &[1]), ("B", 2, &[0])], &[0])));
        let mut b = crate::heap::HeapBuilder::new("t", Limits::default());
        b.type_stat("X", 1, 1).unwrap();
        fuzz_views(Capture::Heap(b.finish().unwrap()));
    }
}
