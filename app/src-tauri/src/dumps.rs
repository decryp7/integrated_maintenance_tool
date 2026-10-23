//! Dump commands (per window, ADR 0023): open crash/memory dumps and UMDH heap logs as documents and query their
//! views. The engine is `debrief_dump`; each window has its own [`DumpsSlot`]. The readers are the dumps plugin's
//! importers (`debrief_dump::IMPORTERS_POINT` of the window's registry), called through their guarded services, so a
//! crashing or hanging reader is contained and counted against its plugin. The file is lent by ranges: a full dump
//! of many gigabytes is never read whole.
//!
//! Every command rejects with the one [`crate::CmdError`]. Wire contract: `debrief_dump::api`, generated into
//! `app/src/app/core/generated/dumps.ts`.

use crate::paths::WindowGrants;
use crate::symbols::{Download, Opened};
use crate::windows::CurrentWindow;
use crate::{CmdError, CmdResult, ErrorCode};
use debrief_core::plugin::services::Service;
use debrief_dump::api::*;
use debrief_dump::import::SNIFF_BYTES;
use debrief_dump::{DumpDocs, DumpError, DumpImporter, ImportOptions, Limits, ReadAt};
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
    pub(crate) fn from_registry(registry: &debrief_core::PluginRegistry) -> DumpsSlot {
        let importers = registry
            .contributions::<dyn DumpImporter>(debrief_dump::IMPORTERS_POINT)
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
    produces: debrief_dump::Produces,
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

    fn produces(&self) -> debrief_dump::Produces {
        self.produces
    }

    fn sniff(&self, file_name: &str, head: &[u8]) -> u8 {
        self.service.call(|i| i.sniff(file_name, head)).unwrap_or(0)
    }

    /// Runs on a helper thread under the import budget: past it the import is abandoned and the plugin marked
    /// hung.
    fn import(&self, file_name: &str, file: Arc<dyn ReadAt>, options: &ImportOptions) -> debrief_dump::Result<debrief_dump::Document> {
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
    /// Set by the open job's Cancel: the next read fails and the import stops.
    cancel: Arc<std::sync::atomic::AtomicBool>,
    /// The furthest byte read so far (the open job's byte progress).
    reached: Arc<std::sync::atomic::AtomicU64>,
}

impl ReadAt for FileAt {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
        if self.cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "cancelled"));
        }
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
        if done > 0 {
            self.reached.fetch_max(offset + done as u64, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(buf)
    }
}

/// An engine error as the command error.
pub(crate) fn cmd_error(e: DumpError) -> CmdError {
    let code = match e.code {
        debrief_dump::ErrorCode::NotADump => ErrorCode::Unsupported,
        debrief_dump::ErrorCode::NotFound => ErrorCode::NotFound,
        debrief_dump::ErrorCode::Io => ErrorCode::Io,
        debrief_dump::ErrorCode::Limit => ErrorCode::Limit,
        debrief_dump::ErrorCode::Unsupported => ErrorCode::Unsupported,
        debrief_dump::ErrorCode::Invalid => ErrorCode::Invalid,
    };
    CmdError::new(code, e.message)
}

fn join_error(e: impl std::fmt::Display) -> CmdError {
    CmdError::new(ErrorCode::Internal, format!("the dump engine failed: {e}"))
}

async fn read<T: Send + 'static>(
    win: &CurrentWindow,
    f: impl FnOnce(&DumpDocs) -> debrief_dump::Result<T> + Send + 'static,
) -> CmdResult<T> {
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
    let label = state.label().to_string();
    let symbols = crate::symbols::of(&app);
    // Opening is a job (a big dump or UMDH log takes a while): Jobs shows the phase and the bytes read, and Cancel
    // stops the reader at its next read. The grant is checked first, so a refused path never becomes a job.
    let granted = grants.read_file(&path, &[])?;
    let canonical = granted.path().to_path_buf();
    let title = canonical
        .file_name()
        .map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
    let spec = crate::jobs::JobSpec::new(crate::jobs::JobKind::Import, format!("Open {title}"))
        .window(state.label())
        .link(crate::jobs::JobLink::Path {
            path: canonical.display().to_string(),
        });
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reached = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let total_len = std::fs::metadata(&canonical).map_or(0, |m| m.len());
    let controls = crate::jobs::Controls::cancel({
        let c = cancel.clone();
        move || c.store(true, std::sync::atomic::Ordering::Relaxed)
    })
    .with_probe({
        let r = reached.clone();
        move || {
            Some(crate::jobs::JobProgress::in_phase(
                "Reading the file",
                crate::jobs::PhaseScale::Bytes,
                r.load(std::sync::atomic::Ordering::Relaxed),
                total_len,
            ))
        }
    });
    let app2 = app.clone();
    crate::jobs::run(
        &app2,
        spec,
        controls,
        move |ctx: &crate::jobs::JobCtx| -> CmdResult<crate::jobs::Done<DumpDocSummary>> {
            ctx.phase("Opening the file", crate::jobs::PhaseScale::Indeterminate, 0, 0);
            let file = std::fs::File::open(&canonical).map_err(|e| io_error("open", &canonical, &e))?;
            let meta = file.metadata().map_err(|e| io_error("read", &canonical, &e))?;
            let name = canonical.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let source = Arc::new(FileAt {
                file,
                len: meta.len(),
                cancel: cancel.clone(),
                reached,
            });
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
                    format!("{} is neither a dump nor a UMDH log Debrief can read", canonical.display()),
                )
            })?;
            let options = ImportOptions {
                limits,
                file_time_ms: meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as i64),
                // Debrief's symbol cache: PDBs the user's symbol servers gave earlier are found by identity.
                symbol_folders: symbols
                    .cache_dir()
                    .map(|d| vec![d.to_string_lossy().into_owned()])
                    .unwrap_or_default(),
            };
            let doc = match importer.import(&name, source.clone(), &options) {
                Ok(d) => d,
                Err(_) if cancel.load(std::sync::atomic::Ordering::Relaxed) => {
                    return Err(CmdError::new(ErrorCode::Cancelled, "cancelled"));
                }
                Err(e) => return Err(cmd_error(e)),
            };
            // The dump opens now with what is there. Its missing PDBs are asked of the user's symbol servers (when they
            // turned them on) by a job after it is open, which reads the dump again when they arrive.
            let (doc, download) = match symbols.begin(doc, limits.max_text) {
                Opened::Final(doc) => (doc, None),
                Opened::Downloading(doc, download) => (doc, Some(download)),
            };
            let path = canonical.to_string_lossy().into_owned();
            let summary = crate::sync::write(&docs).insert(&name, Some(&path), doc).map_err(cmd_error)?;
            if let Some(download) = download {
                let reimport = move || importer.import(&name, source, &options);
                let target = SymbolsTarget {
                    label,
                    docs,
                    id: summary.id,
                    title: summary.name.clone(),
                    path,
                    max_text: limits.max_text,
                };
                let _ = start_symbols_job(&app, target, download, reimport);
            }
            Ok(crate::jobs::Done::new(summary))
        },
    )
    .await
}

/// Event to the window when the symbols of one of its dumps changed (the PDBs arrived, or the download ended).
pub(crate) const SYMBOLS_EVENT: &str = "debrief://dump-symbols";

/// What `debrief://dump-symbols` carries: the document, which the view reads again.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SymbolsChanged {
    pub id: u32,
}

/// The dump a symbol download is for.
struct SymbolsTarget {
    label: String,
    docs: Arc<RwLock<DumpDocs>>,
    id: u32,
    title: String,
    path: String,
    max_text: usize,
}

/// Runs the download of a dump's missing PDBs as a Jobs job (Cancel is the job's), after the dump is open. The
/// document is updated when the PDBs arrive and the window is told; a document closed meanwhile is left alone. When
/// no job can be queued the modules go back to missing, so none stays "downloading".
fn start_symbols_job<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    target: SymbolsTarget,
    download: Download,
    reimport: impl FnOnce() -> debrief_dump::Result<debrief_dump::Document> + Send + 'static,
) -> Option<crate::jobs::JobId> {
    use crate::jobs::{self, Controls, Done, JobCtx, JobKind, JobLink, JobProgress, JobSpec};
    let spec = JobSpec::new(JobKind::Download, format!("Symbols for {}", target.title))
        .window(&target.label)
        .link(JobLink::Path { path: target.path.clone() });
    let (docs, id, label, max_text) = (target.docs.clone(), target.id, target.label.clone(), target.max_text);
    let handle = app.clone();
    let download = Arc::new(download);
    let job = download.clone();
    let queued = jobs::spawn(app, spec, Controls::default(), move |ctx: &JobCtx| -> Result<Done<()>, String> {
        let cancel = ctx.cancel_flag();
        let update = job.run(
            &cancel,
            &|done, total| ctx.progress(JobProgress::counts(done as u64, total as u64)),
            reimport,
        );
        let failure = update.failure();
        let changed = update.apply(&docs, id, max_text);
        if changed {
            crate::windows::emit_to_window(&handle, &label, SYMBOLS_EVENT, SymbolsChanged { id });
        }
        match failure {
            Some(why) => Err(format!("The symbol download failed: {why}")),
            None => Ok(Done::new(()).cancelled_if(cancel.load(std::sync::atomic::Ordering::Relaxed))),
        }
    });
    match queued {
        Ok(job) => Some(job),
        Err(why) => {
            log::warn!("The symbol download could not be queued: {why}");
            if download.abandoned().apply(&target.docs, target.id, target.max_text) {
                crate::windows::emit_to_window(app, &target.label, SYMBOLS_EVENT, SymbolsChanged { id: target.id });
            }
            None
        }
    }
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
    use debrief_dump::Produces;

    /// The plugin's readers in process (in Debrief they run in the dumps plugin's process).
    struct Local(Produces);

    struct Src(Arc<dyn ReadAt>);

    impl debrief_plugin_dumps::source::Source for Src {
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
                Produces::Umdh => debrief_plugin_dumps::umdh_reader::sniff(head),
            }
        }
        fn import(&self, _: &str, file: Arc<dyn ReadAt>, options: &ImportOptions) -> debrief_dump::Result<debrief_dump::Document> {
            let src = Src(file);
            match self.0 {
                Produces::Dump => debrief_plugin_dumps::dump_reader::read_dump(&src, &options.limits).map(debrief_dump::Document::Dump),
                Produces::Umdh => debrief_plugin_dumps::umdh_reader::read_umdh(&src, &options.limits, options.file_time_ms)
                    .map(debrief_dump::Document::Umdh),
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
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            reached: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        };
        assert_eq!(f.read_at(8, 100).unwrap(), b"89");
        assert!(f.read_at(20, 4).unwrap().is_empty());
        assert_eq!(ReadAt::len(&f), 10);
        assert_eq!(
            f.reached.load(std::sync::atomic::Ordering::Relaxed),
            10,
            "the job's byte progress follows the reads"
        );
        f.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(
            f.read_at(0, 2).unwrap_err().kind(),
            std::io::ErrorKind::Interrupted,
            "Cancel stops the reader"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    fn tempfile_dir() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("debrief-dumps-test-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn engine_errors_keep_their_meaning() {
        let codes = [
            (debrief_dump::ErrorCode::NotADump, ErrorCode::Unsupported),
            (debrief_dump::ErrorCode::NotFound, ErrorCode::NotFound),
            (debrief_dump::ErrorCode::Io, ErrorCode::Io),
            (debrief_dump::ErrorCode::Limit, ErrorCode::Limit),
            (debrief_dump::ErrorCode::Unsupported, ErrorCode::Unsupported),
            (debrief_dump::ErrorCode::Invalid, ErrorCode::Invalid),
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

    impl debrief_core::Plugin for TestPlugin {
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
        fn register(&self, r: &mut debrief_core::PluginRegistry) -> anyhow::Result<()> {
            let _ = r.declare_extension_point::<dyn DumpImporter>(debrief_dump::IMPORTERS_POINT);
            if self.0 {
                r.contribute::<dyn DumpImporter>(debrief_dump::IMPORTERS_POINT, Arc::new(Panics));
            } else {
                for i in local() {
                    r.contribute::<dyn DumpImporter>(debrief_dump::IMPORTERS_POINT, i);
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
        fn import(&self, _: &str, _: Arc<dyn ReadAt>, _: &ImportOptions) -> debrief_dump::Result<debrief_dump::Document> {
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
        let mut registry = debrief_core::PluginRegistry::new();
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

    /// A symbol server that fetches by a script: `ok` PDB names arrive, the others fail with `error`; the first
    /// request waits until `release` is set (so a test can cancel while it is running).
    struct Scripted {
        ok: Vec<String>,
        error: Option<debrief_symbols::fetch::FetchError>,
        started: std::sync::atomic::AtomicBool,
        release: Arc<std::sync::atomic::AtomicBool>,
        wait: bool,
    }

    impl debrief_symbols::fetch::Fetcher for Scripted {
        fn fetch(&self, key: &debrief_symbols::id::PdbKey, cache: &Path) -> Result<std::path::PathBuf, debrief_symbols::fetch::FetchError> {
            use std::sync::atomic::Ordering;
            if self.wait && !self.started.swap(true, Ordering::SeqCst) {
                let t = std::time::Instant::now();
                while !self.release.load(Ordering::SeqCst) && t.elapsed() < Duration::from_secs(20) {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            if self.ok.contains(&key.name().to_string()) {
                return Ok(cache.join("fetched.pdb"));
            }
            Err(self
                .error
                .clone()
                .unwrap_or_else(|| debrief_symbols::fetch::FetchError::NotFound(key.name().to_string())))
        }
    }

    fn scripted(ok: &[String], error: Option<debrief_symbols::fetch::FetchError>, wait: bool) -> Arc<Scripted> {
        Arc::new(Scripted {
            ok: ok.to_vec(),
            error,
            started: Default::default(),
            release: Default::default(),
            wait,
        })
    }

    fn module_statuses(docs: &RwLock<DumpDocs>, id: u32) -> Vec<(String, SymbolStatusDto)> {
        let page = crate::sync::read(docs)
            .modules(&ModulesRequest {
                id,
                offset: 0,
                limit: 100,
                filter: None,
            })
            .unwrap();
        page.rows.into_iter().filter_map(|r| r.debug_file.map(|f| (f, r.symbols))).collect()
    }

    /// Symbol downloads are Jobs jobs run after the dump is open: a late arrival updates the open dump and tells the
    /// window, Cancel stops asking for more, a download that could not reach the servers fails, and a dump closed
    /// meanwhile is left alone.
    #[test]
    fn symbol_downloads_are_jobs_that_update_the_open_dump() {
        use crate::test_support::{job_end, test_app};
        use debrief_dump::model::{Document, SymbolStatus};
        use std::sync::atomic::Ordering;
        use std::sync::Mutex;
        use tauri::{Listener, Manager};

        let app = test_app(None);
        let told: Arc<Mutex<Vec<serde_json::Value>>> = Arc::default();
        let t = told.clone();
        app.handle().listen_any(SYMBOLS_EVENT, move |e| {
            t.lock().unwrap().push(serde_json::from_str(e.payload()).unwrap());
        });
        let cache = crate::test_support::TempDir::new("symbols-job");
        let missing = || {
            let mut d = debrief_dump::testing::sample_dump();
            for m in &mut d.modules {
                m.symbols = SymbolStatus::Missing;
            }
            d
        };
        let names: Vec<String> = debrief_dump::symbols::missing(&missing(), 100)
            .iter()
            .map(|k| k.name().to_string())
            .collect();
        assert!(names.len() >= 3);
        // Opens the dump with its missing modules downloading; the job gets `fetcher`.
        let begin = |fetcher: Arc<Scripted>, reimport_loads: Option<String>| {
            let dump = missing();
            let download = Download::with_fetcher(&dump, &cache.0, fetcher);
            let mut marked = dump.clone();
            debrief_dump::symbols::mark_downloading(&mut marked, &debrief_dump::symbols::missing(&dump, 100));
            let docs = Arc::new(RwLock::new(DumpDocs::new(Vec::new(), Limits::default())));
            let id = crate::sync::write(&docs)
                .insert("a.dmp", Some("/x/a.dmp"), Document::Dump(marked))
                .unwrap()
                .id;
            let target = SymbolsTarget {
                label: "main".into(),
                docs: docs.clone(),
                id,
                title: "a.dmp".into(),
                path: "/x/a.dmp".into(),
                max_text: 120,
            };
            let reimport = move || {
                let mut d = missing();
                for m in d
                    .modules
                    .iter_mut()
                    .filter(|m| m.debug_file.as_deref() == reimport_loads.as_deref())
                {
                    m.symbols = SymbolStatus::Loaded;
                }
                Ok(Document::Dump(d))
            };
            let job = start_symbols_job(app.handle(), target, download, reimport).expect("queued");
            (docs, id, job)
        };
        let status_of = |rows: &[(String, SymbolStatusDto)], name: &str| rows.iter().find(|(f, _)| f == name).unwrap().1.clone();

        // A late arrival: the dump was open (downloading) before the PDB came, then it is read again and the window told.
        let (docs, id, job) = begin(scripted(&names[..1], None, false), Some(names[0].clone()));
        let end = job_end(&app, job.0);
        assert_eq!(end.state, crate::jobs::JobState::Done, "{end:?}");
        let rows = module_statuses(&docs, id);
        assert_eq!(status_of(&rows, &names[0]), SymbolStatusDto::Loaded);
        assert_eq!(status_of(&rows, &names[1]), SymbolStatusDto::NotOnServer);
        assert!(
            told.lock().unwrap().contains(&serde_json::json!({ "id": id })),
            "{:?}",
            told.lock().unwrap()
        );

        // Failure: no server could be reached, so the job fails with the reason and the modules say so.
        let down = Some(debrief_symbols::fetch::FetchError::Network("offline".into()));
        let (docs, id, job) = begin(scripted(&[], down, false), None);
        let end = job_end(&app, job.0);
        assert_eq!(end.state, crate::jobs::JobState::Failed, "{end:?}");
        assert!(end.error.unwrap_or_default().contains("offline"));
        assert!(matches!(
            status_of(&module_statuses(&docs, id), &names[0]),
            SymbolStatusDto::DownloadFailed { .. }
        ));

        // Cancel while the first PDB is being asked: no more are asked, the rest go back to missing.
        let fetcher = scripted(&[], None, true);
        let (docs, id, job) = begin(fetcher.clone(), None);
        let t = std::time::Instant::now();
        while !fetcher.started.load(Ordering::SeqCst) && t.elapsed() < Duration::from_secs(20) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(matches!(
            status_of(&module_statuses(&docs, id), &names[1]),
            SymbolStatusDto::Downloading
        ));
        app.state::<crate::jobs::Jobs>().cancel(job).unwrap();
        fetcher.release.store(true, Ordering::SeqCst);
        let end = job_end(&app, job.0);
        assert_eq!(end.state, crate::jobs::JobState::Cancelled, "{end:?}");
        let rows = module_statuses(&docs, id);
        assert!(rows.iter().all(|(_, s)| *s != SymbolStatusDto::Downloading), "{rows:?}");
        assert_eq!(status_of(&rows, &names[1]), SymbolStatusDto::Missing);

        // A dump closed before the PDBs came: nothing changes and nothing is announced.
        let before = told.lock().unwrap().len();
        let fetcher = scripted(&[], None, true);
        let (docs, id, job) = begin(fetcher.clone(), None);
        assert!(crate::sync::write(&docs).close(id));
        fetcher.release.store(true, Ordering::SeqCst);
        assert_eq!(job_end(&app, job.0).state, crate::jobs::JobState::Done);
        assert_eq!(told.lock().unwrap().len(), before);
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
