//! Serving a plugin over `imt.plugin.v1`. The plugin is installed into a registry of this process (so its
//! contributions are wrapped in the same guarded adapters as in IMT: a panic becomes an error), and every
//! call is answered from that registry. Plugin code runs on blocking threads, never on the async runtime.
//!
//! Budgets are not enforced in here: IMT puts a deadline on every call and kills the whole process when
//! one is missed, which (unlike an in-process watchdog) really stops the stuck code.

use crate::arrow::{self, batch_ranges, record_size};
use crate::convert::{annotation_to, condition_to, marker_kind_to, permission_to, section_to, Permission, PermissionRequest};
use crate::{code, pb, CHUNK_BYTES, CONTRACT_MAJOR, CONTRACT_MINOR, MAX_IMPORT_BYTES, MAX_MESSAGE_BYTES};
use anyhow::anyhow;
use bytes::Bytes;
use imt_core::condition::{Condition, ConditionCfg};
use imt_core::export::ExportRow;
use imt_core::guard::Budgets;
use imt_core::importers::{ImportCtx, Imported};
use imt_core::model::Record;
use imt_core::plugin::annotate::{AnnotationView, RecordCursor, ViewFilter, ViewRecord, MAX_VIEW_RECORDS};
use imt_core::plugin::markers::MarkIndex;
use imt_core::plugin::{TransformCtx, Verdict};
use imt_core::profile::{CompiledProfile, Profile};
use imt_core::query::Query;
use imt_core::{Plugin, PluginRegistry};
use imt_ipc::RemoteError;
use std::collections::HashSet;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::{Stream, StreamExt};
use tonic::{Request, Response, Status, Streaming};

/// A service a plugin offers other plugins, with opaque payloads (typed by its API crate).
pub trait ByteService: Send + Sync {
    fn call(&self, method: &str, payload: &[u8]) -> anyhow::Result<Vec<u8>>;
}

/// What a plugin declares beyond its [`Plugin`] contributions.
#[derive(Default, Clone)]
pub struct ServeOptions {
    /// Permissions the plugin asks the user for.
    pub permissions: Vec<PermissionRequest>,
    /// Records each annotator wants (by annotator id); annotators not listed get every record (up to
    /// IMT's view limit).
    pub annotator_interest: Vec<(String, Vec<ConditionCfg>, usize)>,
    /// Services, by id and version.
    pub services: Vec<(String, String, Arc<dyn ByteService>)>,
    /// Event kinds to receive: `recordsAppended`, `profileChanged`, `workspaceCleared`.
    pub events: Vec<String>,
}

/// Serves `plugin` as this process' plugin (see the crate docs) until IMT closes stdin.
pub fn serve(plugin: Arc<dyn Plugin>) -> std::io::Result<()> {
    serve_with(plugin, ServeOptions::default())
}

/// [`serve`] with permissions, annotator interest, services and events.
pub fn serve_with(plugin: Arc<dyn Plugin>, opts: ServeOptions) -> std::io::Result<()> {
    let name = format!("{} {}", plugin.id(), plugin.version());
    let service = PluginService::new(plugin, opts).map_err(|e| std::io::Error::other(format!("{e:#}")))?;
    let serve_opts = imt_ipc::server::ServeOptions {
        name,
        max_message_bytes: MAX_MESSAGE_BYTES,
    };
    imt_ipc::server::child_main(imt_ipc::server::RuntimeKind::MultiThread, serve_opts, move |_| service.routes())
}

fn fail(c: i32, message: impl Into<String>) -> Status {
    let grpc = match c {
        code::INVALID | code::NO_HANDSHAKE => tonic::Code::InvalidArgument,
        code::UNIMPLEMENTED => tonic::Code::Unimplemented,
        code::VERSION => tonic::Code::FailedPrecondition,
        code::TOO_LARGE => tonic::Code::ResourceExhausted,
        code::CRASHED => tonic::Code::Internal,
        _ => tonic::Code::Aborted,
    };
    imt_ipc::error::to_status(grpc, &RemoteError::new(c, message))
}

fn invalid(message: impl Into<String>) -> Status {
    fail(code::INVALID, message)
}

/// The installed plugin and everything calls share.
struct Hosted {
    id: &'static str,
    plugin: Arc<dyn Plugin>,
    registry: PluginRegistry,
    opts: ServeOptions,
    handshaken: AtomicBool,
    granted: Mutex<Vec<Permission>>,
    profile: Mutex<Option<(String, Arc<CompiledProfile>)>>,
    /// A panic the guard turned into a fallback value during the last quiet call.
    last_crash: Mutex<Option<String>>,
    marks: MarkIndex,
    mark_names: Vec<String>,
}

impl Hosted {
    fn gate(&self) -> Result<(), Status> {
        if self.handshaken.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(fail(code::NO_HANDSHAKE, "call Handshake first"))
        }
    }

    fn failures(&self) -> u32 {
        self.registry.health().get(self.id).failures
    }

    /// After a crash counted by the guard: its message, and the plugin's health reset. IMT counts the
    /// crash (and restarts this process); a quarantine in here would only hide later crashes from it.
    fn take_crash(&self) -> String {
        let message = self
            .registry
            .health()
            .get(self.id)
            .last_error
            .unwrap_or_else(|| "the plugin crashed".into());
        self.registry.reset_plugin(self.id);
        message
    }

    /// Runs plugin code: an error is `FAILED`; a panic (counted by the guard) is `CRASHED`.
    fn run<T>(&self, f: impl FnOnce() -> anyhow::Result<T>) -> Result<T, Status> {
        let before = self.failures();
        let r = f();
        let crashed = self.failures() > before;
        match r {
            Err(e) => {
                if crashed {
                    self.take_crash();
                }
                Err(fail(if crashed { code::CRASHED } else { code::FAILED }, format!("{e:#}")))
            }
            Ok(v) => {
                if crashed {
                    let last = self.take_crash();
                    *self.last_crash.lock().unwrap_or_else(|p| p.into_inner()) = Some(last);
                }
                Ok(v)
            }
        }
    }

    /// Notes (refused marks, ...) recorded since `seen`, to pass on to IMT.
    fn new_notes(&self, seen: &[String]) -> Vec<String> {
        self.registry
            .health()
            .notes(self.id)
            .into_iter()
            .filter(|n| !seen.contains(n))
            .collect()
    }

    /// After a call whose panic the guard hid behind a fallback value: a failure counted during the call
    /// (see [`Hosted::run`], which passes it through as `Ok`) becomes `CRASHED`.
    fn crashed_quietly<T>(&self, v: T) -> Result<T, Status> {
        let note = self.last_crash.lock().unwrap_or_else(|p| p.into_inner()).take();
        match note {
            Some(message) => Err(fail(code::CRASHED, message)),
            None => Ok(v),
        }
    }

    fn compiled(&self, json: &str) -> Result<Arc<CompiledProfile>, Status> {
        let mut cache = self.profile.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((j, c)) = cache.as_ref() {
            if j == json {
                return Ok(c.clone());
            }
        }
        let profile: Profile = if json.is_empty() {
            Profile::default_profile()
        } else {
            serde_json::from_str(json).map_err(|e| invalid(format!("the profile is not valid: {e}")))?
        };
        let compiled = Arc::new(CompiledProfile::compile(profile).map_err(|e| invalid(format!("the profile does not compile: {e:#}")))?);
        *cache = Some((json.to_string(), compiled.clone()));
        Ok(compiled)
    }

    /// The import context: the path is the file name only, unless the plugin may read opened files.
    fn context(&self, ctx: Option<&pb::ImportContext>) -> Result<(Arc<CompiledProfile>, PathBuf, Option<chrono::NaiveDate>), Status> {
        let ctx = ctx.ok_or_else(|| invalid("the import context is missing"))?;
        let may_read = self
            .granted
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains(&Permission::ReadOpenedFiles);
        let path = if may_read && !ctx.path.is_empty() {
            PathBuf::from(&ctx.path)
        } else {
            PathBuf::from(&ctx.file_name)
        };
        let date = chrono::NaiveDate::parse_from_str(&ctx.file_date, "%Y-%m-%d").ok();
        Ok((self.compiled(&ctx.profile_json)?, path, date))
    }

    fn capabilities(&self) -> Vec<pb::Capability> {
        use pb::capability::Kind;
        let mut caps = Vec::new();
        for i in self.registry.hosted_importers() {
            caps.push(Kind::Importer(pb::ImporterCap {
                id: i.id().into(),
                binary: false,
                tail: true,
                multiline: i.multiline(),
                identity_fields: i.identity_fields(),
            }));
        }
        for i in self.registry.hosted_binary_importers() {
            caps.push(Kind::Importer(pb::ImporterCap {
                id: i.id().into(),
                binary: true,
                tail: false,
                multiline: false,
                identity_fields: i.identity_fields(),
            }));
        }
        for t in self.registry.hosted_transforms() {
            caps.push(Kind::Transform(pb::TransformCap { id: t.id().into() }));
        }
        for e in self.registry.hosted_exporters() {
            caps.push(Kind::Exporter(pb::ExporterCap {
                id: e.id().into(),
                name: e.name().into(),
                extension: e.extension().into(),
            }));
        }
        if !self.mark_names.is_empty() {
            caps.push(Kind::MarkerKinds(pb::MarkerKindsCap {}));
        }
        for a in self.registry.hosted_annotators() {
            let (interest, limit) = self
                .opts
                .annotator_interest
                .iter()
                .find(|(id, _, _)| id == a.id())
                .map(|(_, w, l)| (w.iter().map(condition_to).collect(), *l as u32))
                .unwrap_or_default();
            caps.push(Kind::Annotator(pb::AnnotatorCap {
                id: a.id().into(),
                interest,
                limit,
            }));
        }
        if self.registry.hosted_settings(self.id).is_some() {
            caps.push(Kind::Settings(pb::SettingsCap {}));
        }
        for (id, version, _) in &self.opts.services {
            caps.push(Kind::Service(pb::ServiceCap {
                id: id.clone(),
                version: version.clone(),
            }));
        }
        if !self.opts.events.is_empty() {
            caps.push(Kind::Events(pb::EventsCap {
                kinds: self.opts.events.clone(),
            }));
        }
        caps.into_iter().map(|k| pb::Capability { kind: Some(k) }).collect()
    }
}

/// The `imt.plugin.v1.Plugin` service over one installed plugin.
pub struct PluginService {
    hosted: Arc<Hosted>,
}

impl PluginService {
    /// Installs `plugin` into a registry of this process. Fails when it does not register or is inactive
    /// (e.g. it needs another plugin, which a plugin process does not have).
    pub fn new(plugin: Arc<dyn Plugin>, opts: ServeOptions) -> anyhow::Result<PluginService> {
        let id = plugin.id();
        let mut registry = PluginRegistry::empty();
        registry.set_budgets(Budgets::unlimited());
        registry.install_shared(plugin.clone())?;
        registry.resolve();
        if let Some(reason) = registry.plugins().into_iter().find(|p| p.id == id).and_then(|p| p.inactive_reason) {
            return Err(anyhow!("plugin \"{id}\" is inactive: {reason}"));
        }
        let mark_names: Vec<String> = registry.hosted_marker_kinds(id).into_iter().map(|k| k.kind).collect();
        Ok(PluginService {
            hosted: Arc::new(Hosted {
                id,
                plugin,
                marks: MarkIndex::of_kinds(id, &mark_names),
                mark_names,
                registry,
                opts,
                handshaken: AtomicBool::new(false),
                granted: Mutex::new(Vec::new()),
                profile: Mutex::new(None),
                last_crash: Mutex::new(None),
            }),
        })
    }

    pub fn routes(self) -> tonic::service::Routes {
        tonic::service::Routes::new(
            pb::plugin_server::PluginServer::new(self)
                .max_decoding_message_size(MAX_MESSAGE_BYTES)
                .max_encoding_message_size(MAX_MESSAGE_BYTES),
        )
    }
}

type Out<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send>>;

/// Runs `f` on a blocking thread; a join failure (a panic outside the guard) is `CRASHED`.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, Status> + Send + 'static) -> Result<T, Status> {
    tokio::task::spawn_blocking(f)
        .await
        .unwrap_or_else(|e| Err(fail(code::CRASHED, format!("the plugin crashed: {e}"))))
}

/// Reads an import request: the start message and the content (or head and tail).
async fn read_import(mut s: Streaming<pb::ImportChunk>) -> Result<(pb::ImportStart, Vec<u8>, Vec<u8>), Status> {
    let start = match s.next().await {
        Some(Ok(pb::ImportChunk {
            item: Some(pb::import_chunk::Item::Start(st)),
        })) => st,
        Some(Err(e)) => return Err(e),
        _ => return Err(invalid("an import starts with ImportStart")),
    };
    if start.total_bytes > MAX_IMPORT_BYTES {
        return Err(fail(
            code::TOO_LARGE,
            format!("the content is larger than {MAX_IMPORT_BYTES} bytes"),
        ));
    }
    let mut head = Vec::new();
    let mut body = Vec::with_capacity(start.total_bytes.min(256 << 20) as usize);
    let mut total = 0u64;
    while let Some(m) = s.next().await {
        let chunk = match m?.item {
            Some(pb::import_chunk::Item::Chunk(c)) => c,
            _ => return Err(invalid("only chunks may follow ImportStart")),
        };
        total += chunk.data.len() as u64;
        if total > MAX_IMPORT_BYTES {
            return Err(fail(
                code::TOO_LARGE,
                format!("the content is larger than {MAX_IMPORT_BYTES} bytes"),
            ));
        }
        match pb::ChunkPart::try_from(chunk.part) {
            Ok(pb::ChunkPart::Head) => head.extend_from_slice(&chunk.data),
            _ => body.extend_from_slice(&chunk.data),
        }
    }
    Ok((start, head, body))
}

fn text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

/// Streams imported records as batches, then the done message.
fn send_imported(imported: Imported, batch_rows: u32, tx: &mpsc::Sender<Result<pb::ImportReply, Status>>) {
    let records = imported.records;
    let rows = match batch_rows as usize {
        0 => arrow::DEFAULT_BATCH_ROWS,
        n => n.min(arrow::MAX_BATCH_ROWS),
    };
    let ranges = batch_ranges(records.len(), rows, |i| record_size(&records[i]));
    let mut failed = false;
    arrow::encode_batches(&records, &ranges, arrow::parallelism(), |bytes, rows| {
        let msg = match bytes {
            Ok(arrow) => Ok(pb::ImportReply {
                item: Some(pb::import_reply::Item::Batch(pb::RecordBatch { arrow, rows })),
            }),
            Err(e) => Err(fail(code::FAILED, format!("cannot encode the records: {e:#}"))),
        };
        failed = msg.is_err();
        tx.blocking_send(msg).is_ok() && !failed
    });
    if failed {
        return;
    }
    let _ = tx.blocking_send(Ok(pb::ImportReply {
        item: Some(pb::import_reply::Item::Done(pb::ImportDone {
            skipped: imported.skipped as u64,
            tail_unsupported: false,
        })),
    }));
}

/// An annotation view over the records IMT sent.
struct BatchView {
    records: Vec<ViewRecord>,
    instances: Vec<String>,
    range: Option<(i64, i64)>,
}

impl AnnotationView for BatchView {
    fn select(&self, filter: &ViewFilter, limit: usize) -> anyhow::Result<Vec<ViewRecord>> {
        let conditions = filter.when.iter().map(Condition::compile).collect::<anyhow::Result<Vec<_>>>()?;
        let ok = |v: &&ViewRecord| {
            let ts = v.record.ts;
            filter.from.is_none_or(|f| ts.is_some_and(|t| t >= f))
                && filter.to.is_none_or(|to| ts.is_some_and(|t| t <= to))
                && conditions.iter().all(|c| c.eval(&v.record, &v.source))
        };
        let limit = limit.min(MAX_VIEW_RECORDS);
        Ok(if filter.newest_first {
            self.records.iter().rev().filter(ok).take(limit).cloned().collect()
        } else {
            self.records.iter().filter(ok).take(limit).cloned().collect()
        })
    }

    fn query(&self, _query: &Query, _limit: usize) -> anyhow::Result<Vec<ViewRecord>> {
        Err(anyhow!(
            "queries are not available to an annotator running out of process; declare the records it needs (AnnotatorCap.interest) and use select"
        ))
    }

    fn instances(&self) -> Vec<String> {
        self.instances.clone()
    }

    fn time_range(&self) -> Option<(i64, i64)> {
        self.range
    }
}

/// Source names of exported rows live for the whole process (an export streams rows that borrow them).
fn interned(name: &str) -> &'static str {
    static NAMES: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let mut names = NAMES.get_or_init(Default::default).lock().unwrap_or_else(|p| p.into_inner());
    if let Some(n) = names.get(name) {
        return n;
    }
    let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
    names.insert(leaked);
    leaked
}

/// Collects an exporter's output into chunks for the reply stream.
struct ChunkWriter {
    tx: mpsc::Sender<Result<pb::ExportOut, Status>>,
    buf: Vec<u8>,
}

impl ChunkWriter {
    fn send(&mut self) -> std::io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let data = Bytes::from(std::mem::take(&mut self.buf));
        self.tx
            .blocking_send(Ok(pb::ExportOut {
                item: Some(pb::export_out::Item::Data(data)),
            }))
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "IMT stopped the export"))
    }
}

impl std::io::Write for ChunkWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(data);
        if self.buf.len() >= CHUNK_BYTES {
            self.send()?;
        }
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.send()
    }
}

#[tonic::async_trait]
impl pb::plugin_server::Plugin for PluginService {
    async fn handshake(&self, req: Request<pb::HandshakeRequest>) -> Result<Response<pb::Manifest>, Status> {
        let r = req.into_inner();
        if r.contract_major != CONTRACT_MAJOR {
            return Err(fail(
                code::VERSION,
                format!(
                    "this plugin implements imt.plugin.v{CONTRACT_MAJOR}, IMT speaks v{}",
                    r.contract_major
                ),
            ));
        }
        let granted = r
            .granted
            .iter()
            .filter_map(|p| crate::convert::permission_from(p).ok())
            .map(|p| p.permission)
            .collect();
        let h = &self.hosted;
        *h.granted.lock().unwrap_or_else(|p| p.into_inner()) = granted;
        h.handshaken.store(true, Ordering::SeqCst);
        let m = h.plugin.manifest();
        Ok(Response::new(pb::Manifest {
            id: h.id.into(),
            name: h.plugin.name().into(),
            version: m.version.to_string(),
            contract_major: CONTRACT_MAJOR,
            contract_minor: CONTRACT_MINOR,
            requires: m
                .requires
                .iter()
                .map(|d| pb::Dependency {
                    id: d.id.clone(),
                    req: d.req.to_string(),
                    optional: d.optional,
                })
                .collect(),
            provides: m
                .provides
                .iter()
                .map(|s| (s.id.clone(), s.version.to_string()))
                .chain(h.opts.services.iter().map(|(id, v, _)| (id.clone(), v.clone())))
                .fold(Vec::<pb::ServiceDecl>::new(), |mut acc, (id, version)| {
                    if !acc.iter().any(|d| d.id == id) {
                        acc.push(pb::ServiceDecl { id, version });
                    }
                    acc
                }),
            permissions: h.opts.permissions.iter().map(permission_to).collect(),
            capabilities: h.capabilities(),
        }))
    }

    async fn sniff(&self, req: Request<pb::SniffRequest>) -> Result<Response<pb::SniffReply>, Status> {
        let h = self.hosted.clone();
        h.gate()?;
        let r = req.into_inner();
        let (profile, path, file_date) = h.context(r.ctx.as_ref())?;
        let score = blocking(move || {
            let ctx = ImportCtx {
                profile: &profile,
                path: &path,
                file_date,
            };
            // The guard turns a panic into "0"; `run` still reports it as a crash.
            if let Some(i) = h.registry.hosted_importers().find(|i| i.id() == r.importer_id) {
                let head = String::from_utf8_lossy(&r.head);
                return h.run(|| Ok(i.sniff(&ctx, &head))).and_then(|s| h.crashed_quietly(s));
            }
            match h.registry.hosted_binary_importers().find(|i| i.id() == r.importer_id) {
                Some(i) => h.run(|| Ok(i.sniff(&ctx, &r.head))).and_then(|s| h.crashed_quietly(s)),
                None => Err(invalid(format!("no importer \"{}\"", r.importer_id))),
            }
        })
        .await?;
        Ok(Response::new(pb::SniffReply {
            score: score.min(100) as u32,
        }))
    }

    type ImportStream = Out<pb::ImportReply>;

    async fn import(&self, req: Request<Streaming<pb::ImportChunk>>) -> Result<Response<Self::ImportStream>, Status> {
        let h = self.hosted.clone();
        h.gate()?;
        let (start, _, body) = read_import(req.into_inner()).await?;
        let (profile, path, file_date) = h.context(start.ctx.as_ref())?;
        let (tx, rx) = mpsc::channel(4);
        tokio::task::spawn_blocking(move || {
            let ctx = ImportCtx {
                profile: &profile,
                path: &path,
                file_date,
            };
            let id = start.importer_id.as_str();
            let result = if let Some(i) = h.registry.hosted_importers().find(|i| i.id() == id) {
                let content = text(body);
                h.run(|| i.import(&ctx, &content))
            } else if let Some(i) = h.registry.hosted_binary_importers().find(|i| i.id() == id) {
                h.run(|| i.import(&ctx, &body))
            } else {
                Err(invalid(format!("no importer \"{id}\"")))
            };
            match result {
                Ok(imported) => send_imported(imported, start.batch_rows, &tx),
                Err(e) => {
                    let _ = tx.blocking_send(Err(e));
                }
            }
        });
        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
    }

    type ImportTailStream = Out<pb::ImportReply>;

    async fn import_tail(&self, req: Request<Streaming<pb::ImportChunk>>) -> Result<Response<Self::ImportTailStream>, Status> {
        let h = self.hosted.clone();
        h.gate()?;
        let (start, head, tail) = read_import(req.into_inner()).await?;
        let (profile, path, file_date) = h.context(start.ctx.as_ref())?;
        let (tx, rx) = mpsc::channel(4);
        tokio::task::spawn_blocking(move || {
            let ctx = ImportCtx {
                profile: &profile,
                path: &path,
                file_date,
            };
            let Some(i) = h.registry.hosted_importers().find(|i| i.id() == start.importer_id) else {
                let _ = tx.blocking_send(Err(invalid(format!("no text importer \"{}\"", start.importer_id))));
                return;
            };
            let (head, tail) = (text(head), text(tail));
            match h.run(|| Ok(i.import_tail(&ctx, &head, &tail))) {
                Ok(None) => {
                    let _ = tx.blocking_send(Ok(pb::ImportReply {
                        item: Some(pb::import_reply::Item::Done(pb::ImportDone {
                            skipped: 0,
                            tail_unsupported: true,
                        })),
                    }));
                }
                Ok(Some(Ok(imported))) => send_imported(imported, start.batch_rows, &tx),
                Ok(Some(Err(e))) => {
                    // A panic the guard turned into an error is a crash.
                    let status = match h.crashed_quietly(()) {
                        Ok(()) => fail(code::FAILED, format!("{e:#}")),
                        Err(crashed) => crashed,
                    };
                    let _ = tx.blocking_send(Err(status));
                }
                Err(e) => {
                    let _ = tx.blocking_send(Err(e));
                }
            }
        });
        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
    }

    type TransformStream = Out<pb::TransformOut>;

    async fn transform(&self, req: Request<Streaming<pb::TransformIn>>) -> Result<Response<Self::TransformStream>, Status> {
        let h = self.hosted.clone();
        h.gate()?;
        let mut s = req.into_inner();
        let start = match s.next().await {
            Some(Ok(pb::TransformIn {
                item: Some(pb::transform_in::Item::Start(st)),
            })) => st,
            Some(Err(e)) => return Err(e),
            _ => return Err(invalid("a transform starts with TransformStart")),
        };
        let transform = h
            .registry
            .hosted_transforms()
            .into_iter()
            .find(|t| t.id() == start.transform_id)
            .ok_or_else(|| invalid(format!("no transform \"{}\"", start.transform_id)))?;
        let source_name = Arc::new(start.source_name);
        let (tx, rx) = mpsc::channel(4);
        tokio::spawn(async move {
            // Up to `parallelism` batches are worked on at once; replies keep the order of the requests.
            let mut inflight: std::collections::VecDeque<tokio::task::JoinHandle<Result<pb::TransformOut, Status>>> = Default::default();
            let flush_one = |out: Option<Result<Result<pb::TransformOut, Status>, tokio::task::JoinError>>| match out {
                Some(Ok(r)) => r,
                Some(Err(e)) => Err(fail(code::CRASHED, format!("the plugin crashed: {e}"))),
                None => Err(invalid("no batch")),
            };
            loop {
                let m = s.next().await;
                let batch = match m {
                    None => break,
                    Some(Ok(pb::TransformIn {
                        item: Some(pb::transform_in::Item::Batch(b)),
                    })) => b,
                    Some(Ok(_)) => {
                        let _ = tx.send(Err(invalid("only batches may follow TransformStart"))).await;
                        return;
                    }
                    Some(Err(e)) => {
                        let _ = tx.send(Err(e)).await;
                        return;
                    }
                };
                let (h, t, source_name) = (h.clone(), transform.clone(), source_name.clone());
                let job = tokio::task::spawn_blocking(move || {
                    let mut records = arrow::decode(batch.arrow, &[]).map_err(|e| invalid(format!("{e:#}")))?.records;
                    let original: Vec<u64> = records.iter().map(|r| r.markers).collect();
                    for r in &mut records {
                        r.markers = 0;
                    }
                    let ctx = TransformCtx::with_marks(&source_name, &h.marks);
                    let seen = h.registry.health().notes(h.id);
                    let before = h.failures();
                    let verdicts = t.apply_batch(&mut records, &ctx);
                    if h.failures() > before {
                        return Err(fail(code::CRASHED, h.take_crash()));
                    }
                    let mut marks = Vec::new();
                    for (i, (r, m)) in records.iter_mut().zip(original).enumerate() {
                        if r.markers != 0 {
                            let kinds = (0..h.mark_names.len())
                                .filter(|b| r.markers & (1 << b) != 0)
                                .map(|b| h.mark_names[b].clone())
                                .collect();
                            marks.push(pb::RowMarks { row: i as u32, kinds });
                        }
                        r.markers = m;
                    }
                    let arrow = arrow::encode(&records, &[]).map_err(|e| fail(code::FAILED, format!("{e:#}")))?;
                    Ok(pb::TransformOut {
                        batch: Some(pb::RecordBatch {
                            arrow,
                            rows: records.len() as u32,
                        }),
                        verdicts: verdicts
                            .iter()
                            .map(|v| match v {
                                Verdict::Keep => pb::Verdict::Keep as i32,
                                Verdict::Drop => pb::Verdict::Drop as i32,
                            })
                            .collect(),
                        marks,
                        notes: h.new_notes(&seen),
                    })
                });
                inflight.push_back(job);
                if inflight.len() >= arrow::parallelism() {
                    let out = flush_one(match inflight.pop_front() {
                        Some(j) => Some(j.await),
                        None => None,
                    });
                    let stop = out.is_err();
                    if tx.send(out).await.is_err() || stop {
                        return;
                    }
                }
            }
            while let Some(j) = inflight.pop_front() {
                let out = flush_one(Some(j.await));
                let stop = out.is_err();
                if tx.send(out).await.is_err() || stop {
                    return;
                }
            }
        });
        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
    }

    async fn marker_kinds(&self, _: Request<pb::MarkerKindsRequest>) -> Result<Response<pb::MarkerKindsReply>, Status> {
        let h = &self.hosted;
        h.gate()?;
        Ok(Response::new(pb::MarkerKindsReply {
            kinds: h.registry.hosted_marker_kinds(h.id).iter().map(marker_kind_to).collect(),
        }))
    }

    async fn annotate(&self, req: Request<Streaming<pb::AnnotateIn>>) -> Result<Response<pb::AnnotateReply>, Status> {
        let h = self.hosted.clone();
        h.gate()?;
        let mut s = req.into_inner();
        let start = match s.next().await {
            Some(Ok(pb::AnnotateIn {
                item: Some(pb::annotate_in::Item::Start(st)),
            })) => st,
            Some(Err(e)) => return Err(e),
            _ => return Err(invalid("an annotation run starts with AnnotateStart")),
        };
        let annotator = h
            .registry
            .hosted_annotators()
            .into_iter()
            .find(|a| a.id() == start.annotator_id)
            .ok_or_else(|| invalid(format!("no annotator \"{}\"", start.annotator_id)))?;
        let mut records = Vec::new();
        while let Some(m) = s.next().await {
            let batch = match m?.item {
                Some(pb::annotate_in::Item::Batch(b)) => b,
                _ => return Err(invalid("only batches may follow AnnotateStart")),
            };
            let d = arrow::decode(batch.arrow, &["source_name", "instance"]).map_err(|e| invalid(format!("{e:#}")))?;
            if records.len() + d.records.len() > MAX_VIEW_RECORDS {
                return Err(fail(code::TOO_LARGE, format!("more than {MAX_VIEW_RECORDS} records")));
            }
            let mut extra = d.extra.into_iter();
            let (sources, instances) = (extra.next().unwrap_or_default(), extra.next().unwrap_or_default());
            records.extend(
                d.records
                    .into_iter()
                    .zip(sources.into_iter().zip(instances))
                    .map(|(record, (source, instance))| ViewRecord { record, source, instance }),
            );
        }
        let view = BatchView {
            records,
            instances: start.instances,
            range: start.has_range.then_some((start.range_from, start.range_to)),
        };
        let since = start.has_since.then_some(RecordCursor {
            id: start.since_id,
            ts: start.since_has_ts.then_some(start.since_ts),
        });
        let list = blocking(move || h.run(|| annotator.annotate(&view, since))).await?;
        Ok(Response::new(pb::AnnotateReply {
            annotations: list.iter().map(annotation_to).collect(),
        }))
    }

    type ExportStream = Out<pb::ExportOut>;

    async fn export(&self, req: Request<Streaming<pb::ExportIn>>) -> Result<Response<Self::ExportStream>, Status> {
        let h = self.hosted.clone();
        h.gate()?;
        let mut s = req.into_inner();
        let start = match s.next().await {
            Some(Ok(pb::ExportIn {
                item: Some(pb::export_in::Item::Start(st)),
            })) => st,
            Some(Err(e)) => return Err(e),
            _ => return Err(invalid("an export starts with ExportStart")),
        };
        let exporter = h
            .registry
            .hosted_exporters()
            .iter()
            .find(|e| e.id() == start.exporter_id)
            .cloned()
            .ok_or_else(|| invalid(format!("no exporter \"{}\"", start.exporter_id)))?;
        let (tx, rx) = mpsc::channel(4);
        let (rows_tx, rows_rx) = std::sync::mpsc::sync_channel::<Result<Vec<(Record, &'static str, String)>, Status>>(2);
        tokio::spawn(async move {
            while let Some(m) = s.next().await {
                let item = match m.map(|m| m.item) {
                    Ok(Some(pb::export_in::Item::Batch(b))) => arrow::decode(b.arrow, &["source_name", "group"])
                        .map_err(|e| invalid(format!("{e:#}")))
                        .map(|d| {
                            let mut extra = d.extra.into_iter();
                            let (sources, groups) = (extra.next().unwrap_or_default(), extra.next().unwrap_or_default());
                            d.records
                                .into_iter()
                                .zip(sources.iter().map(|s| interned(s)).zip(groups))
                                .map(|(r, (s, g))| (r, s, g))
                                .collect()
                        }),
                    Ok(_) => Err(invalid("only batches may follow ExportStart")),
                    Err(e) => Err(e),
                };
                let stop = item.is_err();
                let tx = rows_tx.clone();
                // A full channel blocks this task only briefly: the exporter thread drains it.
                if tokio::task::spawn_blocking(move || tx.send(item))
                    .await
                    .map_or(true, |r| r.is_err())
                    || stop
                {
                    return;
                }
            }
        });
        tokio::task::spawn_blocking(move || {
            let failed: std::cell::Cell<Option<Status>> = std::cell::Cell::new(None);
            let mut rows = rows_rx.iter().map_while(|b| match b {
                Ok(rows) => Some(rows),
                Err(e) => {
                    failed.set(Some(e));
                    None
                }
            });
            let mut pending: std::vec::IntoIter<(Record, &'static str, String)> = Vec::new().into_iter();
            let mut iter = std::iter::from_fn(|| loop {
                if let Some((record, source_name, group)) = pending.next() {
                    return Some(ExportRow {
                        record: std::borrow::Cow::Owned(record),
                        source_name,
                        group,
                    });
                }
                pending = rows.next()?.into_iter();
            });
            let mut out = ChunkWriter {
                tx: tx.clone(),
                buf: Vec::new(),
            };
            let result = h.run(|| {
                let n = exporter.export(&mut iter, &mut out)?;
                std::io::Write::flush(&mut out)?;
                Ok(n)
            });
            let reply = match (failed.take(), result) {
                (Some(e), _) | (None, Err(e)) => Err(e),
                (None, Ok(n)) => Ok(pb::ExportOut {
                    item: Some(pb::export_out::Item::Done(pb::ExportDone { rows: n as u64 })),
                }),
            };
            let _ = tx.blocking_send(reply);
        });
        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
    }

    async fn settings_section(&self, _: Request<pb::SettingsSectionRequest>) -> Result<Response<pb::SettingsSectionReply>, Status> {
        let h = &self.hosted;
        h.gate()?;
        let c = h
            .registry
            .hosted_settings(h.id)
            .ok_or_else(|| fail(code::UNIMPLEMENTED, "this plugin has no settings"))?;
        Ok(Response::new(pb::SettingsSectionReply {
            section: Some(section_to(&c.settings_section())),
        }))
    }

    async fn apply_settings(&self, req: Request<pb::ApplySettingsRequest>) -> Result<Response<pb::ApplySettingsReply>, Status> {
        let h = self.hosted.clone();
        h.gate()?;
        let c = h
            .registry
            .hosted_settings(h.id)
            .ok_or_else(|| fail(code::UNIMPLEMENTED, "this plugin has no settings"))?;
        let values: serde_json::Value =
            serde_json::from_str(&req.into_inner().values_json).map_err(|e| invalid(format!("the values are not JSON: {e}")))?;
        let valid = imt_core::settings::validate(&c.settings_section(), &values).map_err(|errors| {
            invalid(
                errors
                    .iter()
                    .map(|e| format!("{}: {}", e.key, e.message))
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        })?;
        blocking(move || h.run(|| c.apply_settings(&valid))).await?;
        Ok(Response::new(pb::ApplySettingsReply {}))
    }

    async fn call_service(&self, req: Request<pb::ServiceCall>) -> Result<Response<pb::ServiceReply>, Status> {
        let h = self.hosted.clone();
        h.gate()?;
        let r = req.into_inner();
        let svc = h
            .opts
            .services
            .iter()
            .find(|(id, _, _)| *id == r.service_id)
            .map(|(_, _, s)| s.clone())
            .ok_or_else(|| fail(code::UNIMPLEMENTED, format!("no service \"{}\"", r.service_id)))?;
        let payload = blocking(move || {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| svc.call(&r.method, &r.payload)))
                .map_err(|p| fail(code::CRASHED, imt_core::guard::panic_message(p.as_ref())))?
                .map_err(|e| fail(code::FAILED, format!("{e:#}")))
        })
        .await?;
        Ok(Response::new(pb::ServiceReply { payload: payload.into() }))
    }

    async fn events(&self, req: Request<Streaming<pb::HostEvent>>) -> Result<Response<pb::EventsAck>, Status> {
        let h = &self.hosted;
        h.gate()?;
        let mut s = req.into_inner();
        let bus = h.registry.events();
        let mut received = 0u64;
        while let Some(e) = s.next().await {
            let e = e?;
            received += 1;
            let payload: serde_json::Value = serde_json::from_str(&e.payload_json).unwrap_or_default();
            match e.kind.as_str() {
                "recordsAppended" => bus.publish(imt_core::events::RecordsAppended {
                    count: payload.get("count").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                    sources: payload
                        .get("sources")
                        .and_then(|v| v.as_array())
                        .map(|a| a.iter().filter_map(|x| x.as_u64()).map(|x| x as u32).collect())
                        .unwrap_or_default(),
                }),
                "profileChanged" => bus.publish(imt_core::events::ProfileChanged {
                    name: payload.get("name").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                }),
                "workspaceCleared" => bus.publish(imt_core::events::WorkspaceCleared),
                // Kinds of a newer minor version are ignored.
                _ => {}
            }
        }
        Ok(Response::new(pb::EventsAck { received }))
    }
}
