//! UMDH analysis: growth between two snapshots (or what a UMDH diff log says), grouped by stack, module or
//! function, the top-down tree of growing stacks, and leak findings over a series of snapshots.
//!
//! Stacks are matched across snapshots by their frames (`module!function` per frame), not by UMDH's back trace
//! id, which is only stable within one stack database.

use crate::api::*;
use crate::error::{DumpError, Result};
use crate::model::{UmdhFrame, UmdhKind, UmdhLog, UmdhStack};
use crate::modules::{is_allocator, is_framework};
use std::collections::HashMap;

/// A stack's growth from the older to the newer log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Growth {
    pub key: String,
    /// The newer log's frames (the older's when the stack is gone).
    pub frames: Vec<UmdhFrame>,
    pub ids: Vec<u64>,
    pub bytes: u64,
    pub count: u64,
    pub old_bytes: u64,
    pub old_count: u64,
    pub largest: u64,
}

impl Growth {
    pub fn delta_bytes(&self) -> i64 {
        self.bytes as i64 - self.old_bytes as i64
    }

    pub fn delta_count(&self) -> i64 {
        self.count as i64 - self.old_count as i64
    }
}

/// What identifies a stack across logs.
pub fn stack_key(s: &UmdhStack) -> String {
    if s.frames.is_empty() {
        return format!("#{}", s.id);
    }
    s.frames.iter().map(UmdhFrame::key).collect::<Vec<_>>().join("\n")
}

/// Whether a UMDH frame is in a system or runtime module.
pub fn frame_is_framework(f: &UmdhFrame) -> bool {
    f.module.as_deref().is_some_and(is_framework)
}

/// The frame a stack's allocations are attributed to: the first that is neither an allocator nor in a system
/// or runtime module (else the first that is not an allocator, else the first).
pub fn culprit(frames: &[UmdhFrame]) -> Option<usize> {
    let alloc = |f: &UmdhFrame| f.function.as_deref().is_some_and(is_allocator);
    frames
        .iter()
        .position(|f| !alloc(f) && !frame_is_framework(f))
        .or_else(|| frames.iter().position(|f| !alloc(f)))
        .or(if frames.is_empty() { None } else { Some(0) })
}

fn add(map: &mut HashMap<String, Growth>, order: &mut Vec<String>, s: &UmdhStack, new: bool) {
    let key = stack_key(s);
    let g = map.entry(key.clone()).or_insert_with(|| {
        order.push(key.clone());
        Growth {
            key,
            frames: s.frames.clone(),
            ids: Vec::new(),
            bytes: 0,
            count: 0,
            old_bytes: 0,
            old_count: 0,
            largest: 0,
        }
    });
    if new {
        g.bytes = g.bytes.saturating_add(s.bytes);
        g.count = g.count.saturating_add(s.count);
        g.largest = g.largest.max(s.largest);
        g.ids.push(s.id);
        g.frames.clone_from(&s.frames);
        if let (Some(b), Some(c)) = (s.old_bytes, s.old_count) {
            g.old_bytes = g.old_bytes.saturating_add(b);
            g.old_count = g.old_count.saturating_add(c);
        }
    } else {
        g.old_bytes = g.old_bytes.saturating_add(s.bytes);
        g.old_count = g.old_count.saturating_add(s.count);
    }
}

/// The growth of every stack from `base` to `target`, most growth first. Without `base`, a diff log's own
/// older side is used (a snapshot counts as growth from nothing).
pub fn growth(base: Option<&UmdhLog>, target: &UmdhLog) -> Result<Vec<Growth>> {
    let mut map = HashMap::new();
    let mut order = Vec::new();
    if let Some(b) = base {
        if b.kind != UmdhKind::Snapshot || target.kind != UmdhKind::Snapshot {
            return Err(DumpError::invalid(
                "only two UMDH snapshots can be compared; a UMDH diff log already holds its comparison",
            ));
        }
        for s in &b.stacks {
            add(&mut map, &mut order, s, false);
        }
    }
    for s in &target.stacks {
        add(&mut map, &mut order, s, true);
    }
    let mut out: Vec<Growth> = order.into_iter().filter_map(|k| map.remove(&k)).collect();
    out.sort_by(|a, b| b.delta_bytes().cmp(&a.delta_bytes()).then_with(|| a.key.cmp(&b.key)));
    Ok(out)
}

fn culprit_frame(g: &Growth) -> Option<&UmdhFrame> {
    culprit(&g.frames).and_then(|i| g.frames.get(i))
}

fn label_of(g: &Growth) -> String {
    culprit_frame(g).map_or_else(|| g.key.clone(), UmdhFrame::label)
}

/// The growth grouped and paged.
pub fn diff_page(rows: &[Growth], req: &UmdhDiffRequest, max_page: u32) -> UmdhDiffPage {
    let mut groups: Vec<UmdhRow> = match req.group {
        UmdhGroup::Stack => rows
            .iter()
            .map(|g| UmdhRow {
                key: g.key.clone(),
                label: label_of(g),
                module: culprit_frame(g).and_then(|f| f.module.clone()),
                delta_bytes: g.delta_bytes(),
                delta_count: g.delta_count(),
                bytes: g.bytes,
                count: g.count,
                stacks: 1,
            })
            .collect(),
        UmdhGroup::Module | UmdhGroup::Function => {
            let mut map: HashMap<String, UmdhRow> = HashMap::new();
            for g in rows {
                let f = culprit_frame(g);
                let module = f.and_then(|f| f.module.clone());
                let key = match req.group {
                    UmdhGroup::Module => module.clone().unwrap_or_else(|| "?".into()),
                    _ => f.map_or_else(|| g.key.clone(), UmdhFrame::key),
                };
                let r = map.entry(key.clone()).or_insert_with(|| UmdhRow {
                    label: key.clone(),
                    key,
                    module,
                    delta_bytes: 0,
                    delta_count: 0,
                    bytes: 0,
                    count: 0,
                    stacks: 0,
                });
                r.delta_bytes = r.delta_bytes.saturating_add(g.delta_bytes());
                r.delta_count = r.delta_count.saturating_add(g.delta_count());
                r.bytes = r.bytes.saturating_add(g.bytes);
                r.count = r.count.saturating_add(g.count);
                r.stacks += 1;
            }
            map.into_values().collect()
        }
    };
    if let Some(f) = req.filter.as_deref().map(str::trim).filter(|f| !f.is_empty()) {
        let f = f.to_lowercase();
        groups.retain(|r| r.label.to_lowercase().contains(&f) || r.key.to_lowercase().contains(&f));
    }
    groups.sort_by(|a, b| b.delta_bytes.cmp(&a.delta_bytes).then_with(|| a.key.cmp(&b.key)));
    UmdhDiffPage {
        total: groups.len() as u32,
        delta_bytes: rows.iter().map(Growth::delta_bytes).sum(),
        delta_count: rows.iter().map(Growth::delta_count).sum(),
        bytes: rows.iter().map(|g| g.bytes).sum(),
        rows: groups
            .into_iter()
            .skip(req.offset as usize)
            .take(req.limit.min(max_page) as usize)
            .collect(),
    }
}

/// UMDH frames as rows (innermost first), with the culprit marked.
pub fn frame_rows(frames: &[UmdhFrame]) -> Vec<FrameRow> {
    let culprit = culprit(frames);
    frames
        .iter()
        .enumerate()
        .map(|(i, f)| FrameRow {
            index: i as u32,
            address: f.address.map(crate::views::hex).unwrap_or_default(),
            module: f.module.clone(),
            function: f.function.clone(),
            label: f.label(),
            source: f.file.as_ref().map(|file| match f.line {
                Some(l) => format!("{file}:{l}"),
                None => file.clone(),
            }),
            trust: FrameTrust::Recorded,
            framework: frame_is_framework(f),
            culprit: culprit == Some(i),
        })
        .collect()
}

/// One stack's growth and frames.
pub fn stack(rows: &[Growth], key: &str) -> Result<UmdhStackDto> {
    let g = rows
        .iter()
        .find(|g| g.key == key)
        .ok_or_else(|| DumpError::not_found("no such allocation stack"))?;
    Ok(UmdhStackDto {
        key: g.key.clone(),
        delta_bytes: g.delta_bytes(),
        delta_count: g.delta_count(),
        ids: g.ids.clone(),
        frames: frame_rows(&g.frames),
    })
}

/// The children of `path` in the top-down tree (outermost caller first) of the stacks, most growth first.
pub fn tree(rows: &[Growth], path: &[String], limit: u32) -> Vec<UmdhTreeNode> {
    struct Acc {
        label: String,
        delta_bytes: i64,
        delta_count: i64,
        has_children: bool,
        framework: bool,
    }
    let mut map: HashMap<String, Acc> = HashMap::new();
    for g in rows {
        let rev: Vec<&UmdhFrame> = g.frames.iter().rev().collect();
        if rev.len() <= path.len() || !rev.iter().zip(path).all(|(f, p)| f.key() == *p) {
            continue;
        }
        let f = rev[path.len()];
        let a = map.entry(f.key()).or_insert_with(|| Acc {
            label: f.label(),
            delta_bytes: 0,
            delta_count: 0,
            has_children: false,
            framework: frame_is_framework(f),
        });
        a.delta_bytes = a.delta_bytes.saturating_add(g.delta_bytes());
        a.delta_count = a.delta_count.saturating_add(g.delta_count());
        a.has_children |= rev.len() > path.len() + 1;
    }
    let mut out: Vec<UmdhTreeNode> = map
        .into_iter()
        .map(|(key, a)| UmdhTreeNode {
            key,
            label: a.label,
            delta_bytes: a.delta_bytes,
            delta_count: a.delta_count,
            has_children: a.has_children,
            framework: a.framework,
        })
        .collect();
    out.sort_by(|a, b| b.delta_bytes.cmp(&a.delta_bytes).then_with(|| a.key.cmp(&b.key)));
    out.truncate(limit as usize);
    out
}

/// `12.3 MB`.
pub fn fmt_bytes(b: i64) -> String {
    let sign = if b < 0 { "-" } else { "" };
    let v = b.unsigned_abs() as f64;
    let (n, unit) = if v >= 1024.0 * 1024.0 * 1024.0 {
        (v / (1024.0 * 1024.0 * 1024.0), "GB")
    } else if v >= 1024.0 * 1024.0 {
        (v / (1024.0 * 1024.0), "MB")
    } else if v >= 1024.0 {
        (v / 1024.0, "KB")
    } else {
        return format!("{sign}{} bytes", b.unsigned_abs());
    };
    format!("{sign}{n:.1} {unit}")
}

/// Growth at least this large (or this share of the total) is a leak suspect.
const LEAK_BYTES: i64 = 1 << 20;
const LEAK_SHARE: f64 = 0.10;
/// A single allocation at least this large is reported.
const LARGE_ALLOCATION: u64 = 16 << 20;
/// Findings of one kind listed at most.
const TOP: usize = 5;

fn evidence(doc: u32, key: Option<String>, module: Option<String>) -> FindingEvidence {
    FindingEvidence {
        doc_id: doc,
        thread_id: None,
        frame: None,
        module,
        stack: key,
        threads: Vec::new(),
    }
}

/// Leak findings over `logs` (oldest first, with their document ids): the top growing stacks between the first
/// and the last snapshot (or what a single diff log says), stacks that grow in every step of three or more
/// snapshots, and very large single allocations.
pub fn findings(logs: &[(u32, &UmdhLog)]) -> Result<Vec<DumpFinding>> {
    let Some(&(last_id, last)) = logs.last() else {
        return Ok(Vec::new());
    };
    let snapshots = logs.iter().all(|(_, l)| l.kind == UmdhKind::Snapshot);
    if logs.len() > 1 && !snapshots {
        return Err(DumpError::invalid("a series of UMDH logs must be snapshots of one process"));
    }
    let base = (logs.len() > 1).then(|| logs[0].1);
    let rows = growth(base, last)?;
    let mut out = Vec::new();
    let growing: i64 = rows.iter().map(Growth::delta_bytes).filter(|d| *d > 0).sum();
    let compared = last.kind == UmdhKind::Diff || logs.len() > 1;
    if compared {
        let total: i64 = rows.iter().map(Growth::delta_bytes).sum();
        let before: u64 = rows.iter().map(|g| g.old_bytes).sum();
        let after: u64 = rows.iter().map(|g| g.bytes).sum();
        out.push(DumpFinding {
            kind: FindingKind::HeapTotal,
            severity: Severity::Info,
            title: format!(
                "The heap changed by {} ({} → {})",
                fmt_bytes(total),
                fmt_bytes(before as i64),
                fmt_bytes(after as i64)
            ),
            detail: if logs.len() > 1 {
                format!("Compared the first and the last of {} snapshots.", logs.len())
            } else {
                "As computed by UMDH between its two snapshots.".into()
            },
            evidence: evidence(last_id, None, None),
        });
        for g in rows.iter().filter(|g| g.delta_bytes() > 0).take(TOP) {
            let d = g.delta_bytes();
            let share = if growing > 0 { d as f64 / growing as f64 } else { 0.0 };
            if d < LEAK_BYTES && share < LEAK_SHARE {
                continue;
            }
            let f = culprit_frame(g);
            out.push(DumpFinding {
                kind: FindingKind::LeakGrowth,
                severity: if share >= 0.5 && d >= LEAK_BYTES {
                    Severity::Critical
                } else {
                    Severity::Warning
                },
                title: format!(
                    "{} grew by {} ({:+} allocations)",
                    label_of(g),
                    fmt_bytes(d),
                    g.delta_count()
                ),
                detail: format!(
                    "{:.0}% of the heap's growth comes from this allocation stack ({} → {}). Memory allocated here is not freed; look at who keeps it.",
                    share * 100.0,
                    fmt_bytes(g.old_bytes as i64),
                    fmt_bytes(g.bytes as i64)
                ),
                evidence: evidence(last_id, Some(g.key.clone()), f.and_then(|f| f.module.clone())),
            });
        }
    }
    if logs.len() >= 3 {
        let series: Vec<HashMap<String, u64>> = logs
            .iter()
            .map(|(_, l)| {
                let mut m: HashMap<String, u64> = HashMap::new();
                for s in &l.stacks {
                    let e = m.entry(stack_key(s)).or_default();
                    *e = e.saturating_add(s.bytes);
                }
                m
            })
            .collect();
        let mut steady: Vec<&Growth> = rows
            .iter()
            .filter(|g| {
                let values: Vec<u64> = series.iter().map(|m| m.get(&g.key).copied().unwrap_or(0)).collect();
                values.windows(2).all(|w| w[1] > w[0])
            })
            .collect();
        steady.sort_by_key(|g| std::cmp::Reverse(g.delta_bytes()));
        for g in steady.into_iter().take(TOP) {
            let values: Vec<String> = series
                .iter()
                .map(|m| fmt_bytes(m.get(&g.key).copied().unwrap_or(0) as i64))
                .collect();
            out.push(DumpFinding {
                kind: FindingKind::SteadyGrowth,
                severity: Severity::Critical,
                title: format!("Steady growth in {} across {} snapshots", label_of(g), logs.len()),
                detail: format!(
                    "It grew in every snapshot ({}): a leak rather than a cache warming up.",
                    values.join(" → ")
                ),
                evidence: evidence(last_id, Some(g.key.clone()), culprit_frame(g).and_then(|f| f.module.clone())),
            });
        }
    }
    let mut large: Vec<&Growth> = rows.iter().filter(|g| g.largest >= LARGE_ALLOCATION).collect();
    large.sort_by_key(|g| std::cmp::Reverse(g.largest));
    for g in large.into_iter().take(TOP) {
        out.push(DumpFinding {
            kind: FindingKind::LargeAllocation,
            severity: Severity::Warning,
            title: format!("A single allocation of {} from {}", fmt_bytes(g.largest as i64), label_of(g)),
            detail: "Very large single allocations fragment the heap and fail first when address space runs low.".into(),
            evidence: evidence(last_id, Some(g.key.clone()), culprit_frame(g).and_then(|f| f.module.clone())),
        });
    }
    if !compared && out.is_empty() {
        let total: u64 = last.stacks.iter().map(|s| s.bytes).sum();
        out.push(DumpFinding {
            kind: FindingKind::HeapTotal,
            severity: Severity::Info,
            title: format!("{} in {} allocation stacks", fmt_bytes(total as i64), last.stacks.len()),
            detail: "One snapshot shows what is allocated, not what leaks: open a later snapshot of the same process to compare.".into(),
            evidence: evidence(last_id, None, None),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::sample_umdh;

    fn req(group: UmdhGroup) -> UmdhDiffRequest {
        UmdhDiffRequest {
            base: None,
            target: 1,
            group,
            offset: 0,
            limit: 100,
            filter: None,
        }
    }

    #[test]
    fn snapshots_are_compared_by_frames_and_grouped() {
        let a = sample_umdh(UmdhKind::Snapshot, 0, &[(1, 1000, 10), (2, 500, 5)]);
        // Different back trace ids for the same stacks, plus a new one.
        let b = sample_umdh(UmdhKind::Snapshot, 10, &[(4, 5000, 50), (5, 400, 4), (3, 300, 3)]);
        let rows = growth(Some(&a), &b).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].delta_bytes(), 4000, "stack 1 (Cache::Add) and 4 match by frames");
        assert_eq!(rows[0].ids, [4]);
        assert_eq!(label_of(&rows[0]), "Shop.Orders!Cache::Add+0x34");
        assert_eq!(rows.last().unwrap().delta_bytes(), -100);
        let page = diff_page(&rows, &req(UmdhGroup::Stack), 1000);
        assert_eq!((page.total, page.delta_bytes, page.bytes), (3, 4200, 5700));
        let by_module = diff_page(&rows, &req(UmdhGroup::Module), 1000);
        assert_eq!(by_module.rows.len(), 1);
        assert_eq!((by_module.rows[0].label.as_str(), by_module.rows[0].stacks), ("Shop.Orders", 3));
        let by_fn = diff_page(&rows, &req(UmdhGroup::Function), 1000);
        assert_eq!(by_fn.rows[0].key, "Shop.Orders!Cache::Add");
        let mut filtered = req(UmdhGroup::Function);
        filtered.filter = Some("report".into());
        assert_eq!(diff_page(&rows, &filtered, 1000).rows.len(), 1);
        let paged = diff_page(&rows, &req(UmdhGroup::Stack), 1);
        assert_eq!((paged.rows.len(), paged.total), (1, 3));
        let s = stack(&rows, &rows[0].key).unwrap();
        assert!(s.frames[3].culprit && s.frames[0].framework && !s.frames[3].framework);
        assert_eq!(stack(&rows, "nope").unwrap_err().code, crate::error::ErrorCode::NotFound);
        let d = sample_umdh(UmdhKind::Diff, 0, &[(1, 100, 1)]);
        assert!(growth(Some(&a), &d).is_err(), "a diff log is not compared again");
    }

    #[test]
    fn diff_logs_carry_both_sides_and_the_tree_is_top_down() {
        let d = sample_umdh(UmdhKind::Diff, 0, &[(1, 4000, 40), (2, 1000, 10)]);
        let rows = growth(None, &d).unwrap();
        assert_eq!((rows[0].delta_bytes(), rows[0].delta_count()), (2000, 20));
        let roots = tree(&rows, &[], 10);
        assert_eq!(roots.len(), 1);
        assert_eq!(
            (roots[0].key.as_str(), roots[0].delta_bytes, roots[0].has_children),
            ("OrderService!main", 2500, true)
        );
        let kids = tree(&rows, &[roots[0].key.clone()], 10);
        assert_eq!(kids.len(), 2);
        assert_eq!(kids[0].key, "Shop.Orders!Cache::Add");
        let leaf_path: Vec<String> = rows[0].frames.iter().rev().map(UmdhFrame::key).collect();
        assert!(tree(&rows, &leaf_path, 10).is_empty());
        assert_eq!(tree(&rows, &[roots[0].key.clone()], 1).len(), 1);
        let none = sample_umdh(UmdhKind::Snapshot, 0, &[]);
        assert!(growth(None, &none).unwrap().is_empty());
    }

    #[test]
    fn leak_findings() {
        let mib = 1 << 20;
        let s1 = sample_umdh(UmdhKind::Snapshot, 0, &[(1, 10 * mib, 10), (2, mib, 1), (3, 64 * mib, 1)]);
        let s2 = sample_umdh(UmdhKind::Snapshot, 5, &[(1, 20 * mib, 20), (2, mib / 2, 1), (3, 64 * mib, 1)]);
        let s3 = sample_umdh(UmdhKind::Snapshot, 10, &[(1, 30 * mib, 30), (2, 2 * mib, 2), (3, 64 * mib, 1)]);
        let f = findings(&[(1, &s1), (2, &s2), (3, &s3)]).unwrap();
        let kinds: Vec<FindingKind> = f.iter().map(|f| f.kind).collect();
        assert_eq!(kinds[0], FindingKind::HeapTotal);
        let leak = f.iter().find(|f| f.kind == FindingKind::LeakGrowth).unwrap();
        assert_eq!(leak.severity, Severity::Critical);
        assert!(
            leak.title.starts_with("Shop.Orders!Cache::Add+0x34 grew by 20.0 MB"),
            "{}",
            leak.title
        );
        assert_eq!(leak.evidence.doc_id, 3);
        let steady: Vec<&DumpFinding> = f.iter().filter(|f| f.kind == FindingKind::SteadyGrowth).collect();
        assert_eq!(steady.len(), 1, "stack 2 shrank once");
        assert!(steady[0].detail.contains("10.0 MB → 20.0 MB → 30.0 MB"));
        let large = f.iter().find(|f| f.kind == FindingKind::LargeAllocation).unwrap();
        assert!(large.title.contains("64.0 MB"));

        let one = findings(&[(1, &s1)]).unwrap();
        assert!(one.iter().all(|f| f.kind != FindingKind::LeakGrowth));
        let small = sample_umdh(UmdhKind::Snapshot, 0, &[(1, 100, 1)]);
        let only = findings(&[(1, &small)]).unwrap();
        assert_eq!(only.len(), 1);
        assert!(only[0].title.contains("1 allocation stacks"));
        let d = sample_umdh(UmdhKind::Diff, 0, &[(1, 8 * mib, 8)]);
        assert!(findings(&[(1, &d)]).unwrap().iter().any(|f| f.kind == FindingKind::LeakGrowth));
        assert!(findings(&[(1, &s1), (2, &d)]).is_err());
        assert!(findings(&[]).unwrap().is_empty());
    }

    #[test]
    fn bytes_are_formatted() {
        assert_eq!(fmt_bytes(512), "512 bytes");
        assert_eq!(fmt_bytes(-2048), "-2.0 KB");
        assert_eq!(fmt_bytes(3 << 20), "3.0 MB");
        assert_eq!(fmt_bytes(5 << 30), "5.0 GB");
        assert_eq!(culprit(&[]), None);
    }
}
