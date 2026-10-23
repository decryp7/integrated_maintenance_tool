//! The two tools the "Profile native process…" dialog calls (ADR 0022 §2):
//!
//! | Tool | Op | Args | Result |
//! |---|---|---|---|
//! | `native-profiler` | `defaults` | — | `NativeDefaults` |
//! | | `processes` | — | `NativeProcess[]` |
//! | | `start` | `NativeCaptureRequest` (sampling) | `NativeCaptureStatus` (a running capture is a tool task → a Job) |
//! | | `status` | — | `{ captures, outputs }` (pollable) |
//! | | `stop` | `{ id }` | `NativeCaptureStatus` (kept; cancel is the Job's) |
//! | | `etl-processes` | `{ path }` | `EtlProcess[]`, .NET ones first (Windows; the one `.etl` picker for .NET and native) |
//! | | `etl-convert` | `{ path, pid, symbolPath?, symbolServer? }` | `{ path, notes, outputs }` |
//! | `native-profiler.etw` (elevated) | `start` | `NativeCaptureRequest` (ETW) | `NativeCaptureStatus` |
//! | | `status` | — | `{ captures, outputs }` (pollable) |
//! | | `stop` | `{ id }` | `NativeCaptureStatus` |

use crate::captures::{capture_file, capture_name, CaptureStatus, Captures};
use crate::request::{CaptureKind, CaptureRequest, SymbolOptions, DEFAULT_RATE_HZ, MAX_DURATION_SEC, MAX_RATE_HZ, MAX_SYMBOL_PATH};
use crate::settings::Knobs;
use crate::{paths, process, Limits, NativeError, ETW_TOOL_ID, TASK_KIND, TOOL_ID};
use anyhow::{anyhow, bail, Context, Result};
use debrief_plugin_api::tools::{OpSpec, PathAccess, PathKind, Platform, Schema, Tool, ToolCtx, ToolFailure, ToolInfo, ToolTask};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Largest `.etl` file converted.
pub const MAX_ETL_BYTES: u64 = 8 << 30;
/// How long `stop` waits for the file (symbolisation included).
const STOP_WAIT: Duration = Duration::from_secs(110);

/// A tool failure the host reports with the `denied` code (and the restart hint).
fn tool_error(e: NativeError) -> anyhow::Error {
    match e {
        NativeError::Denied(m) => anyhow::Error::new(ToolFailure::denied(m)),
        other => anyhow!(other),
    }
}

/// `{ captures, outputs }`: finished files are tool outputs the window may open.
fn status_json(captures: &Captures) -> Value {
    let list = captures.list();
    let outputs: Vec<&str> = list.iter().filter_map(|c| c.path.as_deref()).collect();
    json!({ "captures": list, "outputs": outputs })
}

fn id_arg(args: &Value) -> &str {
    args.get("id").and_then(Value::as_str).unwrap_or_default()
}

/// Reports a running capture as a tool task and ends the task when the capture ends.
fn report_task(captures: &Arc<Captures>, status: &CaptureStatus, ctx: &ToolCtx) -> Result<()> {
    if !captures.is_running(&status.id) {
        return Ok(());
    }
    let what = if status.mode == "etw" { "ETW" } else { "Sampling" };
    ctx.task_started(ToolTask {
        id: status.id.clone(),
        label: format!("{what}: {} ({})", status.process_name, status.pid),
        kind: TASK_KIND.into(),
        stoppable: true,
    });
    let (captures, ctx, id) = (captures.clone(), ctx.clone(), status.id.clone());
    std::thread::Builder::new()
        .name("debrief-native-watch".into())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_millis(250));
            captures.reap();
            if !captures.is_running(&id) {
                ctx.task_finished(&id);
                break;
            }
        })
        .context("cannot watch the capture")?;
    Ok(())
}

fn capture_args(etw: bool) -> Schema {
    let s = Schema::object()
        .field("pid", Schema::int(1, i64::from(u32::MAX)))
        .optional("durationSec", Schema::int(1, MAX_DURATION_SEC as i64))
        .optional("symbolPath", Schema::text(MAX_SYMBOL_PATH))
        .optional("symbolServer", Schema::Bool);
    if etw {
        s.optional("heap", Schema::Bool)
    } else {
        s.optional("rateHz", Schema::int(1, i64::from(MAX_RATE_HZ)))
    }
}

/// The normal process's tool: listing, sampling and `.etl`.
pub struct NativeTool {
    captures: Arc<Captures>,
    knobs: Arc<Knobs>,
    limits: Limits,
    conversions: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl NativeTool {
    pub fn new(captures: Arc<Captures>, knobs: Arc<Knobs>, limits: Limits) -> NativeTool {
        NativeTool {
            captures,
            knobs,
            limits,
            conversions: Mutex::new(HashMap::new()),
        }
    }

    fn defaults(&self) -> Value {
        json!({
            "symbolPath": self.knobs.symbol_path(),
            "symbolServer": self.knobs.symbol_server(),
            "rateHz": DEFAULT_RATE_HZ,
            "maxRateHz": MAX_RATE_HZ,
            "maxDurationSec": MAX_DURATION_SEC,
            "supported": cfg!(windows),
        })
    }

    fn start(&self, args: &Value, ctx: &ToolCtx) -> Result<Value> {
        let req = CaptureRequest::parse(args, false).map_err(|e| anyhow!(e))?;
        let CaptureKind::Sampling { rate_hz } = req.kind else {
            bail!("not a sampling request");
        };
        let target = process::list()
            .into_iter()
            .find(|p| p.pid == req.pid)
            .ok_or(NativeError::NotFound { pid: req.pid })
            .map_err(tool_error)?;
        if !target.accessible {
            return Err(tool_error(NativeError::Denied(format!(
                "Debrief cannot read {} ({}): it runs elevated or as another user. Record it with ETW (administrator)",
                target.name, target.pid
            ))));
        }
        let dir = paths::capture_dir(ctx.data_dir()).map_err(tool_error)?;
        let out = capture_file(&dir, &target.name, target.pid);
        let symbols = req.symbols.config(ctx.data_dir());
        let limits = self.limits;
        let duration = req.duration.unwrap_or(limits.max_duration).min(limits.max_duration);
        let status = self
            .captures
            .spawn("sampling", target.pid, &target.name, "sampling", move |ctl| {
                run_sampling(target.pid, rate_hz, duration, &out, &symbols, limits, &ctl)
            })
            .map_err(tool_error)?;
        report_task(&self.captures, &status, ctx)?;
        Ok(serde_json::to_value(status)?)
    }

    fn etl_path(args: &Value) -> Result<PathBuf> {
        let raw = args.get("path").and_then(Value::as_str).unwrap_or_default();
        let path = Path::new(raw).canonicalize().with_context(|| format!("cannot open {raw}"))?;
        if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("etl")) {
            bail!("{} is not an .etl file", path.display());
        }
        let len = std::fs::metadata(&path)?.len();
        if len > MAX_ETL_BYTES {
            bail!(
                "{} is larger than {} GB; record a shorter trace",
                path.display(),
                MAX_ETL_BYTES >> 30
            );
        }
        Ok(path)
    }

    fn etl_convert(&self, args: &Value, ctx: &ToolCtx) -> Result<Value> {
        let path = Self::etl_path(args)?;
        let pid = args
            .get("pid")
            .and_then(Value::as_u64)
            .and_then(|p| u32::try_from(p).ok())
            .ok_or_else(|| anyhow!("choose a process"))?;
        let symbols = SymbolOptions::parse(
            args.get("symbolPath").and_then(Value::as_str).unwrap_or(""),
            args.get("symbolServer").and_then(Value::as_bool).unwrap_or(false),
        )
        .map_err(|e| anyhow!(e))?
        .config(ctx.data_dir());
        let dir = paths::capture_dir(ctx.data_dir()).map_err(tool_error)?;
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let out = capture_file(&dir, &stem, pid);
        let id = format!("etl-{pid}-{}", self.conversions.lock().unwrap_or_else(|p| p.into_inner()).len());
        let cancel = Arc::new(AtomicBool::new(false));
        self.conversions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id.clone(), cancel.clone());
        ctx.task_started(ToolTask {
            id: id.clone(),
            label: format!(
                "Converting {} (process {pid})",
                path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
            ),
            kind: TASK_KIND.into(),
            stoppable: true,
        });
        let result = etl::convert(&path, pid, &out, &symbols, self.limits, &cancel);
        ctx.task_finished(&id);
        self.conversions.lock().unwrap_or_else(|p| p.into_inner()).remove(&id);
        let notes = result.inspect_err(|_| {
            let _ = std::fs::remove_file(&out);
        })?;
        let out = out.to_string_lossy().into_owned();
        Ok(json!({ "path": out, "notes": notes, "outputs": [out] }))
    }
}

/// Records, symbolises and writes one sampling capture (the capture thread's body).
fn run_sampling(
    pid: u32,
    rate_hz: u32,
    duration: Duration,
    out: &Path,
    symbols: &debrief_symbols::SymbolConfig,
    limits: Limits,
    ctl: &crate::captures::Control,
) -> Result<crate::Finished, NativeError> {
    #[cfg(windows)]
    {
        use crate::sampling::{live, write_nettrace};
        let spec = live::SamplingSpec {
            rate_hz,
            duration,
            max_samples: limits.max_samples,
        };
        let (set, mut notes) = live::record(pid, &spec, &|| ctl.stopping(), &|n| ctl.set_samples(n))?;
        if ctl.cancelled() {
            return Err(NativeError::Cancelled);
        }
        let (syms, sym_notes) = crate::symbolize::symbolize(symbols, &set.images, &set.addresses(), limits.max_addresses);
        notes.extend(sym_notes);
        let file = std::fs::File::create(out)?;
        let written = write_nettrace(&set, &syms, std::io::BufWriter::with_capacity(1 << 20, file)).and_then(|(w, stats)| {
            let f = w.into_inner().map_err(|e| e.into_error())?;
            f.sync_all()?;
            Ok((f.metadata()?.len(), stats))
        });
        let (bytes, stats) = written.inspect_err(|_| {
            let _ = std::fs::remove_file(out);
        })?;
        if stats.samples + stats.waits == 0 {
            notes.push("No samples were taken (the process had no threads Debrief could sample)".into());
        }
        Ok(crate::Finished {
            path: out.to_path_buf(),
            bytes,
            truncated: set.dropped > 0,
            notes,
        })
    }
    #[cfg(not(windows))]
    {
        let _ = (pid, rate_hz, duration, out, symbols, limits, ctl);
        Err(NativeError::Unsupported("native profiling is only available on Windows".into()))
    }
}

impl Tool for NativeTool {
    fn id(&self) -> &str {
        TOOL_ID
    }

    fn describe(&self) -> ToolInfo {
        ToolInfo::new(TOOL_ID, "Native profiler")
            .describe(
                "Samples native (C/C++) processes from outside (suspend, copy the stack, resume; no administrator rights) \
                 and converts .etl traces. Nothing is loaded into the target process.",
            )
            .only_on(&[Platform::Windows])
            .op(OpSpec::new("defaults", "Capture defaults").args(Schema::object()))
            .op(OpSpec::new("processes", "List processes").args(Schema::object()))
            .op(OpSpec::new("start", "Start a sampling capture").args(capture_args(false)))
            .op(OpSpec::new("status", "List captures").args(Schema::object()).polling())
            .op(OpSpec::new("stop", "Stop a capture")
                .args(Schema::object().field("id", Schema::text(64)))
                .budget(Some(Duration::from_secs(120))))
            .op(OpSpec::new("etl-processes", "List the processes in an .etl trace")
                .args(Schema::object().field("path", Schema::path(PathKind::File, PathAccess::Read)))
                .budget(Some(Duration::from_secs(1800))))
            .op(OpSpec::new("etl-convert", "Convert one process of an .etl trace")
                .args(
                    Schema::object()
                        .field("path", Schema::path(PathKind::File, PathAccess::Read))
                        .field("pid", Schema::int(1, i64::from(u32::MAX)))
                        .optional("symbolPath", Schema::text(MAX_SYMBOL_PATH))
                        .optional("symbolServer", Schema::Bool),
                )
                .budget(Some(Duration::from_secs(3600))))
    }

    fn invoke(&self, op: &str, args: Value, ctx: &ToolCtx) -> Result<Value> {
        match op {
            "defaults" => Ok(self.defaults()),
            "processes" => Ok(json!(process::list())),
            "start" => self.start(&args, ctx),
            "status" => Ok(status_json(&self.captures)),
            "stop" => Ok(serde_json::to_value(
                self.captures.stop(id_arg(&args), STOP_WAIT).map_err(tool_error)?,
            )?),
            "etl-processes" => {
                let path = Self::etl_path(&args)?;
                Ok(json!(etl::processes(&path)?))
            }
            "etl-convert" => self.etl_convert(&args, ctx),
            other => bail!("unknown operation {other}"),
        }
    }

    /// Cancel from the Jobs pane: a capture stops at once and its file is deleted; a conversion is cancelled.
    fn stop(&self, task: &str, _ctx: &ToolCtx) -> Result<()> {
        if let Some(flag) = self.conversions.lock().unwrap_or_else(|p| p.into_inner()).get(task) {
            flag.store(true, Ordering::Relaxed);
            return Ok(());
        }
        self.captures.cancel(task, STOP_WAIT).map(|_| ()).map_err(tool_error)
    }
}

/// The elevated process's tool: ETW captures.
pub struct EtwTool {
    captures: Arc<Captures>,
    limits: Limits,
}

impl EtwTool {
    pub fn new(captures: Arc<Captures>, limits: Limits) -> EtwTool {
        EtwTool { captures, limits }
    }

    fn start(&self, args: &Value, ctx: &ToolCtx) -> Result<Value> {
        let req = CaptureRequest::parse(args, true).map_err(|e| anyhow!(e))?;
        let CaptureKind::Etw { heap } = req.kind else {
            bail!("not an ETW request");
        };
        let target = process::list()
            .into_iter()
            .find(|p| p.pid == req.pid)
            .ok_or(NativeError::NotFound { pid: req.pid })
            .map_err(tool_error)?;
        // ADR 0022 §7: the elevated process writes only below the user's own application-data folders (its symbol
        // cache). The capture itself is not written by it at all: its bytes are streamed to the host, which saves
        // them as `<data>/captures/native/<name>` (ADR 0017 §7).
        let data = paths::elevated_data_dir(ctx.data_dir()).map_err(tool_error)?;
        let outputs = ctx
            .outputs()
            .cloned()
            .ok_or_else(|| anyhow!("this Debrief gave the elevated recorder no way to save its capture; update Debrief"))?;
        let name = capture_name(&target.name, target.pid);
        // Where the host saves it (its own data folder, as it named it).
        let saved_at = ctx
            .data_dir()
            .map(|d| d.join("captures").join(paths::CAPTURE_FOLDER).join(&name))
            .ok_or_else(|| anyhow!("Debrief has no data folder to write captures to"))?;
        let symbols = req.symbols.config(Some(&data));
        let spec = crate::etw::EtwSpec {
            heap,
            duration: req.duration.unwrap_or(self.limits.max_duration).min(self.limits.max_duration),
            max_bytes: self.limits.max_bytes,
            max_addresses: self.limits.max_addresses,
        };
        let pid = target.pid;
        let status = self
            .captures
            .spawn("etw", pid, &target.name, "etw", move |ctl| {
                // A size limit is reached by the recorder itself, which then writes its closing blocks: the host's
                // cut is a little above it, so only a stream that runs away is cut.
                let cut = spec.max_bytes.saturating_add(crate::etw::CLOSING_ALLOWANCE);
                let sink = outputs
                    .create(paths::CAPTURE_FOLDER, &name, cut)
                    .map_err(|e| NativeError::Failed(format!("{e:#}")))?;
                crate::etw::record(pid, &spec, sink, &saved_at, &symbols, &ctl)
            })
            .map_err(tool_error)?;
        report_task(&self.captures, &status, ctx)?;
        Ok(serde_json::to_value(status)?)
    }
}

impl Tool for EtwTool {
    fn id(&self) -> &str {
        ETW_TOOL_ID
    }

    fn describe(&self) -> ToolInfo {
        let elevated = |op: OpSpec| op.elevated();
        ToolInfo::new(ETW_TOOL_ID, "Native profiler (ETW)")
            .describe(
                "Records native processes with ETW: every CPU sample, waits with the thread that ended them, and heap \
                 allocations with their stacks. Runs in the plugin's elevated process (administrator prompt).",
            )
            .only_on(&[Platform::Windows])
            .needs_elevation()
            .op(elevated(OpSpec::new("start", "Start an ETW capture").args(capture_args(true))))
            .op(elevated(
                OpSpec::new("status", "List ETW captures").args(Schema::object()).polling(),
            ))
            .op(elevated(
                OpSpec::new("stop", "Stop an ETW capture")
                    .args(Schema::object().field("id", Schema::text(64)))
                    .budget(Some(Duration::from_secs(120))),
            ))
    }

    fn invoke(&self, op: &str, args: Value, ctx: &ToolCtx) -> Result<Value> {
        match op {
            "start" => self.start(&args, ctx),
            "status" => Ok(status_json(&self.captures)),
            "stop" => Ok(serde_json::to_value(
                self.captures.stop(id_arg(&args), STOP_WAIT).map_err(tool_error)?,
            )?),
            other => bail!("unknown operation {other}"),
        }
    }

    fn stop(&self, task: &str, _ctx: &ToolCtx) -> Result<()> {
        self.captures.cancel(task, STOP_WAIT).map(|_| ()).map_err(tool_error)
    }
}

#[cfg(windows)]
mod etl {
    use crate::{symbolize, Limits};
    use anyhow::{anyhow, Result};
    use debrief_etw::convert::Options;
    use debrief_etw::etl;
    use debrief_etw::scan::{EtlProcess, Order};
    use debrief_plugin_profiling::nettrace::ext::{ImageLoad, NativeSymbol};
    use debrief_symbols::SymbolConfig;
    use std::path::Path;
    use std::sync::atomic::AtomicBool;

    pub(super) fn processes(path: &Path) -> Result<Vec<EtlProcess>> {
        Ok(etl::scan(path, Order::DotnetFirst, &AtomicBool::new(false))?)
    }

    /// Converts process `pid` of `path` to NetTrace at `out` with symbols; returns the notes.
    pub(super) fn convert(
        path: &Path,
        pid: u32,
        out: &Path,
        symbols: &SymbolConfig,
        limits: Limits,
        cancel: &AtomicBool,
    ) -> Result<Vec<String>> {
        let procs = etl::scan(path, Order::DotnetFirst, cancel)?;
        let p = procs
            .iter()
            .find(|p| p.pid == pid)
            .ok_or_else(|| anyhow!("process {pid} is not in the trace"))?;
        let mut sym_notes = Vec::new();
        let mut name = |images: &[ImageLoad], addresses: &[u64]| -> Vec<NativeSymbol> {
            let (s, n) = symbolize::symbolize(symbols, images, addresses, limits.max_addresses);
            sym_notes = n;
            s
        };
        let options = Options {
            heap: true,
            max_addresses: limits.max_addresses,
        };
        let c = etl::convert(path, p, out, limits.max_bytes, cancel, options, &mut name)?;
        let mut notes = c.notes;
        notes.extend(sym_notes);
        Ok(notes)
    }
}

#[cfg(not(windows))]
mod etl {
    use crate::Limits;
    use anyhow::{bail, Result};
    use debrief_etw::scan::EtlProcess;
    use debrief_symbols::SymbolConfig;
    use std::path::Path;
    use std::sync::atomic::AtomicBool;

    pub(super) fn processes(_: &Path) -> Result<Vec<EtlProcess>> {
        bail!("ETL import needs Windows")
    }

    pub(super) fn convert(_: &Path, _: u32, _: &Path, _: &SymbolConfig, _: Limits, _: &AtomicBool) -> Result<Vec<String>> {
        bail!("ETL import needs Windows")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use debrief_plugin_api::tools::{failure_reason, FailureReason, NoTasks};

    fn ctx(dir: Option<&Path>) -> ToolCtx {
        ToolCtx::new(dir.map(Path::to_path_buf), Arc::new(NoTasks)).for_tool(TOOL_ID)
    }

    fn tool() -> NativeTool {
        NativeTool::new(Arc::default(), Arc::default(), Limits::default())
    }

    #[test]
    fn descriptions_are_valid_and_the_etw_tool_is_all_elevated() {
        let n = tool().describe();
        n.check().unwrap();
        assert!(n.ops.iter().all(|o| !o.elevated));
        assert!(n.ops.iter().any(|o| o.name == "status" && o.poll));
        let e = EtwTool::new(Arc::default(), Limits::default()).describe();
        e.check().unwrap();
        assert!(
            e.ops.iter().all(|o| o.elevated),
            "Tool::stop goes to the elevated process only then"
        );
        assert!(e.needs_elevation);
        // Path arguments are typed, so the host checks them against the window's grants.
        let etl = n.ops.iter().find(|o| o.name == "etl-convert").unwrap();
        assert!(etl.args.has_paths());
    }

    #[test]
    fn defaults_status_and_refusals() {
        let t = tool();
        let d = t.invoke("defaults", json!({}), &ctx(None)).unwrap();
        assert_eq!(d["rateHz"], DEFAULT_RATE_HZ);
        assert_eq!(d["symbolServer"], false);
        let s = t.invoke("status", json!({}), &ctx(None)).unwrap();
        assert!(s["captures"].as_array().unwrap().is_empty() && s["outputs"].as_array().unwrap().is_empty());
        assert!(t.invoke("start", json!({ "pid": 0 }), &ctx(None)).is_err());
        let gone = t.invoke("start", json!({ "pid": u32::MAX - 5 }), &ctx(None)).unwrap_err();
        assert!(gone.to_string().contains("not running"), "{gone:#}");
        assert!(t.invoke("stop", json!({ "id": "sampling-99" }), &ctx(None)).is_err());
        assert!(t.invoke("nope", json!({}), &ctx(None)).is_err());
        assert!(t.stop("sampling-99", &ctx(None)).is_err());
        let dir = tempfile::tempdir().unwrap();
        let txt = dir.path().join("x.txt");
        std::fs::write(&txt, "x").unwrap();
        let e = t
            .invoke("etl-processes", json!({ "path": txt.to_string_lossy() }), &ctx(None))
            .unwrap_err();
        assert!(e.to_string().contains("not an .etl file"), "{e:#}");
        assert!(t
            .invoke("etl-convert", json!({ "path": txt.to_string_lossy() }), &ctx(None))
            .is_err());
        let etw = EtwTool::new(Arc::default(), Limits::default());
        assert!(etw.invoke("start", json!({ "pid": 1, "rateHz": 5 }), &ctx(None)).is_err());
        assert!(etw.invoke("other", json!({}), &ctx(None)).is_err());
        assert!(etw.invoke("status", json!({}), &ctx(None)).unwrap()["captures"].is_array());
        assert!(failure_reason(&tool_error(NativeError::Denied("x".into()))) == Some(FailureReason::Denied));
    }

    #[cfg(windows)]
    #[test]
    fn junk_etl_files_are_errors_not_panics() {
        let dir = tempfile::tempdir().unwrap();
        let junk = dir.path().join("junk.etl");
        std::fs::write(&junk, vec![0x5a; 4096]).unwrap();
        let t = tool();
        assert!(t
            .invoke("etl-processes", json!({ "path": junk.to_string_lossy() }), &ctx(None))
            .is_err());
        let r = t.invoke(
            "etl-convert",
            json!({ "path": junk.to_string_lossy(), "pid": 4 }),
            &ctx(Some(dir.path())),
        );
        assert!(r.is_err());
        let left: Vec<_> = std::fs::read_dir(dir.path().join("captures").join("native")).unwrap().collect();
        assert!(left.is_empty(), "a failed conversion leaves no file");
    }
}
