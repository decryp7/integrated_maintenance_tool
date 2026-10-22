//! The plugin process' side: [`serve`] runs Debrief's one child loop (`debrief_ipc::child`) and answers the host's
//! [`PluginRequest`]s with the plugin's contributions.

use crate::annotate::{AnnotationView, ViewFilter, ViewRecord};
use crate::host_file::HostFile;
use crate::importer::{ImportCtx, Imported};
use crate::manifest::Manifest;
use crate::plugin::{Contributions, Plugin, TransformCtx, Verdict};
use crate::protocol::*;
use crate::settings::validate;
use crate::time::TimestampParser;
use crate::tools::{
    failure_reason, ElevatedOps, FailureReason, PathAccess, PathKind, PathPolicy, TaskSink, ToolCtx, ToolFailure, ToolTask,
};
use anyhow::{anyhow, Result};
use debrief_ipc::peer::{code, decode_request, encode_reply, CallCtx, Handler};
use debrief_ipc::{Peer, RemoteError};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// The role of the plugin's elevated process (ADR 0017 §7): it serves only its plugin's elevated operations.
pub const ELEVATED_ROLE: &str = "elevated";
/// Largest file an importer reads whole (plugins run under a memory cap anyway).
pub const MAX_IMPORT_BYTES: u64 = 2 << 30;

/// Serves `plugin` in this process (its `main`): connects to Debrief, locks down and answers until Debrief closes the
/// connection.
pub fn serve(plugin: Arc<dyn Plugin>) -> std::io::Result<()> {
    debrief_ipc::child::child_main_then(
        "Debrief",
        PROTOCOL,
        move |boot| Arc::new(PluginHandler::new(plugin, boot.role == ELEVATED_ROLE)),
        |boot, peer| {
            // The elevated process has no standard error the host reads: it forwards its own.
            if boot.role == ELEVATED_ROLE {
                crate::forward::forward_stderr(peer);
            }
        },
    )
}

/// Answers the host's requests for one plugin.
pub struct PluginHandler {
    plugin: Arc<dyn Plugin>,
    c: Contributions,
    registered: Result<Manifest, String>,
    elevated: bool,
}

fn invalid(m: impl Into<String>) -> RemoteError {
    RemoteError::new(code::INVALID, m)
}

fn failed(e: anyhow::Error) -> RemoteError {
    RemoteError::new(code::INTERNAL, format!("{e:#}"))
}

impl PluginHandler {
    /// Registers the plugin's contributions (a registration error is answered to every request).
    pub fn new(plugin: Arc<dyn Plugin>, elevated: bool) -> PluginHandler {
        let mut c = Contributions::default();
        let registered = plugin.register(&mut c).map_err(|e| format!("{e:#}")).and_then(|()| {
            let m = c.manifest(plugin.as_ref());
            m.check().map(|()| m)
        });
        PluginHandler {
            plugin,
            c,
            registered,
            elevated,
        }
    }

    fn timestamps(t: &TimestampSettings) -> Result<TimestampParser, RemoteError> {
        TimestampParser::new(&t.formats, &t.zone, t.day_first).map_err(|e| invalid(format!("{e:#}")))
    }

    fn date(d: &Option<String>) -> Option<chrono::NaiveDate> {
        d.as_deref().and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
    }

    /// Streams the records; returns what the end of the call carries (skipped lines, problems).
    fn send_imported(ctx: &CallCtx, imported: Imported) -> Result<(u64, Vec<crate::problems::Problem>), RemoteError> {
        let skipped = imported.skipped as u64;
        chunk_events(imported.records.iter().map(WireEvent::of), |item| ctx.send(&item))?;
        Ok((skipped, imported.problems.into_vec()))
    }

    fn dispatch(&self, ctx: &CallCtx, req: PluginRequest) -> Result<Vec<u8>, RemoteError> {
        let manifest = self.registered.as_ref().map_err(|e| RemoteError::new(code::INTERNAL, e.clone()))?;
        match req {
            PluginRequest::Manifest => encode_reply(&debrief_ipc::Json(serde_json::to_value(manifest).map_err(|e| failed(e.into()))?)),
            // Any process may be asked for the streams it opened (only the elevated one opens any).
            PluginRequest::PullOutput { stream } => crate::output::serve_pull(ctx, stream),
            _ if self.elevated && !matches!(req, PluginRequest::InvokeTool { .. } | PluginRequest::StopTask { .. }) => {
                Err(invalid("the elevated process only runs elevated tool operations"))
            }
            PluginRequest::Sniff { importer, file, head } => {
                let ts = Self::timestamps(&file.timestamps)?;
                let path = PathBuf::from(&file.path);
                let cancelled = || ctx.is_cancelled();
                let ictx = ImportCtx::of(&file.name, &path, Self::date(&file.date), &ts).with_cancel(&cancelled);
                let score = if let Some(i) = self.c.text.iter().find(|i| i.id() == &*importer) {
                    i.sniff(&ictx, &String::from_utf8_lossy(&head))
                } else if let Some(i) = self.c.binary.iter().find(|i| i.id() == &*importer) {
                    i.sniff(&ictx, &head)
                } else {
                    return Err(invalid(format!("no importer \"{importer}\"")));
                };
                encode_reply(&score.min(100))
            }
            PluginRequest::Import { importer, file, content } => {
                let ts = Self::timestamps(&file.timestamps)?;
                let path = PathBuf::from(&file.path);
                let cancelled = || ctx.is_cancelled();
                let ictx = ImportCtx::of(&file.name, &path, Self::date(&file.date), &ts).with_cancel(&cancelled);
                let f = HostFile::new(ctx.peer(), content).with_cancel(&cancelled);
                // Whatever fails after a cancel is the cancel.
                let failed = |e: anyhow::Error| {
                    if ctx.is_cancelled() {
                        CallCtx::cancelled_error()
                    } else {
                        failed(e)
                    }
                };
                let imported = if let Some(i) = self.c.text.iter().find(|i| i.id() == &*importer) {
                    let text = f.read_text(MAX_IMPORT_BYTES).map_err(failed)?;
                    i.import(&ictx, &text).map_err(failed)?
                } else if let Some(i) = self.c.binary.iter().find(|i| i.id() == &*importer) {
                    let bytes = f.read_all(MAX_IMPORT_BYTES).map_err(failed)?;
                    i.import(&ictx, &bytes).map_err(failed)?
                } else {
                    return Err(invalid(format!("no importer \"{importer}\"")));
                };
                let (skipped, problems) = Self::send_imported(ctx, imported)?;
                encode_reply(&ImportDone { skipped, problems })
            }
            PluginRequest::ImportTail {
                importer,
                file,
                head,
                tail,
            } => {
                let ts = Self::timestamps(&file.timestamps)?;
                let path = PathBuf::from(&file.path);
                let cancelled = || ctx.is_cancelled();
                let ictx = ImportCtx::of(&file.name, &path, Self::date(&file.date), &ts).with_cancel(&cancelled);
                let i = self
                    .c
                    .text
                    .iter()
                    .find(|i| i.id() == &*importer)
                    .ok_or_else(|| invalid(format!("no text importer \"{importer}\"")))?;
                let head = HostFile::new(ctx.peer(), head).read_text(MAX_IMPORT_BYTES).map_err(failed)?;
                let tail = HostFile::new(ctx.peer(), tail).read_text(MAX_IMPORT_BYTES).map_err(failed)?;
                match i.import_tail(&ictx, &head, &tail) {
                    None => encode_reply(&TailDone {
                        supported: false,
                        skipped: 0,
                        problems: Vec::new(),
                    }),
                    Some(r) => {
                        let (skipped, problems) = Self::send_imported(ctx, r.map_err(failed)?)?;
                        encode_reply(&TailDone {
                            supported: true,
                            skipped,
                            problems,
                        })
                    }
                }
            }
            PluginRequest::SniffConfig { parser, file_name, head } => {
                let p = self.config_parser(&parser)?;
                encode_reply(&p.sniff(&file_name, &head).min(100))
            }
            PluginRequest::ParseConfig { parser, content, limits } => {
                let p = self.config_parser(&parser)?;
                let f = HostFile::new(ctx.peer(), content);
                let result = match f.read_text(limits.max_bytes as u64) {
                    Ok(text) => p.parse(&text, &limits),
                    Err(e) => Err(crate::config::ConfigError::Io(format!("{e:#}"))),
                };
                encode_reply(&result)
            }
            PluginRequest::Annotate { annotator, ticket, since } => {
                let a = self
                    .c
                    .annotators
                    .iter()
                    .find(|a| a.id() == &*annotator)
                    .ok_or_else(|| invalid(format!("no annotator \"{annotator}\"")))?;
                let view = RemoteView { peer: ctx.peer(), ticket };
                let mut list = a.annotate(&view, since).map_err(failed)?;
                list.truncate(MAX_ANNOTATIONS);
                encode_reply(&list)
            }
            PluginRequest::Transform {
                transform,
                source_name,
                rows,
            } => {
                let t = self
                    .c
                    .transforms
                    .iter()
                    .find(|t| t.id() == &*transform)
                    .ok_or_else(|| invalid(format!("no transform \"{transform}\"")))?;
                let tctx = TransformCtx::new(&source_name);
                let mut out = TransformOut::default();
                for (i, row) in rows.into_iter().enumerate() {
                    let mut r = row.into_record();
                    let verdict = t.apply(&mut r, &tctx);
                    out.marks.extend(tctx.take_marks().into_iter().map(|k| (i as u32, k)));
                    out.keep.push(verdict == Verdict::Keep);
                    out.rows.push(WireEvent::of(&r));
                }
                encode_reply(&out)
            }
            PluginRequest::ApplySettings { values } => {
                let (Some(s), Some(section)) = (&self.c.settings, &manifest.settings) else {
                    return Err(invalid("this plugin has no settings"));
                };
                let valid = validate(section, &values).map_err(|errors| {
                    invalid(
                        errors
                            .iter()
                            .map(|e| format!("{}: {}", e.key, e.message))
                            .collect::<Vec<_>>()
                            .join("; "),
                    )
                })?;
                s.apply_settings(&valid).map_err(failed)?;
                encode_reply(&())
            }
            PluginRequest::InvokeTool { tool, op, args, data_dir } => {
                let (info, t) = self.tool(&tool)?;
                let spec = info
                    .ops
                    .iter()
                    .find(|o| o.name == op)
                    .ok_or_else(|| invalid(format!("tool \"{tool}\" has no operation \"{op}\"")))?;
                if spec.elevated != self.elevated {
                    return Err(invalid(if spec.elevated {
                        format!("\"{tool} {op}\" runs in the plugin's elevated process")
                    } else {
                        format!("\"{tool} {op}\" does not run in the elevated process")
                    }));
                }
                // The host validated the arguments (and the paths); check the shape again.
                spec.args.validate(&args).map_err(|e| invalid(e.to_string()))?;
                let tctx = self.tool_ctx(ctx.peer(), &tool, data_dir);
                let result = t
                    .invoke(&op, args.into_inner(), &tctx)
                    .map(debrief_ipc::Json)
                    .map_err(|e| ToolFail {
                        reason: failure_reason(&e),
                        message: format!("{e:#}"),
                    });
                encode_reply(&result)
            }
            PluginRequest::StopTask { tool, task, data_dir } => {
                let (_, t) = self.tool(&tool)?;
                let tctx = self.tool_ctx(ctx.peer(), &tool, data_dir);
                let result = t.stop(&task, &tctx).map_err(|e| ToolFail {
                    reason: failure_reason(&e),
                    message: format!("{e:#}"),
                });
                encode_reply(&result)
            }
            PluginRequest::SniffProfile { importer, file_name, head } => {
                let i = self.profile_importer(&importer)?;
                encode_reply(&i.sniff(&file_name, &head).min(100))
            }
            PluginRequest::ImportProfile {
                importer,
                file_name,
                content,
                limits,
            } => {
                let i = self.profile_importer(&importer)?;
                let bytes = HostFile::new(ctx.peer(), content).read_all(MAX_IMPORT_BYTES).map_err(failed)?;
                let encoded = i.import_encoded(&file_name, &bytes, &limits.0);
                drop(bytes);
                for piece in encoded.chunks(READ_CHUNK as usize) {
                    ctx.send_item(piece.to_vec())?;
                }
                encode_reply(&ProfileDone {
                    bytes: encoded.len() as u64,
                    heap: i.heap(),
                })
            }
            PluginRequest::SniffDocument { importer, file_name, head } => {
                let i = self.document_importer(&importer)?;
                encode_reply(&i.sniff(&file_name, &head).min(100))
            }
            PluginRequest::ImportDocument {
                importer,
                file_name,
                content,
                options,
            } => {
                let i = self.document_importer(&importer)?;
                let file = HostFile::new(ctx.peer(), content);
                let encoded = i.import_encoded(&file_name, &file, &options.0);
                for piece in encoded.chunks(READ_CHUNK as usize) {
                    ctx.send_item(piece.to_vec())?;
                }
                encode_reply(&DocumentDone {
                    bytes: encoded.len() as u64,
                })
            }
        }
    }

    fn config_parser(&self, id: &str) -> Result<&Arc<dyn crate::config::ConfigParser>, RemoteError> {
        self.c
            .config_parsers
            .iter()
            .find(|p| p.id() == id)
            .ok_or_else(|| invalid(format!("no configuration parser \"{id}\"")))
    }

    fn profile_importer(&self, id: &str) -> Result<&Arc<dyn crate::plugin::ProfileImporter>, RemoteError> {
        self.c
            .profile_importers
            .iter()
            .find(|p| p.id() == id)
            .ok_or_else(|| invalid(format!("no profile importer \"{id}\"")))
    }

    fn document_importer(&self, id: &str) -> Result<&Arc<dyn crate::plugin::DocumentImporter>, RemoteError> {
        self.c
            .document_importers
            .iter()
            .find(|p| p.id() == id)
            .ok_or_else(|| invalid(format!("no document importer \"{id}\"")))
    }

    fn tool(&self, id: &str) -> Result<&(crate::tools::ToolInfo, Arc<dyn crate::tools::Tool>), RemoteError> {
        self.c
            .tools
            .iter()
            .find(|(i, _)| i.id == id)
            .ok_or_else(|| invalid(format!("no tool \"{id}\"")))
    }

    fn tool_ctx(&self, peer: &Peer, tool: &str, data_dir: Option<String>) -> ToolCtx {
        let ctx = ToolCtx::new(data_dir.map(PathBuf::from), Arc::new(HostTasks(peer.clone()))).with_paths(Arc::new(CheckedByHost));
        // The elevated process runs elevated operations itself; it never asks for another one. It never writes at
        // a path either: its output files are streamed to the host.
        let ctx = if self.elevated {
            ctx.with_outputs(Arc::new(crate::output::HostOutputs {
                peer: peer.clone(),
                tool: tool.to_string(),
            }))
        } else {
            ctx.with_elevated(Arc::new(HostElevated(peer.clone())))
        };
        ctx.for_tool(tool)
    }

    /// The plugin this handler serves.
    pub fn plugin(&self) -> &Arc<dyn Plugin> {
        &self.plugin
    }
}

impl Handler for PluginHandler {
    fn call(&self, ctx: &CallCtx, body: &[u8]) -> Result<Vec<u8>, RemoteError> {
        let req: PluginRequest = decode_request(body)?;
        self.dispatch(ctx, req)
    }
}

/// Task reports go to the host (also after the call that started the task returned).
struct HostTasks(Peer);

const REPORT_TIMEOUT: Option<Duration> = Some(Duration::from_secs(10));

impl TaskSink for HostTasks {
    fn started(&self, tool: &str, task: &ToolTask) {
        let _: Result<(), _> = self.0.request(
            &HostRequest::TaskStarted {
                tool: tool.into(),
                task: task.clone(),
            },
            REPORT_TIMEOUT,
        );
    }

    fn progress(&self, tool: &str, task: &str, fraction: Option<f32>) {
        let _: Result<(), _> = self.0.request(
            &HostRequest::TaskProgress {
                tool: tool.into(),
                task: task.into(),
                fraction,
            },
            REPORT_TIMEOUT,
        );
    }

    fn finished(&self, tool: &str, task: &str) {
        let _: Result<(), _> = self.0.request(
            &HostRequest::TaskFinished {
                tool: tool.into(),
                task: task.into(),
            },
            REPORT_TIMEOUT,
        );
    }
}

/// The plugin's elevated operations, run by the host in the plugin's elevated process.
struct HostElevated(Peer);

impl ElevatedOps for HostElevated {
    fn invoke(&self, tool: &str, op: &str, args: serde_json::Value, timeout: Duration) -> anyhow::Result<serde_json::Value> {
        let req = HostRequest::InvokeElevated {
            tool: tool.into(),
            op: op.into(),
            args: debrief_ipc::Json(args),
            timeout_ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
        };
        // The host may first show the administrator prompt: wait for the operation's time and the prompt's.
        let reply: Result<debrief_ipc::Json, ToolFail> = self
            .0
            .request(&req, Some(timeout + ELEVATION_PROMPT_WAIT))
            .map_err(|e| anyhow::anyhow!("{tool} {op} (elevated): {e}"))?;
        reply.map(debrief_ipc::Json::into_inner).map_err(|f| match f.reason {
            Some(FailureReason::Denied) => anyhow::Error::new(ToolFailure::denied(f.message)),
            Some(FailureReason::Missing) => anyhow::Error::new(ToolFailure::missing(f.message)),
            None => anyhow::anyhow!(f.message),
        })
    }
}

/// How long the user may take to answer the administrator prompt.
pub const ELEVATION_PROMPT_WAIT: Duration = Duration::from_secs(120);

/// Paths reach the plugin already checked by the host.
struct CheckedByHost;

impl PathPolicy for CheckedByHost {
    fn check(&self, path: &str, _: PathKind, _: PathAccess) -> std::result::Result<PathBuf, String> {
        Ok(Path::new(path).to_path_buf())
    }
}

/// An annotator's view: every call goes to the host under the annotate call's ticket.
struct RemoteView<'a> {
    peer: &'a Peer,
    ticket: u64,
}

const VIEW_TIMEOUT: Option<Duration> = Some(Duration::from_secs(120));

impl AnnotationView for RemoteView<'_> {
    fn select(&self, filter: &ViewFilter, limit: usize) -> Result<Vec<ViewRecord>> {
        let req = HostRequest::ViewSelect {
            ticket: self.ticket,
            filter: debrief_ipc::Json(serde_json::to_value(filter)?),
            limit: limit.min(crate::annotate::MAX_VIEW_RECORDS) as u32,
        };
        let rows: Result<Vec<WireViewRecord>, String> = self.peer.request(&req, VIEW_TIMEOUT).map_err(|e| anyhow!("{e}"))?;
        Ok(rows
            .map_err(|e| anyhow!(e))?
            .into_iter()
            .map(|r| ViewRecord {
                record: r.event.into_record(),
                source: r.source,
                instance: r.instance,
            })
            .collect())
    }

    fn instances(&self) -> Vec<String> {
        self.peer
            .request(&HostRequest::ViewInstances { ticket: self.ticket }, VIEW_TIMEOUT)
            .unwrap_or_default()
    }

    fn time_range(&self) -> Option<(i64, i64)> {
        self.peer
            .request(&HostRequest::ViewTimeRange { ticket: self.ticket }, VIEW_TIMEOUT)
            .ok()
            .flatten()
    }
}
