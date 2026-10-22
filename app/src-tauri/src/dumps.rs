//! Dump commands (per window, ADR 0023): open crash/memory dumps and UMDH heap logs as documents and query their
//! views. The engine is `imt_dump`; each window has its own [`DumpsSlot`]. The readers are the dumps plugin's
//! importers (`imt_dump::IMPORTERS_POINT` of the window's registry), called through their guarded services, so a
//! crashing or hanging reader is contained and counted against its plugin. The file is lent by ranges: a full dump
//! of many gigabytes is never read whole.
//!
//! Every command rejects with the one [`crate::CmdError`]. Wire contract: `imt_dump::api`, generated into
//! `app/src/app/core/generated/dumps.ts`.

use crate::paths::WindowGrants;
use crate::windows::CurrentWindow;
use crate::{CmdError, CmdResult, ErrorCode};
use imt_core::plugin::services::Service;
use imt_dump::api::*;
use imt_dump::import::SNIFF_BYTES;
use imt_dump::{DumpDocs, DumpError, DumpImporter, ImportOptions, Limits, ReadAt};
use std::io::Read;
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// Time a reader may take for one file (a full dump with many threads, a large UMDH log).
pub(crate) const IMPORT_BUDGET: Duration = Duration::from_secs(600);
/// Paths one `dumps_sniff` call accepts (one drop).
pub(crate) const MAX_SNIFF_PATHS: usize = 1_000;
/// Documents one findings or explain request may name.
const MAX_IDS: usize = 32;

/// One window's dump documents.
pub(crate) struct DumpsSlot {
    docs: Arc<RwLock<DumpDocs>>,
}

impl Default for DumpsSlot {
    fn default() -> Self {
        DumpsSlot::new(Vec::new())
    }
}

impl DumpsSlot {
    pub(crate) fn new(importers: Vec<Arc<dyn DumpImporter>>) -> DumpsSlot {
        DumpsSlot {
            docs: Arc::new(RwLock::new(DumpDocs::new(importers, Limits::default()))),
        }
    }

    /// A slot using the importers plugins contributed to `dumps.importers`.
    pub(crate) fn from_registry(registry: &imt_core::PluginRegistry) -> DumpsSlot {
        let importers = registry
            .contributions::<dyn DumpImporter>(imt_dump::IMPORTERS_POINT)
            .into_iter()
            .filter_map(|s| GuardedImporter::within(s, IMPORT_BUDGET))
            .map(|g| Arc::new(g) as Arc<dyn DumpImporter>)
            .collect();
        DumpsSlot::new(importers)
    }
}

/// An importer contribution called under its plugin's guard: a panic, a hang or a quarantined plugin becomes an
/// error, never a crash.
struct GuardedImporter {
    id: String,
    produces: imt_dump::Produces,
    service: Service<dyn DumpImporter>,
    budget: Duration,
}

impl GuardedImporter {
    fn within(service: Service<dyn DumpImporter>, budget: Duration) -> Option<GuardedImporter> {
        let (id, produces) = service.call(|i| (i.id().to_string(), i.produces())).ok()?;
        Some(GuardedImporter {
            id,
            produces,
            service,
            budget,
        })
    }
}

impl DumpImporter for GuardedImporter {
    fn id(&self) -> &str {
        &self.id
    }

    fn produces(&self) -> imt_dump::Produces {
        self.produces
    }

    fn sniff(&self, file_name: &str, head: &[u8]) -> u8 {
        self.service.call(|i| i.sniff(file_name, head)).unwrap_or(0)
    }

    /// Runs on a helper thread under the import budget: past it the import is abandoned and the plugin marked
    /// hung.
    fn import(&self, file_name: &str, file: Arc<dyn ReadAt>, options: &ImportOptions) -> imt_dump::Result<imt_dump::Document> {
        let (name, options) = (file_name.to_string(), options.clone());
        match self
            .service
            .call_detached_within(self.budget, move |i| i.import(&name, file, &options))
        {
            Ok(result) => result,
            Err(e) => Err(DumpError::invalid(format!("the {} reader failed: {e:#}", self.id))),
        }
    }
}

/// A file read by ranges for the reader (positional reads; the file stays open for the import).
struct FileAt {
    file: std::fs::File,
    len: u64,
}

impl ReadAt for FileAt {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
        let want = usize::try_from(self.len.saturating_sub(offset)).unwrap_or(usize::MAX).min(len);
        let mut buf = vec![0u8; want];
        let mut done = 0;
        while done < want {
            #[cfg(windows)]
            let n = std::os::windows::fs::FileExt::seek_read(&self.file, &mut buf[done..], offset + done as u64)?;
            #[cfg(unix)]
            let n = std::os::unix::fs::FileExt::read_at(&self.file, &mut buf[done..], offset + done as u64)?;
            if n == 0 {
                break;
            }
            done += n;
        }
        buf.truncate(done);
        Ok(buf)
    }
}

/// An engine error as the command error.
pub(crate) fn cmd_error(e: DumpError) -> CmdError {
    let code = match e.code {
        imt_dump::ErrorCode::NotADump => ErrorCode::Unsupported,
        imt_dump::ErrorCode::NotFound => ErrorCode::NotFound,
        imt_dump::ErrorCode::Io => ErrorCode::Io,
        imt_dump::ErrorCode::Limit => ErrorCode::Limit,
        imt_dump::ErrorCode::Unsupported => ErrorCode::Unsupported,
        imt_dump::ErrorCode::Invalid => ErrorCode::Invalid,
    };
    CmdError::new(code, e.message)
}

fn join_error(e: impl std::fmt::Display) -> CmdError {
    CmdError::new(ErrorCode::Internal, format!("the dump engine failed: {e}"))
}

async fn read<T: Send + 'static>(win: &CurrentWindow, f: impl FnOnce(&DumpDocs) -> imt_dump::Result<T> + Send + 'static) -> CmdResult<T> {
    let docs = win.dumps.docs.clone();
    tauri::async_runtime::spawn_blocking(move || f(&crate::sync::read(&docs)).map_err(cmd_error))
        .await
        .map_err(join_error)?
}

/// I/O errors about `path`: `notFound` when it does not exist, `io` otherwise.
fn io_error(what: &str, path: &Path, e: &std::io::Error) -> CmdError {
    let code = if e.kind() == std::io::ErrorKind::NotFound {
        ErrorCode::NotFound
    } else {
        ErrorCode::Io
    };
    CmdError::new(code, format!("cannot {what} {}: {e}", path.display()))
}

/// The start of a file the user chose, for sniffing; `None` when it is not a readable granted file.
fn read_head(grants: &WindowGrants, path: &str) -> Option<(String, Vec<u8>)> {
    let g = grants.read_file(path, &[]).ok()?;
    let mut head = Vec::with_capacity(SNIFF_BYTES);
    std::fs::File::open(g.path())
        .ok()?
        .take(SNIFF_BYTES as u64)
        .read_to_end(&mut head)
        .ok()?;
    let name = g.path().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Some((name, head))
}

/// What each path is (dump, UMDH log or neither), from its first 64 KiB, without opening it: dropped files are
/// routed with it. Folders, unreadable paths and paths the user did not choose are `none`.
#[tauri::command]
pub async fn dumps_sniff(state: CurrentWindow, grants: WindowGrants, paths: Vec<String>) -> CmdResult<Vec<DumpSniffResult>> {
    if paths.len() > MAX_SNIFF_PATHS {
        return Err(CmdError::new(
            ErrorCode::Limit,
            format!("at most {MAX_SNIFF_PATHS} files can be checked at once"),
        ));
    }
    let heads = tauri::async_runtime::spawn_blocking(move || paths.into_iter().map(|p| (read_head(&grants, &p), p)).collect::<Vec<_>>())
        .await
        .map_err(join_error)?;
    read(&state, move |d| {
        Ok(heads
            .into_iter()
            .map(|(head, path)| DumpSniffResult {
                kind: head.map_or(DumpSniffKind::None, |(name, head)| d.sniff(&name, &head)),
                path,
            })
            .collect())
    })
    .await
}

/// Opens a dump or UMDH log the user chose (recognised by content). The reader runs without holding the
/// window's documents; the document is added when it is done. Reopening a path replaces its document.
#[tauri::command]
pub async fn dumps_open<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: CurrentWindow,
    grants: WindowGrants,
    path: String,
) -> CmdResult<DumpDocSummary> {
    let docs = state.dumps.docs.clone();
    let symbols = crate::symbols::of(&app);
    tauri::async_runtime::spawn_blocking(move || {
        let granted = grants.read_file(&path, &[])?;
        let canonical = granted.path().to_path_buf();
        let file = std::fs::File::open(&canonical).map_err(|e| io_error("open", &canonical, &e))?;
        let meta = file.metadata().map_err(|e| io_error("read", &canonical, &e))?;
        let name = canonical.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let source = Arc::new(FileAt { file, len: meta.len() });
        let head = source.read_at(0, SNIFF_BYTES).map_err(|e| io_error("read", &canonical, &e))?;
        let (importer, limits) = {
            let d = crate::sync::read(&docs);
            if !d.has_importers() {
                return Err(CmdError::new(
                    ErrorCode::Unavailable,
                    "The dumps plugin is not available (safe mode, disabled or quarantined): dumps and UMDH logs cannot be opened.",
                ));
            }
            (d.pick(&name, &head), *d.limits())
        };
        let importer = importer.ok_or_else(|| {
            CmdError::new(
                ErrorCode::Unsupported,
                format!("{} is neither a dump nor a UMDH log IMT can read", canonical.display()),
            )
        })?;
        let options = ImportOptions {
            limits,
            file_time_ms: meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64),
            // IMT's symbol cache: PDBs the user's symbol servers gave earlier are found by identity.
            symbol_folders: symbols
                .cache_dir()
                .map(|d| vec![d.to_string_lossy().into_owned()])
                .unwrap_or_default(),
        };
        let doc = importer.import(&name, source.clone(), &options).map_err(cmd_error)?;
        // The missing PDBs are asked of the user's symbol servers (when they turned them on), then the dump is read again.
        let doc = symbols.complete(doc, limits.max_text, || importer.import(&name, source, &options));
        let path = canonical.to_string_lossy().into_owned();
        crate::sync::write(&docs).insert(&name, Some(&path), doc).map_err(cmd_error)
    })
    .await
    .map_err(join_error)?
}

#[tauri::command]
pub async fn dumps_list(state: CurrentWindow) -> CmdResult<Vec<DumpDocSummary>> {
    read(&state, |d| Ok(d.list())).await
}

/// Closes a document; `false` when it was not open.
#[tauri::command]
pub async fn dumps_close(state: CurrentWindow, id: u32) -> CmdResult<bool> {
    let docs = state.dumps.docs.clone();
    tauri::async_runtime::spawn_blocking(move || crate::sync::write(&docs).close(id))
        .await
        .map_err(join_error)
}

#[tauri::command]
pub async fn dumps_overview(state: CurrentWindow, id: u32) -> CmdResult<DumpOverview> {
    read(&state, move |d| d.overview(id)).await
}

#[tauri::command]
pub async fn dumps_threads(state: CurrentWindow, request: ThreadsRequest) -> CmdResult<ThreadsPage> {
    read(&state, move |d| d.threads(&request)).await
}

#[tauri::command]
pub async fn dumps_stack(state: CurrentWindow, request: StackRequest) -> CmdResult<StackDto> {
    read(&state, move |d| d.stack(&request)).await
}

#[tauri::command]
pub async fn dumps_modules(state: CurrentWindow, request: ModulesRequest) -> CmdResult<ModulesPage> {
    read(&state, move |d| d.modules(&request)).await
}

fn check_ids(ids: &[u32]) -> CmdResult<()> {
    if ids.len() > MAX_IDS {
        return Err(CmdError::new(ErrorCode::Limit, format!("at most {MAX_IDS} documents at once")));
    }
    Ok(())
}

/// Findings of one dump, or of UMDH logs of one process (oldest first).
#[tauri::command]
pub async fn dumps_findings(state: CurrentWindow, request: DumpFindingsRequest) -> CmdResult<Vec<DumpFinding>> {
    check_ids(&request.ids)?;
    read(&state, move |d| d.findings(&request)).await
}

#[tauri::command]
pub async fn dumps_umdh_diff(state: CurrentWindow, request: UmdhDiffRequest) -> CmdResult<UmdhDiffPage> {
    read(&state, move |d| d.umdh_diff(&request)).await
}

#[tauri::command]
pub async fn dumps_umdh_stack(state: CurrentWindow, request: UmdhStackRequest) -> CmdResult<UmdhStackDto> {
    read(&state, move |d| d.umdh_stack(&request)).await
}

#[tauri::command]
pub async fn dumps_umdh_tree(state: CurrentWindow, request: UmdhTreeRequest) -> CmdResult<Vec<UmdhTreeNode>> {
    read(&state, move |d| d.umdh_tree(&request)).await
}

/// The open documents with a time, as markers for the log timeline.
#[tauri::command]
pub async fn dumps_markers(state: CurrentWindow) -> CmdResult<Vec<DumpMarker>> {
    read(&state, |d| Ok(d.markers())).await
}

/// The compact, redacted question "Explain with AI" sends to the chat about these documents.
#[tauri::command]
pub async fn dumps_explain(state: CurrentWindow, request: DumpFindingsRequest) -> CmdResult<String> {
    check_ids(&request.ids)?;
    read(&state, move |d| d.explain(&request)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::samples;
    use imt_dump::Produces;

    /// The plugin's readers in process (in IMT they run in the dumps plugin's process).
    struct Local(Produces);

    struct Src(Arc<dyn ReadAt>);

    impl imt_plugin_dumps::source::Source for Src {
        fn len(&self) -> u64 {
            self.0.len()
        }

        fn read(&self, offset: u64, len: usize) -> Result<Vec<u8>, String> {
            self.0.read_at(offset, len).map_err(|e| e.to_string())
        }
    }

    impl DumpImporter for Local {
        fn id(&self) -> &str {
            match self.0 {
                Produces::Dump => "minidump",
                Produces::Umdh => "umdh",
            }
        }
        fn produces(&self) -> Produces {
            self.0
        }
        fn sniff(&self, _: &str, head: &[u8]) -> u8 {
            match self.0 {
                Produces::Dump => {
                    if head.starts_with(b"MDMP") {
                        100
                    } else {
                        0
                    }
                }
                Produces::Umdh => imt_plugin_dumps::umdh_reader::sniff(head),
            }
        }
        fn import(&self, _: &str, file: Arc<dyn ReadAt>, options: &ImportOptions) -> imt_dump::Result<imt_dump::Document> {
            let src = Src(file);
            match self.0 {
                Produces::Dump => imt_plugin_dumps::dump_reader::read_dump(&src, &options.limits).map(imt_dump::Document::Dump),
                Produces::Umdh => {
                    imt_plugin_dumps::umdh_reader::read_umdh(&src, &options.limits, options.file_time_ms).map(imt_dump::Document::Umdh)
                }
            }
        }
    }

    pub(crate) fn local() -> Vec<Arc<dyn DumpImporter>> {
        vec![Arc::new(Local(Produces::Dump)), Arc::new(Local(Produces::Umdh))]
    }

    #[test]
    fn files_are_read_by_ranges() {
        let dir = tempfile_dir();
        let p = dir.join("x.bin");
        std::fs::write(&p, b"0123456789").unwrap();
        let f = FileAt {
            file: std::fs::File::open(&p).unwrap(),
            len: 10,
        };
        assert_eq!(f.read_at(8, 100).unwrap(), b"89");
        assert!(f.read_at(20, 4).unwrap().is_empty());
        assert_eq!(ReadAt::len(&f), 10);
        let _ = std::fs::remove_dir_all(dir);
    }

    fn tempfile_dir() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("imt-dumps-test-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn engine_errors_keep_their_meaning() {
        let codes = [
            (imt_dump::ErrorCode::NotADump, ErrorCode::Unsupported),
            (imt_dump::ErrorCode::NotFound, ErrorCode::NotFound),
            (imt_dump::ErrorCode::Io, ErrorCode::Io),
            (imt_dump::ErrorCode::Limit, ErrorCode::Limit),
            (imt_dump::ErrorCode::Unsupported, ErrorCode::Unsupported),
            (imt_dump::ErrorCode::Invalid, ErrorCode::Invalid),
        ];
        for (from, to) in codes {
            let e = cmd_error(DumpError::new(from, "m"));
            assert_eq!((e.code, e.message.as_str()), (to, "m"));
        }
        assert_eq!(join_error("boom").code, ErrorCode::Internal);
        assert!(check_ids(&[0; MAX_IDS + 1]).is_err() && check_ids(&[1]).is_ok());
        let missing = io_error("open", Path::new("/x"), &std::io::Error::from(std::io::ErrorKind::NotFound));
        assert_eq!(missing.code, ErrorCode::NotFound);
    }

    /// The in-process readers and one that panics, as plugins (so their calls are guarded).
    struct TestPlugin(bool);

    impl imt_core::Plugin for TestPlugin {
        fn id(&self) -> &str {
            if self.0 {
                "test.boom"
            } else {
                "test.dumps"
            }
        }
        fn name(&self) -> &str {
            "Dumps"
        }
        fn version(&self) -> &str {
            "1.0.0"
        }
        fn register(&self, r: &mut imt_core::PluginRegistry) -> anyhow::Result<()> {
            let _ = r.declare_extension_point::<dyn DumpImporter>(imt_dump::IMPORTERS_POINT);
            if self.0 {
                r.contribute::<dyn DumpImporter>(imt_dump::IMPORTERS_POINT, Arc::new(Panics));
            } else {
                for i in local() {
                    r.contribute::<dyn DumpImporter>(imt_dump::IMPORTERS_POINT, i);
                }
            }
            Ok(())
        }
    }

    struct Panics;

    impl DumpImporter for Panics {
        fn id(&self) -> &str {
            "boom"
        }
        fn produces(&self) -> Produces {
            Produces::Dump
        }
        fn sniff(&self, _: &str, head: &[u8]) -> u8 {
            if head.starts_with(b"BOOM") {
                100
            } else {
                0
            }
        }
        fn import(&self, _: &str, _: Arc<dyn ReadAt>, _: &ImportOptions) -> imt_dump::Result<imt_dump::Document> {
            panic!("reader bug")
        }
    }

    fn opts() -> ImportOptions {
        ImportOptions {
            limits: Limits::default(),
            file_time_ms: None,
            symbol_folders: Vec::new(),
        }
    }

    /// A crashing reader contributed by a plugin is contained: the open fails with its id, the others work.
    #[test]
    fn crashing_readers_are_contained() {
        let mut registry = imt_core::PluginRegistry::new();
        registry.install(&TestPlugin(false)).unwrap();
        registry.install(&TestPlugin(true)).unwrap();
        registry.resolve();
        let slot = DumpsSlot::from_registry(&registry);
        let docs = crate::sync::read(&slot.docs);
        let boom = docs.pick("x", b"BOOM").unwrap();
        let e = boom.import("x", Arc::new(b"BOOM".to_vec()), &opts()).unwrap_err();
        assert!(e.message.contains("boom"), "{e}");
        let bytes = std::fs::read(samples().join("windows/WEB02/OrderService.exe.4120.dmp")).unwrap();
        let reader = docs.pick("x.dmp", &bytes).unwrap();
        assert!(reader.import("x.dmp", Arc::new(bytes), &opts()).is_ok());
        assert!(!DumpsSlot::default().docs.read().unwrap().has_importers());
    }

    fn ipc(
        w: &tauri::WebviewWindow<tauri::test::MockRuntime>,
        cmd: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, serde_json::Value> {
        crate::tools::tests::ipc(w, cmd, body)
    }

    /// The commands over the real IPC path, with the real dumps plugin process when the plugins binary is built
    /// (the workspace's tests build it); without it, opening says the plugin is unavailable.
    #[test]
    fn commands_open_query_and_close_per_window() {
        use serde_json::json;
        let app = crate::test_support::test_app_with_commands(None);
        let a = crate::test_support::window(&app, "main");
        let b = crate::test_support::window(&app, "w-2");
        let dmp = samples()
            .join("windows/WEB02/OrderService.exe.4120.dmp")
            .to_string_lossy()
            .into_owned();
        let sniff = ipc(&a, "dumps_sniff", json!({ "paths": [dmp, "/definitely/missing.dmp"] })).unwrap();
        let opened = ipc(&a, "dumps_open", json!({ "path": dmp }));
        let Ok(doc) = opened else {
            let e = opened.unwrap_err();
            eprintln!("the plugins binary has no dumps plugin; only the unavailable path was tested");
            assert_eq!(e["code"], "unavailable", "{e}");
            assert_eq!(sniff[0]["kind"], "none");
            return;
        };
        assert_eq!(sniff[0]["kind"], "dump");
        assert_eq!(sniff[1]["kind"], "none");
        assert_eq!(doc["kind"], "dump");
        assert_eq!(doc["severity"], "critical");
        let id = doc["id"].clone();
        assert_eq!(ipc(&b, "dumps_list", json!({})).unwrap().as_array().unwrap().len(), 0, "per window");
        let o = ipc(&a, "dumps_overview", json!({ "id": id })).unwrap();
        assert_eq!(o["exception"]["name"], "CLR_EXCEPTION");
        let t = ipc(&a, "dumps_threads", json!({ "request": { "id": id, "offset": 0, "limit": 10 } })).unwrap();
        let tid = t["rows"][0]["id"].clone();
        assert_eq!(t["rows"][0]["state"], "faulted");
        let s = ipc(&a, "dumps_stack", json!({ "request": { "id": id, "threadId": tid } })).unwrap();
        assert!(!s["frames"].as_array().unwrap().is_empty());
        let m = ipc(&a, "dumps_modules", json!({ "request": { "id": id, "offset": 0, "limit": 100 } })).unwrap();
        assert!(m["total"].as_u64().unwrap() > 0);
        let f = ipc(&a, "dumps_findings", json!({ "request": { "ids": [id] } })).unwrap();
        assert_eq!(f[0]["kind"], "crashCause");
        let text = ipc(&a, "dumps_explain", json!({ "request": { "ids": [id] } })).unwrap();
        assert!(text.as_str().unwrap().contains("OrderService.exe.4120.dmp"));
        assert_eq!(ipc(&a, "dumps_markers", json!({})).unwrap().as_array().unwrap().len(), 1);

        let umdh = samples().join("dumps/umdh");
        let snap = |n: &str| umdh.join(n).to_string_lossy().into_owned();
        let s1 = ipc(&a, "dumps_open", json!({ "path": snap("OrderService-4120-snap-1030.txt") })).unwrap();
        let s3 = ipc(&a, "dumps_open", json!({ "path": snap("OrderService-4120-snap-1040.txt") })).unwrap();
        assert_eq!(s1["kind"], "umdhSnapshot");
        let page = ipc(
            &a,
            "dumps_umdh_diff",
            json!({ "request": { "base": s1["id"], "target": s3["id"], "group": "function", "offset": 0, "limit": 10 } }),
        )
        .unwrap();
        assert_eq!(page["rows"][0]["key"], "Shop.Orders!OrderCache::Add");
        let tree = ipc(
            &a,
            "dumps_umdh_tree",
            json!({ "request": { "base": s1["id"], "target": s3["id"], "path": [], "limit": 10 } }),
        )
        .unwrap();
        assert!(tree[0]["hasChildren"].as_bool().unwrap());
        let wrong = ipc(
            &a,
            "dumps_umdh_diff",
            json!({ "request": { "base": null, "target": id, "group": "stack", "offset": 0, "limit": 10 } }),
        )
        .unwrap_err();
        assert_eq!(wrong["code"], "unsupported");
        assert_eq!(ipc(&a, "dumps_close", json!({ "id": id })).unwrap(), json!(true));
        assert_eq!(ipc(&a, "dumps_overview", json!({ "id": id })).unwrap_err()["code"], "notFound");
        let not = ipc(&a, "dumps_open", json!({ "path": samples().join("README.md").to_string_lossy() })).unwrap_err();
        assert_eq!(not["code"], "unsupported");
    }
}
