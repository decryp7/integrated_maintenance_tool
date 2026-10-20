//! Profiling commands (per window): open CPU/allocation profiles and heap
//! snapshots and query their views. The engine is `imt_profile`; each window
//! has its own [`ProfilingSlot`] holding a `dyn ProfileStore`. Importers come
//! from the `profiling.importers` extension point of the window's plugin
//! registry and are called through their guarded service handles, so a
//! crashing importer is contained and counted against its plugin.
//!
//! Every command rejects with the one [`crate::CmdError`] (profiling's codes are part of it),
//! so the UI reacts to the code (`notACapture`, `notFound`, `io`, ...)
//! instead of matching message text.
//!
//! Wire contract: `app/src/app/core/api.types.ts` (section "Profiling").

use crate::paths::WindowGrants;
use crate::windows::CurrentWindow;
use imt_core::plugin::services::Service;
use imt_profile::api::*;
use imt_profile::error::ErrorCode;
use imt_profile::import::SNIFF_BYTES;
use imt_profile::{Capture, CaptureImporter, Captures, Limits, ProfileError, ProfileStore};
use std::io::Read;
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// Result of a profiling command.
pub(crate) type ProfResult<T> = crate::CmdResult<T>;

/// Time an importer may take for one file. Imports of large captures take
/// minutes, far more than the ordinary service budget; past this the import
/// is abandoned (the plugin is marked hung) and the user gets an error.
pub(crate) const IMPORT_BUDGET: Duration = Duration::from_secs(600);

/// Paths one `profiling_sniff` call accepts (one drop).
pub(crate) const MAX_SNIFF_PATHS: usize = 1_000;

/// One window's profiling store.
pub(crate) struct ProfilingSlot {
    store: Arc<RwLock<Box<dyn ProfileStore>>>,
}

impl Default for ProfilingSlot {
    fn default() -> Self {
        ProfilingSlot::new(Vec::new())
    }
}

impl ProfilingSlot {
    pub(crate) fn new(importers: Vec<Arc<dyn CaptureImporter>>) -> ProfilingSlot {
        ProfilingSlot {
            store: Arc::new(RwLock::new(Box::new(Captures::new(importers, Limits::default())))),
        }
    }

    /// A store using the importers plugins contributed to `profiling.importers`.
    pub(crate) fn from_registry(registry: &imt_core::PluginRegistry) -> ProfilingSlot {
        let importers = registry
            .contributions::<dyn CaptureImporter>(imt_profile::IMPORTERS_POINT)
            .into_iter()
            .filter_map(GuardedImporter::new)
            .map(|g| Arc::new(g) as Arc<dyn CaptureImporter>)
            .collect();
        ProfilingSlot::new(importers)
    }
}

/// An importer contribution called under its plugin's guard: a panic or a
/// quarantined plugin becomes an error, never a crash.
struct GuardedImporter {
    id: String,
    name: String,
    produces: CaptureKind,
    service: Service<dyn CaptureImporter>,
    budget: Duration,
}

impl GuardedImporter {
    fn new(service: Service<dyn CaptureImporter>) -> Option<GuardedImporter> {
        GuardedImporter::within(service, IMPORT_BUDGET)
    }

    fn within(service: Service<dyn CaptureImporter>, budget: Duration) -> Option<GuardedImporter> {
        let (id, name, produces) = service.call(|i| (i.id().to_string(), i.name().to_string(), i.produces())).ok()?;
        Some(GuardedImporter {
            id,
            name,
            produces,
            service,
            budget,
        })
    }
}

impl CaptureImporter for GuardedImporter {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn sniff(&self, file_name: &str, head: &[u8]) -> u8 {
        self.service.call(|i| i.sniff(file_name, head)).unwrap_or(0)
    }

    /// Runs on a helper thread under the import budget (not the short
    /// service budget): the bytes are handed over as an owned copy.
    fn import(&self, file_name: &str, bytes: &[u8], limits: &Limits) -> imt_profile::Result<Capture> {
        let (name, data, limits) = (file_name.to_string(), bytes.to_vec(), *limits);
        match self
            .service
            .call_detached_within(self.budget, move |i| i.import(&name, &data, &limits))
        {
            Ok(result) => result,
            Err(e) => Err(ProfileError::Importer {
                importer: self.id.clone(),
                message: format!("{e:#}"),
            }),
        }
    }

    fn produces(&self) -> CaptureKind {
        self.produces
    }
}

/// A failed blocking task (a panic in the engine) as a command error.
fn join_error(e: impl std::fmt::Display) -> ProfilingErrorDto {
    ProfilingErrorDto::new(ErrorCode::Io, format!("the profiling engine failed: {e}"))
}

async fn read<T: Send + 'static>(
    win: &CurrentWindow,
    f: impl FnOnce(&dyn ProfileStore) -> imt_profile::Result<T> + Send + 'static,
) -> ProfResult<T> {
    let store = win.profiling.store.clone();
    crate::error::lift_dto(
        tauri::async_runtime::spawn_blocking(move || f(crate::sync::read(&store).as_ref()).map_err(ProfilingErrorDto::from))
            .await
            .map_err(join_error)?,
    )
}

async fn write<T: Send + 'static>(
    win: &CurrentWindow,
    f: impl FnOnce(&mut dyn ProfileStore) -> imt_profile::Result<T> + Send + 'static,
) -> ProfResult<T> {
    let store = win.profiling.store.clone();
    crate::error::lift_dto(
        tauri::async_runtime::spawn_blocking(move || f(crate::sync::write(&store).as_mut()).map_err(ProfilingErrorDto::from))
            .await
            .map_err(join_error)?,
    )
}

/// An I/O error about `path`: `notFound` when it does not exist, `io` otherwise.
fn io_error(what: &str, path: &Path, e: &std::io::Error) -> ProfilingErrorDto {
    let code = if e.kind() == std::io::ErrorKind::NotFound {
        ErrorCode::NotFound
    } else {
        ErrorCode::Io
    };
    ProfilingErrorDto::new(code, format!("cannot {what} {}: {e}", path.display()))
}

/// Resolves a path the user chose to a canonical regular file.
fn canonical_file(path: &str) -> ProfResult<(std::path::PathBuf, std::fs::Metadata)> {
    let path = Path::new(path);
    let canonical = path.canonicalize().map_err(|e| io_error("open", path, &e))?;
    let meta = std::fs::metadata(&canonical).map_err(|e| io_error("open", &canonical, &e))?;
    if !meta.is_file() {
        return Err(crate::CmdError::from(ProfilingErrorDto::new(
            ErrorCode::Invalid,
            format!("{} is not a file", canonical.display()),
        )));
    }
    Ok((canonical, meta))
}

/// Reads a capture the user chose: canonical path, a regular file, within the size limit.
fn read_capture(path: &str) -> ProfResult<(String, String, Vec<u8>)> {
    let (canonical, meta) = canonical_file(path)?;
    let max = Limits::default().max_input_bytes;
    if meta.len() > max {
        return Err(crate::CmdError::from(ProfilingErrorDto::new(
            ErrorCode::Limit,
            format!(
                "{} is {} MB; captures up to {} MB can be opened",
                canonical.display(),
                meta.len() >> 20,
                max >> 20
            ),
        )));
    }
    let bytes = std::fs::read(&canonical).map_err(|e| io_error("read", &canonical, &e))?;
    let name = canonical.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Ok((name, canonical.to_string_lossy().into_owned(), bytes))
}

/// File name and first bytes of a path, for sniffing; `None` when it is not a readable file.
fn read_head(path: &str) -> Option<(String, Vec<u8>)> {
    let (canonical, _) = canonical_file(path).ok()?;
    let mut head = Vec::with_capacity(SNIFF_BYTES);
    std::fs::File::open(&canonical)
        .ok()?
        .take(SNIFF_BYTES as u64)
        .read_to_end(&mut head)
        .ok()?;
    let name = canonical.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Some((name, head))
}

/// What each path is (profile, heap snapshot or neither), from its name and
/// first 64 KiB, without opening it: dropped files are routed with it.
/// Folders, unreadable paths and paths the user did not choose (dropped, picked) are `none` (the log
/// import reports them).
#[tauri::command]
pub async fn profiling_sniff(state: CurrentWindow, grants: WindowGrants, paths: Vec<String>) -> ProfResult<Vec<SniffResult>> {
    if paths.len() > MAX_SNIFF_PATHS {
        return Err(crate::CmdError::from(ProfilingErrorDto::new(
            ErrorCode::Limit,
            format!("at most {MAX_SNIFF_PATHS} files can be checked at once"),
        )));
    }
    // Read the heads first, without holding the store.
    let heads = tauri::async_runtime::spawn_blocking(move || {
        paths
            .into_iter()
            .map(|p| (grants.read_file(&p, &[]).ok().and_then(|_| read_head(&p)), p))
            .collect::<Vec<_>>()
    })
    .await
    .map_err(join_error)?;
    read(&state, move |s| {
        Ok(heads
            .into_iter()
            .map(|(head, path)| {
                let kind = head.map_or(SniffKind::None, |(name, head)| s.sniff(&name, &head));
                SniffResult { path, kind }
            })
            .collect())
    })
    .await
}

/// Opens a profile or heap snapshot the user chose (any supported format, recognised by content). A
/// missing file is `notFound`; one nobody chose is `invalid`.
#[tauri::command]
pub async fn profiling_open(state: CurrentWindow, grants: WindowGrants, path: String) -> ProfResult<CaptureSummary> {
    let (name, canonical, bytes) = tauri::async_runtime::spawn_blocking(move || {
        canonical_file(&path)?;
        grants
            .read_file(&path, &[])
            .map_err(|e| ProfilingErrorDto::new(ErrorCode::Invalid, e.message))?;
        read_capture(&path)
    })
    .await
    .map_err(join_error)??;
    write(&state, move |s| s.open(&name, Some(&canonical), &bytes)).await
}

#[tauri::command]
pub async fn profiling_list(state: CurrentWindow) -> ProfResult<Vec<CaptureSummary>> {
    read(&state, |s| Ok(s.list())).await
}

/// Closes a capture; `false` when it was not open.
#[tauri::command]
pub async fn profiling_close(state: CurrentWindow, id: u32) -> ProfResult<bool> {
    write(&state, move |s| Ok(s.close(id))).await
}

/// Wall-clock spans of the open captures, for the log timeline.
#[tauri::command]
pub async fn profiling_spans(state: CurrentWindow) -> ProfResult<Vec<ProfileSpan>> {
    read(&state, |s| Ok(s.spans())).await
}

#[tauri::command]
pub async fn profiling_hotspots(state: CurrentWindow, request: HotspotsRequest) -> ProfResult<HotspotsPage> {
    read(&state, move |s| s.hotspots(&request)).await
}

#[tauri::command]
pub async fn profiling_tree(state: CurrentWindow, request: TreeRequest) -> ProfResult<TreePage> {
    read(&state, move |s| s.tree(&request)).await
}

#[tauri::command]
pub async fn profiling_hot_path(state: CurrentWindow, request: HotPathRequest) -> ProfResult<HotPath> {
    read(&state, move |s| s.hot_path(&request)).await
}

/// The hottest top-down paths to a function ("show in tree", "zoom the flame graph to it").
#[tauri::command]
pub async fn profiling_frame_paths(state: CurrentWindow, request: FramePathsRequest) -> ProfResult<FramePaths> {
    read(&state, move |s| s.frame_paths(&request)).await
}

#[tauri::command]
pub async fn profiling_butterfly(state: CurrentWindow, request: ButterflyRequest) -> ProfResult<Butterfly> {
    read(&state, move |s| s.butterfly(&request)).await
}

#[tauri::command]
pub async fn profiling_flame(state: CurrentWindow, request: FlameRequest) -> ProfResult<FlameGraph> {
    read(&state, move |s| s.flame(&request)).await
}

#[tauri::command]
pub async fn profiling_diff_flame(state: CurrentWindow, request: DiffFlameRequest) -> ProfResult<FlameGraph> {
    read(&state, move |s| s.diff_flame(&request)).await
}

#[tauri::command]
pub async fn profiling_function_diff(state: CurrentWindow, request: FunctionDiffRequest) -> ProfResult<FunctionDiffPage> {
    read(&state, move |s| s.function_diff(&request)).await
}

#[tauri::command]
pub async fn profiling_threads(state: CurrentWindow, request: ThreadTimelineRequest) -> ProfResult<ThreadTimeline> {
    read(&state, move |s| s.thread_timeline(&request)).await
}

#[tauri::command]
pub async fn profiling_findings(state: CurrentWindow, request: FindingsRequest) -> ProfResult<Vec<ProfileFinding>> {
    read(&state, move |s| s.findings(&request)).await
}

/// "Profile samples in this window" for the Moment view.
#[tauri::command]
pub async fn profiling_moment(state: CurrentWindow, request: ProfileMomentRequest) -> ProfResult<Vec<ProfileMoment>> {
    read(&state, move |s| s.moment(&request)).await
}

#[tauri::command]
pub async fn profiling_heap_types(state: CurrentWindow, request: HeapTypesRequest) -> ProfResult<HeapTypesPage> {
    read(&state, move |s| s.heap_types(&request)).await
}

#[tauri::command]
pub async fn profiling_heap_diff(state: CurrentWindow, request: HeapDiffRequest) -> ProfResult<HeapDiffPage> {
    read(&state, move |s| s.heap_diff(&request)).await
}

#[tauri::command]
pub async fn profiling_heap_growth(state: CurrentWindow, request: HeapGrowthRequest) -> ProfResult<HeapGrowthPage> {
    read(&state, move |s| s.heap_growth(&request)).await
}

#[tauri::command]
pub async fn profiling_heap_dominators(state: CurrentWindow, request: HeapDominatorsRequest) -> ProfResult<HeapObjectsPage> {
    read(&state, move |s| s.heap_dominators(&request)).await
}

#[tauri::command]
pub async fn profiling_heap_instances(state: CurrentWindow, request: HeapInstancesRequest) -> ProfResult<HeapObjectsPage> {
    read(&state, move |s| s.heap_instances(&request)).await
}

#[tauri::command]
pub async fn profiling_heap_paths(state: CurrentWindow, request: HeapPathsRequest) -> ProfResult<HeapPaths> {
    read(&state, move |s| s.heap_paths(&request)).await
}

/// Automatic findings over a heap snapshot (growth against `baselineId`).
#[tauri::command]
pub async fn profiling_heap_findings(state: CurrentWindow, request: HeapFindingsRequest) -> ProfResult<Vec<HeapFinding>> {
    read(&state, move |s| s.heap_findings(&request)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{samples, test_app_with_commands, window, TempDir};
    use serde_json::{json, Value};
    use tauri::test::MockRuntime;

    fn invoke(w: &tauri::WebviewWindow<MockRuntime>, cmd: &str, body: Value) -> Result<Value, Value> {
        use tauri::ipc::{CallbackFn, InvokeBody, InvokeResponseBody};
        use tauri::test::{get_ipc_response, INVOKE_KEY};
        use tauri::webview::InvokeRequest;
        get_ipc_response(
            w,
            InvokeRequest {
                cmd: cmd.into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: crate::test_support::ipc_url(),
                body: InvokeBody::Json(body),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            },
        )
        .map(|r| match r {
            InvokeResponseBody::Json(s) => serde_json::from_str::<Value>(&s).unwrap(),
            InvokeResponseBody::Raw(_) => Value::Null,
        })
    }

    fn sample(name: &str) -> String {
        samples().join("profiling").join(name).to_string_lossy().into_owned()
    }

    #[test]
    fn commands_open_query_and_close_per_window() {
        let app = test_app_with_commands(None);
        let a = window(&app, "main");
        let b = window(&app, "w-2");
        let base = invoke(&a, "profiling_open", json!({ "path": sample("orders.folded") })).unwrap();
        assert_eq!(base["capture"], "profile");
        assert_eq!(base["format"], "folded");
        let target = invoke(&a, "profiling_open", json!({ "path": sample("orders-regression.folded") })).unwrap();
        let (bid, tid) = (base["id"].clone(), target["id"].clone());
        assert_eq!(invoke(&a, "profiling_list", json!({})).unwrap().as_array().unwrap().len(), 2);
        assert_eq!(
            invoke(&b, "profiling_list", json!({})).unwrap().as_array().unwrap().len(),
            0,
            "per window"
        );

        let hot = invoke(&a, "profiling_hotspots", json!({ "request": { "profileId": tid, "limit": 5 } })).unwrap();
        assert_eq!(hot["rows"].as_array().unwrap().len(), 5);
        let top = &hot["rows"][0];
        let frame = top["frame"]["id"].clone();
        let tree = invoke(
            &a,
            "profiling_tree",
            json!({ "request": { "profileId": tid, "direction": "bottomUp" } }),
        )
        .unwrap();
        assert!(tree["totalChildren"].as_u64().unwrap() > 0);
        let path = invoke(&a, "profiling_hot_path", json!({ "request": { "profileId": tid } })).unwrap();
        assert!(!path["path"].as_array().unwrap().is_empty());
        let bf = invoke(
            &a,
            "profiling_butterfly",
            json!({ "request": { "profileId": tid, "frame": frame } }),
        )
        .unwrap();
        assert!(bf["function"]["name"].is_string());
        let flame = invoke(
            &a,
            "profiling_flame",
            json!({ "request": { "profileId": tid, "widthPx": 1200, "search": "Serialize", "view": { "justMyCode": { "enabled": true } } } }),
        )
        .unwrap();
        assert!(flame["matchedPct"].as_f64().unwrap() > 20.0);
        let diff = invoke(
            &a,
            "profiling_function_diff",
            json!({ "request": { "baselineId": bid, "targetId": tid, "sort": "deltaSelf" } }),
        )
        .unwrap();
        assert!(diff["rows"][0]["deltaSelfPct"].as_f64().unwrap() > 5.0);
        let df = invoke(
            &a,
            "profiling_diff_flame",
            json!({ "request": { "baselineId": bid, "targetId": tid, "widthPx": 800 } }),
        )
        .unwrap();
        assert!(df["rects"][0]["delta"].is_number());
        let findings = invoke(
            &a,
            "profiling_findings",
            json!({ "request": { "profileId": tid, "baselineId": bid } }),
        )
        .unwrap();
        assert!(findings.as_array().unwrap().iter().any(|f| f["kind"] == "regression"), "{findings}");
        // Folded stacks have no timestamps: a clear error, not a crash.
        let e = invoke(&a, "profiling_threads", json!({ "request": { "profileId": tid } })).unwrap_err();
        assert!(e["message"].as_str().unwrap().contains("no timestamps"), "{e}");
        assert_eq!(e["code"], "unsupported");
        // The hottest path to a function, as a tree path / flame focus.
        let paths = invoke(
            &a,
            "profiling_frame_paths",
            json!({ "request": { "profileId": tid, "frame": frame, "limit": 2 } }),
        )
        .unwrap();
        let first = &paths["paths"][0];
        assert_eq!(first["path"].as_array().unwrap().last().unwrap(), &frame);
        assert_eq!(first["frames"].as_array().unwrap().len(), first["path"].as_array().unwrap().len());
        assert!(paths["totalPaths"].as_u64().unwrap() >= 1);
        let e = invoke(&a, "profiling_frame_paths", json!({ "request": { "profileId": 999, "frame": 0 } })).unwrap_err();
        assert_eq!(e["code"], "notFound");
        // Sorting by the average per call needs call counts, which folded stacks lack.
        let e = invoke(
            &a,
            "profiling_hotspots",
            json!({ "request": { "profileId": tid, "sort": { "metric": "samples", "of": "avgPerCall" } } }),
        )
        .unwrap_err();
        assert_eq!(e["code"], "unsupported");
        // Flame rectangles link to their parents; zoom breadcrumbs come with the graph.
        let zoomed = invoke(
            &a,
            "profiling_flame",
            json!({ "request": { "profileId": tid, "widthPx": 800, "focus": first["path"] } }),
        )
        .unwrap();
        assert_eq!(
            zoomed["focusFrames"].as_array().unwrap().len(),
            first["path"].as_array().unwrap().len()
        );
        assert!(zoomed["rects"][0]["parent"].is_null());
        assert!(invoke(&a, "profiling_spans", json!({})).unwrap().as_array().unwrap().is_empty());
        assert!(invoke(&a, "profiling_moment", json!({ "request": { "fromMs": 0, "toMs": 1 } }))
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(invoke(&a, "profiling_close", json!({ "id": tid })).unwrap(), true);
        assert_eq!(invoke(&a, "profiling_close", json!({ "id": tid })).unwrap(), false);
        // Invalid IPC arguments are rejected in Rust.
        assert!(invoke(&a, "profiling_hotspots", json!({ "request": { "profileId": "x" } })).is_err());
        assert!(invoke(&a, "profiling_hotspots", json!({ "request": { "profileId": bid, "limit": 0 } })).is_err());
    }

    #[test]
    fn heap_snapshots_timed_profiles_and_bad_paths() {
        let app = test_app_with_commands(None);
        let w = window(&app, "main");
        let ids: Vec<Value> = ["heap-1.heapsnapshot", "heap-2.heapsnapshot", "heap-3.heapsnapshot"]
            .iter()
            .map(|n| invoke(&w, "profiling_open", json!({ "path": sample(n) })).unwrap()["id"].clone())
            .collect();
        let types = invoke(
            &w,
            "profiling_heap_types",
            json!({ "request": { "snapshotId": ids[2], "sort": "retained" } }),
        )
        .unwrap();
        assert!(types["hasGraph"].as_bool().unwrap());
        let diff = invoke(
            &w,
            "profiling_heap_diff",
            json!({ "request": { "baselineId": ids[0], "targetId": ids[2] } }),
        )
        .unwrap();
        assert!(diff["rows"][0]["bytesDelta"].as_i64().unwrap() > 0);
        let growth = invoke(&w, "profiling_heap_growth", json!({ "request": { "snapshotIds": ids } })).unwrap();
        assert!(growth["steadyCount"].as_u64().unwrap() > 0);
        let dom = invoke(&w, "profiling_heap_dominators", json!({ "request": { "snapshotId": ids[2] } })).unwrap();
        let obj = dom["rows"][0]["object"].clone();
        let ty = dom["rows"][0]["typeName"].clone();
        invoke(
            &w,
            "profiling_heap_paths",
            json!({ "request": { "snapshotId": ids[2], "object": obj } }),
        )
        .unwrap();
        let inst = invoke(
            &w,
            "profiling_heap_instances",
            json!({ "request": { "snapshotId": ids[2], "typeName": ty } }),
        )
        .unwrap();
        assert!(inst["totalRows"].as_u64().unwrap() >= 1);
        let findings = invoke(
            &w,
            "profiling_heap_findings",
            json!({ "request": { "snapshotId": ids[2], "baselineId": ids[0] } }),
        )
        .unwrap();
        assert!(!findings.as_array().unwrap().is_empty(), "{findings}");

        let net = invoke(&w, "profiling_open", json!({ "path": sample("orders.nettrace") })).unwrap();
        assert!(net["hasTime"].as_bool().unwrap());
        let started = net["startedMs"].as_i64().unwrap();
        let threads = invoke(
            &w,
            "profiling_threads",
            json!({ "request": { "profileId": net["id"], "buckets": 50 } }),
        )
        .unwrap();
        assert!(!threads["threads"].as_array().unwrap().is_empty());
        let spans = invoke(&w, "profiling_spans", json!({})).unwrap();
        assert!(spans.as_array().unwrap().iter().any(|s| s["id"] == net["id"]));
        let m = invoke(
            &w,
            "profiling_moment",
            json!({ "request": { "fromMs": started, "toMs": started + 1000 } }),
        )
        .unwrap();
        assert!(m[0]["samples"].as_u64().unwrap() > 0);

        let dir = TempDir::new("profiling");
        crate::paths::allow_under(&w, "main", &dir.0);
        let e = invoke(&w, "profiling_open", json!({ "path": dir.0.to_string_lossy() })).unwrap_err();
        assert!(e["message"].as_str().unwrap().contains("not a file"), "{e}");
        assert_eq!(e["code"], "invalid");
        let missing = dir.0.join("missing.cpuprofile");
        let e = invoke(&w, "profiling_open", json!({ "path": missing.to_string_lossy() })).unwrap_err();
        assert_eq!(e["code"], "notFound", "{e}");
        assert!(e["message"].as_str().unwrap().contains("missing.cpuprofile"), "{e}");
        let junk = dir.0.join("junk.bin");
        std::fs::write(&junk, b"definitely not a profile").unwrap();
        let e = invoke(&w, "profiling_open", json!({ "path": junk.to_string_lossy() })).unwrap_err();
        assert!(e["message"].as_str().unwrap().contains("not a recognised profile"), "{e}");
        assert_eq!(e["code"], "notACapture");
    }

    #[test]
    fn sniff_routes_captures_heaps_and_everything_else() {
        let app = test_app_with_commands(None);
        let w = window(&app, "main");
        let dir = TempDir::new("profiling-sniff");
        let log = dir.0.join("service.log");
        std::fs::write(&log, "2026-10-14 12:00:00.000 INFO started\n2026-10-14 12:00:01.000 WARN slow\n").unwrap();
        let paths = [
            sample("orders.nettrace"),
            sample("heap-1.heapsnapshot"),
            sample("orders.folded"),
            log.to_string_lossy().into_owned(),
            dir.0.to_string_lossy().into_owned(),
            dir.0.join("missing.cpuprofile").to_string_lossy().into_owned(),
        ];
        let r = invoke(&w, "profiling_sniff", json!({ "paths": paths })).unwrap();
        let kinds: Vec<&str> = r.as_array().unwrap().iter().map(|x| x["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["capture", "heap", "capture", "none", "none", "none"]);
        assert_eq!(r[0]["path"], paths[0], "results carry the path as given");
        assert!(
            invoke(&w, "profiling_list", json!({})).unwrap().as_array().unwrap().is_empty(),
            "sniffing opens nothing"
        );
        let too_many = vec!["x"; MAX_SNIFF_PATHS + 1];
        let e = invoke(&w, "profiling_sniff", json!({ "paths": too_many })).unwrap_err();
        assert_eq!(e["code"], "limit");
    }

    /// Imports run under their own (long) budget; a hung importer is abandoned with a message.
    #[test]
    fn imports_have_their_own_time_budget() {
        struct Slow;
        impl CaptureImporter for Slow {
            fn id(&self) -> &'static str {
                "slow"
            }
            fn name(&self) -> &'static str {
                "Slow"
            }
            fn sniff(&self, _: &str, _: &[u8]) -> u8 {
                100
            }
            fn import(&self, f: &str, b: &[u8], l: &Limits) -> imt_profile::Result<Capture> {
                std::thread::sleep(Duration::from_millis(300));
                imt_plugin_profiling::folded::FoldedImporter.import(f, b, l)
            }
        }
        struct SlowPlugin;
        impl imt_core::Plugin for SlowPlugin {
            fn id(&self) -> &'static str {
                "test.slow"
            }
            fn name(&self) -> &'static str {
                "Slow"
            }
            fn version(&self) -> &'static str {
                "1.0.0"
            }
            fn register(&self, r: &mut imt_core::PluginRegistry) -> anyhow::Result<()> {
                r.declare_extension_point::<dyn CaptureImporter>(imt_profile::IMPORTERS_POINT)?;
                r.contribute::<dyn CaptureImporter>(imt_profile::IMPORTERS_POINT, Arc::new(Slow));
                Ok(())
            }
        }
        assert!(
            IMPORT_BUDGET >= Duration::from_secs(300),
            "imports get minutes, not the service budget"
        );
        let mut registry = imt_core::PluginRegistry::new();
        registry.install(&SlowPlugin).unwrap();
        registry.resolve();
        let service = registry
            .contributions::<dyn CaptureImporter>(imt_profile::IMPORTERS_POINT)
            .pop()
            .unwrap();
        let folded = std::fs::read(sample("orders.folded")).unwrap();
        let patient = GuardedImporter::new(service.clone()).unwrap();
        assert_eq!(patient.produces(), CaptureKind::Profile);
        assert!(patient.import("orders.folded", &folded, &Limits::default()).is_ok());
        let hasty = GuardedImporter::within(service, Duration::from_millis(20)).unwrap();
        let e = hasty.import("orders.folded", &folded, &Limits::default()).unwrap_err();
        assert!(e.to_string().contains("did not finish within"), "{e}");
        assert_eq!(e.code(), ErrorCode::Invalid);
    }

    /// A crashing importer contributed by a plugin is contained: the open fails with its id, IMT goes on.
    #[test]
    fn crashing_importers_are_contained() {
        struct Boom;
        impl CaptureImporter for Boom {
            fn id(&self) -> &'static str {
                "boom"
            }
            fn name(&self) -> &'static str {
                "Boom"
            }
            fn sniff(&self, _: &str, head: &[u8]) -> u8 {
                if head.starts_with(b"BOOM") {
                    100
                } else {
                    0
                }
            }
            fn import(&self, _: &str, _: &[u8], _: &Limits) -> imt_profile::Result<Capture> {
                panic!("importer bug")
            }
        }
        struct BoomPlugin;
        impl imt_core::Plugin for BoomPlugin {
            fn id(&self) -> &'static str {
                "test.boom"
            }
            fn name(&self) -> &'static str {
                "Boom"
            }
            fn version(&self) -> &'static str {
                "1.0.0"
            }
            fn register(&self, r: &mut imt_core::PluginRegistry) -> anyhow::Result<()> {
                r.contribute::<dyn CaptureImporter>(imt_profile::IMPORTERS_POINT, Arc::new(Boom));
                Ok(())
            }
        }
        /// The built-in importers contributed in process (in IMT they come from the profiling plugin's process).
        struct Builtin;
        impl imt_core::Plugin for Builtin {
            fn id(&self) -> &str {
                "test.profiling"
            }
            fn name(&self) -> &str {
                "Profiling"
            }
            fn version(&self) -> &str {
                "1.0.0"
            }
            fn register(&self, r: &mut imt_core::PluginRegistry) -> anyhow::Result<()> {
                r.declare_extension_point::<dyn CaptureImporter>(imt_profile::IMPORTERS_POINT)?;
                for i in imt_plugin_profiling::builtin_importers() {
                    r.contribute::<dyn CaptureImporter>(imt_profile::IMPORTERS_POINT, i);
                }
                Ok(())
            }
        }
        let mut registry = imt_core::PluginRegistry::new();
        registry.install(&Builtin).unwrap();
        registry.install(&BoomPlugin).unwrap();
        registry.resolve();
        let slot = ProfilingSlot::from_registry(&registry);
        let mut store = slot.store.write().unwrap();
        let e = store.open("x.boom", None, b"BOOM!").unwrap_err();
        assert!(e.to_string().contains("importer \"boom\" failed"), "{e}");
        // The built-in importers still work.
        let folded = std::fs::read(sample("orders.folded")).unwrap();
        assert!(store.open("orders.folded", None, &folded).is_ok());
        assert!(ProfilingSlot::default().store.read().unwrap().list().is_empty());
    }
}
