//! Findings for native (C/C++) code (ADR 0022): the hot native function with its source location, the
//! native function where threads wait longest, and the call path that holds most unfreed heap memory.
//!
//! "System" modules are the OS and C/C++ runtime libraries ([`SYSTEM_MODULES`]): a hot spot is reported at the
//! first frame of the user's own code above them, which is where a fix goes.

use super::{matches_any, ms, Builder, LOCKS};
use crate::api::{EvidenceView, FindingKind, FindingSeverity, HotPathRequest, TreeDirection};
use crate::model::{FrameKind, MetricId, Profile};
use crate::ops::common::{frame_dto, pct};
use crate::ops::diff::Side;
use crate::ops::nav::hot_path;
use rustc_hash::FxHashMap;

/// OS and runtime libraries (lower-case file names): never the place of a fix.
pub(super) const SYSTEM_MODULES: &[&str] = &[
    "ntdll.dll",
    "kernel32.dll",
    "kernelbase.dll",
    "ucrtbase.dll",
    "ucrtbased.dll",
    "msvcrt.dll",
    "win32u.dll",
    "user32.dll",
    "gdi32.dll",
    "gdi32full.dll",
    "combase.dll",
    "rpcrt4.dll",
    "sechost.dll",
    "advapi32.dll",
    "ws2_32.dll",
    "mswsock.dll",
    "ole32.dll",
    "oleaut32.dll",
    "shell32.dll",
    "bcrypt.dll",
    "bcryptprimitives.dll",
    "wow64.dll",
    "wow64win.dll",
    "wow64cpu.dll",
    "ntoskrnl.exe",
];

/// Prefixes of runtime library families with versioned names (`vcruntime140.dll`, `msvcp140.dll`).
const SYSTEM_PREFIXES: &[&str] = &["vcruntime", "msvcp", "api-ms-win-"];

/// Share of the CPU a native function's self time needs to be reported.
const HOT_NATIVE_PCT: f64 = 5.0;
/// Share of all wait time one function needs to be a wait hot spot, and the least wait worth it.
const WAIT_SPOT_PCT: f64 = 20.0;
const WAIT_SPOT_MIN_NS: u64 = 10_000_000;
/// Unfreed bytes worth a warning (else information).
const GROWTH_WARN_BYTES: u64 = 1 << 20;

/// Whether `module` is an OS or runtime library.
pub(super) fn is_system_module(module: Option<&str>) -> bool {
    let Some(m) = module else { return false };
    let m = m.to_ascii_lowercase();
    SYSTEM_MODULES.contains(&m.as_str()) || SYSTEM_PREFIXES.iter().any(|p| m.starts_with(p))
}

/// A native frame named from symbols (not `module+0x…` or `0x…`).
fn is_named_native(p: &Profile, frame: u32) -> bool {
    let f = &p.frames[frame as usize];
    f.kind == FrameKind::Native && !f.function.starts_with("0x") && !f.function.contains("+0x")
}

/// `file:line` (file name only) of a frame, when known.
fn location(p: &Profile, frame: u32) -> Option<String> {
    let f = &p.frames[frame as usize];
    let file = f.file.as_deref()?;
    let name = file.rsplit(['\\', '/']).next().unwrap_or(file);
    Some(match f.line {
        Some(l) => format!("{name}:{l}"),
        None => name.to_string(),
    })
}

/// The hot native function: the user-code native function with the most self time. Returns its frame so the
/// generic top-self finding does not repeat it.
pub(super) fn hot_function(b: &mut Builder<'_>, s: &Side<'_>, k: usize, grand: u64) -> Option<u32> {
    let p = s.profile;
    let mut by_frame: FxHashMap<u32, u64> = FxHashMap::default();
    for n in 0..s.tree.len() as u32 {
        let f = s.tree.frame[n as usize];
        if is_named_native(p, f) && !is_system_module(p.frames[f as usize].module.as_deref()) {
            *by_frame.entry(f).or_default() += s.tree.self_of(n, k);
        }
    }
    let (&frame, &value) = by_frame.iter().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))?;
    let share = pct(value, grand);
    if share < HOT_NATIVE_PCT {
        return None;
    }
    let f = &p.frames[frame as usize];
    let node = s.tree.nodes_of_frame(frame).iter().copied().max_by_key(|&n| s.tree.self_of(n, k));
    let mut e = b.evidence(EvidenceView::Butterfly);
    e.frame = Some(frame_dto(p, frame));
    e.path = node.map(|n| s.tree.path_of(n)).unwrap_or_default();
    let at = location(p, frame).map(|l| format!(" ({l})")).unwrap_or_default();
    let module = f.module.as_deref().map(|m| format!(" in {m}")).unwrap_or_default();
    b.push(
        FindingKind::HotNativeFunction,
        if share >= 25.0 { FindingSeverity::Warning } else { FindingSeverity::Info },
        format!("Native hot spot: {}{at} uses {share:.1}% of the time itself", f.function),
        format!(
            "The function's own code{module} is where the time goes. Look at its loops and data access at the source location, or call it less often (see its callers)."
        ),
        Some(share),
        e,
    );
    Some(frame)
}

/// The native function in which threads waited longest: wait time grouped by the first user-code frame above
/// the wait (the caller of the lock or wait API).
pub(super) fn wait_hot_spot(b: &mut Builder<'_>, s: &Side<'_>) {
    let p = s.profile;
    let Some(k) = s.tree.metrics.iter().position(|m| *m == MetricId::WaitNs) else {
        return;
    };
    let total = s.tree.grand[k];
    if total < WAIT_SPOT_MIN_NS {
        return;
    }
    let mut by_frame: FxHashMap<u32, (u64, bool)> = FxHashMap::default();
    for n in 0..s.tree.len() as u32 {
        let v = s.tree.self_of(n, k);
        if v == 0 {
            continue;
        }
        let path = s.tree.path_of(n);
        let Some(i) = path
            .iter()
            .rposition(|&f| is_named_native(p, f) && !is_system_module(p.frames[f as usize].module.as_deref()))
        else {
            continue;
        };
        let lock = path[i..]
            .iter()
            .any(|&f| matches_any(&p.frames[f as usize].function, LOCKS) || is_lock_wait(p, f));
        let e = by_frame.entry(path[i]).or_default();
        e.0 = e.0.saturating_add(v);
        e.1 |= lock;
    }
    let Some((&frame, &(value, lock))) = by_frame.iter().max_by(|a, b| a.1 .0.cmp(&b.1 .0).then(b.0.cmp(a.0))) else {
        return;
    };
    let share = pct(value, total);
    if share < WAIT_SPOT_PCT || value < WAIT_SPOT_MIN_NS {
        return;
    }
    let f = &p.frames[frame as usize];
    let node = s.tree.nodes_of_frame(frame).iter().copied().max_by_key(|&n| s.tree.total_of(n, k));
    let mut e = b.evidence(EvidenceView::CallTree);
    e.metric = MetricId::WaitNs;
    e.frame = Some(frame_dto(p, frame));
    e.path = node.map(|n| s.tree.path_of(n)).unwrap_or_default();
    let at = location(p, frame).map(|l| format!(" ({l})")).unwrap_or_default();
    let what = if lock { "on a lock" } else { "blocked" };
    b.push(
        FindingKind::WaitHotSpot,
        if lock && share >= 50.0 { FindingSeverity::Warning } else { FindingSeverity::Info },
        format!("{}{at} waited {} {what} ({share:.0}% of all waits)", f.function, ms(value)),
        if lock {
            "Threads queue for a lock taken here. Hold it for less time (move work out of the locked section), split the protected state, or use a reader/writer lock if most callers only read.".to_string()
        } else {
            "Threads block here (I/O, a wait handle, a sleep). Check whether the wait is expected (idle workers) or on a path that should be working; make the operation asynchronous or move it off that thread.".to_string()
        },
        Some(share),
        e,
    );
}

/// Frames that are the OS's lock waits (critical sections, SRW locks, wait-on-address).
fn is_lock_wait(p: &Profile, frame: u32) -> bool {
    const NT_LOCKS: &[&str] = &[
        "rtlpwaitoncriticalsection",
        "rtlpentercriticalsection",
        "rtlacquiresrwlock",
        "rtlpwaitonaddress",
    ];
    matches_any(&p.frames[frame as usize].function, NT_LOCKS)
}

/// The call path holding most unfreed native heap memory (LiveBytes).
pub(super) fn heap_growth(b: &mut Builder<'_>, s: &Side<'_>) {
    let p = s.profile;
    let Some(k) = s.tree.metrics.iter().position(|m| *m == MetricId::LiveBytes) else {
        return;
    };
    let unfreed = s.tree.grand[k];
    if unfreed == 0 {
        return;
    }
    let allocated = s
        .tree
        .metrics
        .iter()
        .position(|m| *m == MetricId::AllocBytes)
        .map_or(0, |a| s.tree.grand[a]);
    let req = HotPathRequest {
        profile_id: b.ctx.profile_id,
        view: b.ctx.view.clone(),
        direction: TreeDirection::TopDown,
        path: Vec::new(),
        metric: Some(MetricId::LiveBytes),
        min_fraction: Some(0.2),
    };
    let path = hot_path(p, s.tree, &req).map(|h| h.path).unwrap_or_default();
    // The allocation site: the deepest user-code frame of the path (not malloc or HeapAlloc themselves).
    let site = path
        .iter()
        .rev()
        .copied()
        .find(|&f| is_named_native(p, f) && !is_system_module(p.frames[f as usize].module.as_deref()))
        .or_else(|| path.last().copied());
    let under = site
        .and_then(|f| path.iter().position(|&x| x == f))
        .and_then(|i| s.tree.node_at(&path[..=i]))
        .map_or(unfreed, |n| s.tree.total_of(n, k));
    let mut e = b.evidence(EvidenceView::CallTree);
    e.metric = MetricId::LiveBytes;
    e.frame = site.map(|f| frame_dto(p, f));
    e.path = path;
    let mb = |v: u64| format!("{:.1} MB", v as f64 / 1e6);
    let share = pct(under, unfreed);
    let kept = pct(unfreed, allocated.max(unfreed));
    let warn = unfreed >= GROWTH_WARN_BYTES && kept >= 25.0;
    let at = site
        .map(|f| {
            let loc = location(p, f).map(|l| format!(" ({l})")).unwrap_or_default();
            format!("{}{loc}", p.frames[f as usize].function)
        })
        .unwrap_or_else(|| "the highlighted path".into());
    b.push(
        FindingKind::NativeHeapGrowth,
        if warn { FindingSeverity::Warning } else { FindingSeverity::Info },
        format!("{} of native heap not freed; {share:.0}% allocated under {at}", mb(unfreed)),
        format!(
            "{kept:.0}% of the {} allocated during the capture was still allocated at its end. Memory that grows across captures is a leak or an unbounded cache: check who owns these allocations and when they are released.",
            mb(allocated.max(unfreed))
        ),
        Some(share),
        e,
    );
}
