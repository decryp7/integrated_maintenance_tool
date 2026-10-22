//! What the host does for a plugin's elevated process besides running its operations (ADR 0017 §7, amendment
//! 2026-10-22): it saves the process' output streams into the data folder the host chose ([`save_output`]), and
//! logs the standard-error lines the process forwards ([`ForwardedLog`]).
//!
//! The elevated process never writes at a path: it names a folder and a file, both checked here; the bytes come
//! through [`PluginRequest::PullOutput`] with credits, each piece and the total bounded, into a hidden partial
//! file that becomes the file only when the stream ended as announced. Anything else removes the partial file.

use imt_ipc::child_log::{self, ChildEvent, ChildLogSpec};
use imt_ipc::peer::CallCtx;
use imt_ipc::StreamEvent;
use imt_plugin_api::output::LocalFile;
use imt_plugin_api::protocol::{
    cut_line, OutputEnd, PluginRequest, SavedOutput, MAX_LOG_BATCH, MAX_LOG_LINE, MAX_LOG_LINES, MAX_OUTPUTS, OUTPUT_CHUNK, OUTPUT_CREDIT,
};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

/// How often a waiting save looks whether the plugin gave up the stream.
const POLL: Duration = Duration::from_millis(500);

/// One output stream being saved (counts against [`MAX_OUTPUTS`]).
struct Active<'a>(&'a AtomicUsize);

impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// What the host needs to save one stream.
pub(crate) struct OutputRequest<'a> {
    pub data_dir: Option<&'a Path>,
    pub stream: u64,
    pub folder: &'a str,
    pub name: &'a str,
    pub max_bytes: u64,
}

/// Saves the stream `req` names, pulling it from the process that asked (`ctx`'s connection), into
/// `<data_dir>/captures/<folder>/<name>`. `active` counts the streams being saved for the plugin.
pub(crate) fn save_output(ctx: &CallCtx, req: &OutputRequest<'_>, active: &AtomicUsize) -> Result<SavedOutput, String> {
    if active.fetch_add(1, Ordering::SeqCst) >= MAX_OUTPUTS {
        active.fetch_sub(1, Ordering::SeqCst);
        return Err(format!("too many output files are being written at once (at most {MAX_OUTPUTS})"));
    }
    let _active = Active(active);
    let data = req.data_dir.ok_or("IMT has no data folder for this tool's output")?;
    let label = format!("{}/{}", req.folder, req.name);
    let mut file = LocalFile::create(data, req.folder, req.name, req.max_bytes).map_err(|e| format!("{e:#}"))?;
    let broke = |why: String| format!("the output {label} broke off ({why}); the partial file was removed");
    let mut s = ctx
        .peer()
        .stream_request(&PluginRequest::PullOutput { stream: req.stream }, OUTPUT_CREDIT)
        .map_err(|e| broke(e.to_string()))?;
    // `file` is dropped on every early return, which removes the partial file.
    loop {
        match s.next(Some(POLL)) {
            Ok(StreamEvent::Idle) if ctx.is_cancelled() => {
                s.cancel();
                return Err(broke("the plugin gave it up".into()));
            }
            Ok(StreamEvent::Idle) => {}
            Ok(StreamEvent::Item(piece)) => {
                if piece.len() > OUTPUT_CHUNK {
                    s.cancel();
                    return Err(broke(format!(
                        "a piece of {} bytes is over the limit of {OUTPUT_CHUNK}",
                        piece.len()
                    )));
                }
                if file.written() + piece.len() as u64 > file.limit() {
                    s.cancel();
                    return Err(format!(
                        "the output {label} exceeds its limit of {} bytes, so IMT cut it off and removed the partial file",
                        file.limit()
                    ));
                }
                file.write_all(&piece).map_err(|e| broke(format!("cannot write it: {e}")))?;
            }
            Ok(StreamEvent::Done(b)) => {
                let end: OutputEnd = imt_ipc::wire::decode(&b).map_err(broke)?;
                if end.bytes != file.written() {
                    return Err(broke(format!(
                        "the plugin announced {} bytes but sent {}",
                        end.bytes,
                        file.written()
                    )));
                }
                return imt_plugin_api::output::OutputFile::finish(Box::new(file)).map_err(|e| format!("{e:#}"));
            }
            Err(e) => return Err(broke(e.to_string())),
        }
    }
}

/// The elevated process' forwarded standard error, logged like a normal child's (its own child log), bounded per
/// start of the process.
pub(crate) struct ForwardedLog {
    spec: ChildLogSpec,
    logged: AtomicU64,
    capped: AtomicBool,
}

impl ForwardedLog {
    pub(crate) fn new(spec: ChildLogSpec) -> ForwardedLog {
        ForwardedLog {
            spec,
            logged: AtomicU64::new(0),
            capped: AtomicBool::new(false),
        }
    }

    /// A line of IMT's own about the process (e.g. why one of its output streams failed).
    pub(crate) fn note(&self, text: &str) {
        child_log::get(&self.spec).record(&ChildEvent::Note(text.to_string()));
    }

    /// A new start of the process: the cap starts again.
    pub(crate) fn restart(&self) {
        self.logged.store(0, Ordering::SeqCst);
        self.capped.store(false, Ordering::SeqCst);
    }

    /// Logs one [`imt_plugin_api::protocol::HostRequest::Log`].
    pub(crate) fn lines(&self, lines: Vec<String>, dropped: u64) {
        let log = child_log::get(&self.spec);
        let extra = lines.len().saturating_sub(MAX_LOG_BATCH) as u64;
        for line in lines.into_iter().take(MAX_LOG_BATCH) {
            if self.logged.fetch_add(1, Ordering::SeqCst) >= MAX_LOG_LINES {
                if !self.capped.swap(true, Ordering::SeqCst) {
                    log.record(&ChildEvent::Note(format!(
                        "The elevated process wrote more than {MAX_LOG_LINES} lines; IMT drops the rest until it restarts"
                    )));
                }
                return;
            }
            log.output(cut_line(&line, MAX_LOG_LINE));
        }
        let dropped = dropped.saturating_add(extra);
        if dropped > 0 && !self.capped.load(Ordering::SeqCst) {
            log.record(&ChildEvent::Note(format!("{dropped} lines of the elevated process were dropped")));
        }
    }
}
