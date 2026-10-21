//! A plugin process as an ordinary [`imt_core::Plugin`]: [`RemotePlugin`] registers proxies of everything the
//! plugin's manifest declares (importers, configuration parsers, marker kinds, annotators, transforms, tools,
//! settings, profile importers), whose calls go to the process. The registry wraps them in the same guarded
//! adapters as any contribution, so quarantine, status, settings and the UI see no difference.
//!
//! Every call has a deadline just under its watchdog budget ([`deadline_for`]). A crash, a missed deadline,
//! garbage on the wire or a panic in the plugin's code counts as a failure of the plugin (towards its quarantine)
//! and stops its process; the supervisor starts a fresh one, with backoff, on the next call. A typed error of the
//! plugin's code is just an error.

use crate::process::{Lent, PluginProcess};
use anyhow::{anyhow, Result};
use imt_core::config::{ConfigError, ConfigParser, ConfigTree, Limits, PARSERS_POINT};
use imt_core::guard::{CallKind, Health};
use imt_core::importers::{BinaryImporter, ImportCtx, Imported, Importer};
use imt_core::model::Record;
use imt_core::plugin::annotate::{Annotation, AnnotationView, Annotator, RecordCursor};
use imt_core::plugin::tools::{Tool, ToolCtx, ToolFailure, ToolInfo};
use imt_core::plugin::{Manifest as CoreManifest, RecordTransform, TransformCtx, Verdict};
use imt_core::query::{FieldFilter, Query};
use imt_core::settings::{Configurable, SettingsSection, ValidValues};
use imt_core::{Plugin, PluginRegistry};
use imt_ipc::peer::code;
use imt_ipc::{IpcError, Json};
use imt_plugin_api::annotate as api_annotate;
use imt_plugin_api::importer::path_tail;
use imt_plugin_api::manifest::{ConfigParserCap, ImporterCap, Permission};
use imt_plugin_api::problems::Problems;
use imt_plugin_api::protocol::*;
use std::sync::Arc;
use std::time::Duration;

/// The deadline of a call whose watchdog budget is `budget`: 90% of it, so the call fails (and the process is
/// stopped) before the watchdog marks the plugin hung for the same call.
pub fn deadline_for(budget: Duration) -> Duration {
    budget.mul_f64(0.9)
}

/// Settings and sniffing a configuration have no watchdog budget in the engine; out of process they get this.
const DEFAULT_DEADLINE: Duration = Duration::from_secs(10);

/// A plugin process registered like an in-process plugin.
pub struct RemotePlugin {
    process: Arc<PluginProcess>,
}

impl RemotePlugin {
    pub fn new(process: Arc<PluginProcess>) -> RemotePlugin {
        RemotePlugin { process }
    }

    pub fn process(&self) -> &Arc<PluginProcess> {
        &self.process
    }
}

/// The process plus the health ledger of the registry a proxy was registered in.
#[derive(Clone)]
struct Ctx {
    process: Arc<PluginProcess>,
    health: Arc<Health>,
}

impl Ctx {
    fn id(&self) -> &str {
        self.process.id()
    }

    fn deadline(&self, kind: CallKind) -> Option<Duration> {
        self.health.budgets().get(kind).map(deadline_for)
    }

    /// Turns a failed call into the plugin's error; process failures and panics count against the plugin.
    fn fail(&self, what: &str, timeout: Option<Duration>, e: IpcError) -> anyhow::Error {
        let id = self.id();
        match e {
            IpcError::Remote(r) if r.code != code::PANIC => anyhow!("plugin \"{id}\" ({what}): {}", r.message),
            IpcError::Denied(m) => anyhow::Error::new(ToolFailure::denied(m)),
            e => {
                let crash = match &e {
                    IpcError::Died(m) => Some(imt_core::guard::Crashed::new(format!("plugin \"{id}\" ({what})"), m.clone())),
                    IpcError::Remote(r) => Some(imt_core::guard::Crashed::new(
                        format!("plugin \"{id}\" ({what})"),
                        r.message.clone(),
                    )),
                    _ => None,
                };
                let message = match &e {
                    IpcError::Timeout => format!(
                        "plugin \"{id}\" ({what}) did not finish within {}; its process was stopped",
                        timeout.map_or("its deadline".into(), |t| format!("{:.1} s", t.as_secs_f64()))
                    ),
                    IpcError::Died(m) => format!("plugin \"{id}\" ({what}) crashed: {m}"),
                    IpcError::Remote(r) => format!("plugin \"{id}\" ({what}) crashed: {}", r.message),
                    other => format!("plugin \"{id}\" ({what}) failed: {other}"),
                };
                // A panic inside the process may have left it inconsistent: it is restarted on the next call.
                if matches!(e, IpcError::Remote(_)) {
                    self.process.shutdown();
                }
                self.health.fail(id, &message);
                match crash {
                    Some(c) => anyhow::Error::new(c),
                    None => anyhow!(message),
                }
            }
        }
    }

    fn call<Rep: serde::de::DeserializeOwned>(&self, what: &str, req: &PluginRequest, timeout: Option<Duration>) -> Result<Rep> {
        self.process.call(req, timeout, false).map_err(|e| self.fail(what, timeout, e))
    }

    /// What an importer learns about the file: its name, the tail of its path (the full path only with
    /// [`Permission::OpenedFilePaths`]), its date and the profile's timestamp settings.
    fn file_info(&self, ctx: &ImportCtx) -> FileInfo {
        let parse = &ctx.profile.profile.parse;
        FileInfo {
            name: ctx.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            path: if self.process.granted(Permission::OpenedFilePaths) {
                ctx.path.to_string_lossy().into_owned()
            } else {
                path_tail(ctx.path).to_string_lossy().into_owned()
            },
            date: ctx.file_date.map(|d| d.format("%Y-%m-%d").to_string()),
            timestamps: TimestampSettings {
                formats: parse.timestamp_formats.clone(),
                zone: parse.assume_timezone.clone(),
                day_first: ctx.profile.timestamps.day_first(),
            },
        }
    }

    /// Runs an import-like call lending `files`; collects the streamed events.
    fn import_call<Rep: serde::de::DeserializeOwned>(
        &self,
        what: &str,
        kind: CallKind,
        files: Vec<&[u8]>,
        make: impl FnOnce(&[HostFileRef]) -> PluginRequest,
    ) -> Result<(Rep, Vec<Record>)> {
        let timeout = self.deadline(kind);
        let mut records = Vec::new();
        let lent = Lent { files, ..Lent::default() };
        let rep = self
            .process
            .call_lending(
                |refs, _| make(refs),
                &lent,
                timeout,
                |item| {
                    let events: Vec<WireEvent> = imt_ipc::wire::decode(&item).map_err(IpcError::Protocol)?;
                    records.extend(events.into_iter().map(WireEvent::into_record));
                    Ok(())
                },
            )
            .map_err(|e| self.fail(what, timeout, e))?;
        Ok((rep, records))
    }
}

struct RemoteImporter {
    ctx: Ctx,
    cap: ImporterCap,
}

impl Importer for RemoteImporter {
    fn id(&self) -> &str {
        &self.cap.id
    }

    fn sniff(&self, ctx: &ImportCtx, head: &str) -> u8 {
        let req = PluginRequest::Sniff {
            importer: self.cap.id.clone(),
            file: self.ctx.file_info(ctx),
            head: head.as_bytes().to_vec(),
        };
        self.ctx.call("sniff", &req, self.ctx.deadline(CallKind::Sniff)).unwrap_or(0)
    }

    fn import(&self, ctx: &ImportCtx, content: &str) -> Result<Imported> {
        let file = self.ctx.file_info(ctx);
        let importer = self.cap.id.clone();
        let (done, records): (ImportDone, _) =
            self.ctx
                .import_call("import", CallKind::Import, vec![content.as_bytes()], |refs| PluginRequest::Import {
                    importer,
                    file,
                    content: refs[0],
                })?;
        Ok(Imported {
            records,
            skipped: done.skipped as usize,
            problems: Problems::from_vec(done.problems),
        })
    }

    fn import_tail(&self, ctx: &ImportCtx, head: &str, tail: &str) -> Option<Result<Imported>> {
        if !self.cap.tail {
            return None;
        }
        let file = self.ctx.file_info(ctx);
        let importer = self.cap.id.clone();
        let r: Result<(TailDone, Vec<Record>)> =
            self.ctx
                .import_call("import_tail", CallKind::Tail, vec![head.as_bytes(), tail.as_bytes()], |refs| {
                    PluginRequest::ImportTail {
                        importer,
                        file,
                        head: refs[0],
                        tail: refs[1],
                    }
                });
        match r {
            Ok((done, _)) if !done.supported => None,
            Ok((done, records)) => Some(Ok(Imported {
                records,
                skipped: done.skipped as usize,
                problems: Problems::from_vec(done.problems),
            })),
            Err(e) => Some(Err(e)),
        }
    }

    fn multiline(&self) -> bool {
        self.cap.multiline
    }

    fn identity_fields(&self) -> Vec<String> {
        self.cap.identity_fields.clone()
    }
}

struct RemoteBinaryImporter {
    ctx: Ctx,
    cap: ImporterCap,
}

impl BinaryImporter for RemoteBinaryImporter {
    fn id(&self) -> &str {
        &self.cap.id
    }

    fn identity_fields(&self) -> Vec<String> {
        self.cap.identity_fields.clone()
    }

    fn sniff(&self, ctx: &ImportCtx, head: &[u8]) -> u8 {
        let req = PluginRequest::Sniff {
            importer: self.cap.id.clone(),
            file: self.ctx.file_info(ctx),
            head: head.to_vec(),
        };
        self.ctx.call("sniff", &req, self.ctx.deadline(CallKind::Sniff)).unwrap_or(0)
    }

    fn import(&self, ctx: &ImportCtx, bytes: &[u8]) -> Result<Imported> {
        let file = self.ctx.file_info(ctx);
        let importer = self.cap.id.clone();
        let (done, records): (ImportDone, _) =
            self.ctx
                .import_call("import", CallKind::Import, vec![bytes], |refs| PluginRequest::Import {
                    importer,
                    file,
                    content: refs[0],
                })?;
        Ok(Imported {
            records,
            skipped: done.skipped as usize,
            problems: Problems::from_vec(done.problems),
        })
    }
}

struct RemoteConfigParser {
    ctx: Ctx,
    cap: ConfigParserCap,
}

impl ConfigParser for RemoteConfigParser {
    fn id(&self) -> &str {
        &self.cap.id
    }

    fn name(&self) -> &str {
        &self.cap.name
    }

    fn extensions(&self) -> Vec<String> {
        self.cap.extensions.clone()
    }

    fn sniff(&self, file_name: &str, head: &str) -> u8 {
        let req = PluginRequest::SniffConfig {
            parser: self.cap.id.clone(),
            file_name: file_name.into(),
            head: head.into(),
        };
        self.ctx.call("sniff", &req, Some(DEFAULT_DEADLINE)).unwrap_or(0)
    }

    fn parse(&self, content: &str, limits: &Limits) -> Result<ConfigTree, ConfigError> {
        let parser = self.cap.id.clone();
        let limits = *limits;
        let lent = Lent {
            files: vec![content.as_bytes()],
            ..Lent::default()
        };
        let timeout = self.ctx.deadline(CallKind::Import);
        self.ctx
            .process
            .call_lending::<Result<ConfigTree, ConfigError>>(
                |refs, _| PluginRequest::ParseConfig {
                    parser,
                    content: refs[0],
                    limits,
                },
                &lent,
                timeout,
                |_| Err(IpcError::Protocol("unexpected item".into())),
            )
            .map_err(|e| ConfigError::Plugin(format!("{:#}", self.ctx.fail("parse", timeout, e))))?
    }
}

struct RemoteAnnotator {
    ctx: Ctx,
    id: Arc<str>,
}

fn evidence_query(e: api_annotate::Evidence) -> Query {
    Query {
        text: e.text,
        from: e.from,
        to: e.to,
        filters: e
            .filters
            .into_iter()
            .map(|f| FieldFilter {
                field: f.field,
                op: f.op,
                value: f.value,
            })
            .collect(),
        ..Query::default()
    }
}

fn severity(s: api_annotate::Severity) -> imt_core::api::Severity {
    match s {
        api_annotate::Severity::Critical => imt_core::api::Severity::Critical,
        api_annotate::Severity::Warning => imt_core::api::Severity::Warning,
        api_annotate::Severity::Info => imt_core::api::Severity::Info,
    }
}

impl Annotator for RemoteAnnotator {
    fn id(&self) -> &str {
        &self.id
    }

    fn annotate(&self, view: &dyn AnnotationView, since: Option<RecordCursor>) -> Result<Vec<Annotation>> {
        let timeout = self.ctx.deadline(CallKind::Annotate);
        let lent = Lent {
            view: Some(view),
            ..Lent::default()
        };
        let annotator = self.id.clone();
        let list: Vec<api_annotate::Annotation> = self
            .ctx
            .process
            .call_lending(
                |_, ticket| PluginRequest::Annotate { annotator, ticket, since },
                &lent,
                timeout,
                |_| Err(IpcError::Protocol("unexpected item".into())),
            )
            .map_err(|e| self.ctx.fail("annotate", timeout, e))?;
        Ok(list
            .into_iter()
            .map(|a| Annotation {
                kind: a.kind,
                from: a.from,
                to: a.to,
                instance: a.instance,
                on: a.on,
                label: a.label,
                severity: severity(a.severity),
                evidence_query: evidence_query(a.evidence),
            })
            .collect())
    }
}

struct RemoteTransform {
    ctx: Ctx,
    id: Arc<str>,
}

impl RecordTransform for RemoteTransform {
    fn id(&self) -> &str {
        &self.id
    }

    fn apply(&self, record: &mut Record, ctx: &TransformCtx) -> Verdict {
        self.apply_batch(std::slice::from_mut(record), ctx)
            .first()
            .copied()
            .unwrap_or(Verdict::Keep)
    }

    fn batched(&self) -> bool {
        true
    }

    fn apply_batch(&self, records: &mut [Record], ctx: &TransformCtx) -> Vec<Verdict> {
        let req = PluginRequest::Transform {
            transform: self.id.clone(),
            source_name: ctx.source_name.into(),
            rows: records.iter().map(WireEvent::of).collect(),
        };
        // The budget is per record; a batch gets it per row, within reason.
        let timeout = self
            .ctx
            .deadline(CallKind::Transform)
            .map(|t| (t * records.len().max(1) as u32).min(Duration::from_secs(120)));
        let out: TransformOut = match self.ctx.call("transform", &req, timeout) {
            Ok(o) => o,
            // A failed step keeps every record unchanged (the failure is counted and noted on the plugin).
            Err(_) => return vec![Verdict::Keep; records.len()],
        };
        if out.rows.len() != records.len() {
            self.ctx.health.note(
                self.ctx.id(),
                "a transform returned a different number of records; they were kept unchanged",
            );
            return vec![Verdict::Keep; records.len()];
        }
        for (r, w) in records.iter_mut().zip(out.rows) {
            let (id, source, markers) = (r.id, r.source, r.markers);
            *r = w.into_record();
            (r.id, r.source, r.markers) = (id, source, markers);
        }
        for (row, kind) in out.marks {
            if let Some(r) = records.get_mut(row as usize) {
                ctx.mark_record(r, &kind);
            }
        }
        out.keep
            .iter()
            .map(|k| if *k { Verdict::Keep } else { Verdict::Drop })
            .chain(std::iter::repeat(Verdict::Keep))
            .take(records.len())
            .collect()
    }
}

struct RemoteTool {
    ctx: Ctx,
    info: ToolInfo,
}

impl Tool for RemoteTool {
    fn id(&self) -> &str {
        &self.info.id
    }

    fn describe(&self) -> ToolInfo {
        self.info.clone()
    }

    fn invoke(&self, op: &str, args: serde_json::Value, ctx: &ToolCtx) -> Result<serde_json::Value> {
        let spec = self.info.ops.iter().find(|o| o.name == op);
        let elevated = spec.is_some_and(|s| s.elevated);
        let timeout = spec.and_then(|s| s.budget).map(deadline_for);
        self.ctx.process.services().remember_tool(&self.info.id, ctx);
        let req = PluginRequest::InvokeTool {
            tool: self.info.id.as_str().into(),
            op: op.into(),
            args: Json(args),
            data_dir: self.ctx.process.tool_data_dir(ctx.data_dir()),
        };
        let r: Result<Json, ToolFail> = self
            .ctx
            .process
            .call(&req, timeout, elevated)
            .map_err(|e| self.ctx.fail(&format!("tool {} {op}", self.info.id), timeout, e))?;
        r.map(Json::into_inner).map_err(tool_error)
    }

    fn stop(&self, task: &str, ctx: &ToolCtx) -> Result<()> {
        let req = PluginRequest::StopTask {
            tool: self.info.id.as_str().into(),
            task: task.into(),
            data_dir: self.ctx.process.tool_data_dir(ctx.data_dir()),
        };
        // A task of an elevated operation is stopped in the elevated process.
        let elevated = self.info.ops.iter().any(|o| o.elevated) && self.info.ops.iter().all(|o| o.elevated);
        let timeout = Some(deadline_for(imt_core::plugin::tools::DEFAULT_OP_BUDGET));
        let r: Result<(), ToolFail> = self
            .ctx
            .process
            .call(&req, timeout, elevated)
            .map_err(|e| self.ctx.fail(&format!("tool {} stop", self.info.id), timeout, e))?;
        r.map_err(tool_error)
    }
}

fn tool_error(f: ToolFail) -> anyhow::Error {
    use imt_core::plugin::tools::FailureReason;
    match f.reason {
        Some(FailureReason::Denied) => anyhow::Error::new(ToolFailure::denied(f.message)),
        Some(FailureReason::Missing) => anyhow::Error::new(ToolFailure::missing(f.message)),
        None => anyhow!(f.message),
    }
}

struct RemoteSettings {
    ctx: Ctx,
    section: SettingsSection,
}

impl Configurable for RemoteSettings {
    fn settings_section(&self) -> SettingsSection {
        self.section.clone()
    }

    fn apply_settings(&self, values: &ValidValues) -> Result<()> {
        let json = values.to_json();
        self.ctx.process.remember_settings(json.clone());
        self.ctx.call::<()>(
            "settings",
            &PluginRequest::ApplySettings { values: Json(json) },
            Some(DEFAULT_DEADLINE),
        )
    }
}

impl Plugin for RemotePlugin {
    fn id(&self) -> &str {
        self.process.id()
    }

    fn name(&self) -> &str {
        &self.process.manifest().name
    }

    fn version(&self) -> &str {
        &self.process.manifest().version
    }

    /// Plugins have no dependencies on each other (shared code is a library, ADR 0017 §4).
    fn manifest(&self) -> CoreManifest {
        CoreManifest::new(self.id(), self.version())
    }

    fn register(&self, registry: &mut PluginRegistry) -> Result<()> {
        let ctx = Ctx {
            process: self.process.clone(),
            health: registry.health(),
        };
        let m = self.process.manifest().clone();
        for cap in &m.importers {
            if cap.binary {
                registry.add_binary_importer(Box::new(RemoteBinaryImporter {
                    ctx: ctx.clone(),
                    cap: cap.clone(),
                }));
            } else {
                registry.add_importer(Box::new(RemoteImporter {
                    ctx: ctx.clone(),
                    cap: cap.clone(),
                }));
            }
        }
        for cap in &m.config_parsers {
            registry.contribute::<dyn ConfigParser>(
                PARSERS_POINT,
                Arc::new(RemoteConfigParser {
                    ctx: ctx.clone(),
                    cap: cap.clone(),
                }),
            );
        }
        if !m.marker_kinds.is_empty() {
            registry.add_marker_kinds(m.marker_kinds.clone())?;
        }
        for id in &m.annotators {
            registry.add_annotator(Arc::new(RemoteAnnotator {
                ctx: ctx.clone(),
                id: id.clone(),
            }))?;
        }
        for id in &m.transforms {
            registry.add_transform(Arc::new(RemoteTransform {
                ctx: ctx.clone(),
                id: id.clone(),
            }))?;
        }
        for info in &m.tools {
            registry.add_tool(Arc::new(RemoteTool {
                ctx: ctx.clone(),
                info: info.clone(),
            }))?;
        }
        if !m.profile_importers.is_empty() {
            crate::profile::register(registry, &self.process, &ctx.health)?;
        }
        if !m.document_importers.is_empty() {
            crate::document::register(registry, &self.process, &ctx.health)?;
        }
        Ok(())
    }

    fn configurable(&self) -> Option<Arc<dyn Configurable>> {
        let section = self.process.manifest().settings.clone()?;
        Some(Arc::new(RemoteSettings {
            ctx: Ctx {
                process: self.process.clone(),
                // Settings calls are not counted in a window's health (they apply app-wide).
                health: Arc::new(Health::default()),
            },
            section,
        }))
    }
}
