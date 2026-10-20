//! A plugin process as an ordinary [`Plugin`]: [`RemotePlugin`] registers importers, transforms,
//! exporters, marker kinds, annotators, settings and services whose calls go to the process over
//! `imt.plugin.v1`. The registry wraps them in the same guarded adapters as in-process contributions, so
//! the core, quarantine, settings, status and health see no difference.
//!
//! Every call has a deadline just under its watchdog budget ([`deadline_for`]). A crash, a missed
//! deadline, garbage on the wire or a panic in the plugin's code counts as a failure of the plugin (towards
//! its quarantine, like a panic in process) and the process is killed; the supervisor starts a fresh one,
//! with backoff, on the next call. Records cross the boundary in Arrow batches of up to 64k rows.

use crate::connection::{runtime, PluginConnection, PluginSpec};
use anyhow::{anyhow, Result};
use bytes::Bytes;
use imt_core::export::{ExportRow, Exporter};
use imt_core::guard::{CallKind, Health};
use imt_core::importers::{BinaryImporter, ImportCtx, Imported, Importer};
use imt_core::model::Record;
use imt_core::plugin::annotate::{Annotation, AnnotationView, Annotator, RecordCursor, ViewFilter};
use imt_core::plugin::{ActivationCtx, Manifest, RecordTransform, TransformCtx, Verdict};
use imt_core::settings::{Configurable, SettingsSection, ValidValues};
use imt_core::{Plugin, PluginRegistry};
use imt_ipc::supervisor::{ChildStatus, Supervisor};
use imt_ipc::IpcError;
use imt_plugin_sdk::arrow::{self, batch_ranges, record_size, WithExtra};
use imt_plugin_sdk::client::{self, Client};
use imt_plugin_sdk::convert::{annotation_from, marker_kind_from, section_from, Permission, PermissionRequest};
use imt_plugin_sdk::manifest::{ImporterCap, RemoteManifest};
use imt_plugin_sdk::{code, pb, ByteService};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The deadline of a call whose watchdog budget is `budget`: 90% of it, so the call fails (and the process
/// is stopped) before the in-process watchdog would mark the plugin hung for the same call.
pub fn deadline_for(budget: Duration) -> Duration {
    budget.mul_f64(0.9)
}

/// Settings calls run without a watchdog budget in process; out of process they get this deadline.
const SETTINGS_DEADLINE: Duration = Duration::from_secs(10);

fn leak(s: &str) -> &'static str {
    // Plugin ids and names are &'static in the plugin traits; one small string per installed contribution.
    Box::leak(s.to_string().into_boxed_str())
}

/// The process and what it declared, shared by every registry the plugin is installed in.
pub(crate) struct Link {
    pub(crate) sup: Supervisor<PluginConnection>,
    pub(crate) id: &'static str,
    pub(crate) name: &'static str,
    pub(crate) version: &'static str,
    pub(crate) manifest: RemoteManifest,
    pub(crate) grants: Vec<Permission>,
    /// Marker kinds and settings schema, read once at start.
    pub(crate) marker_kinds: Vec<pb::MarkerKind>,
    pub(crate) section: Option<SettingsSection>,
    /// Rows per record batch, both directions.
    pub(crate) batch_rows: std::sync::atomic::AtomicUsize,
}

/// The link plus the health ledger of the registry a contribution was registered in.
#[derive(Clone)]
struct Ctx {
    link: Arc<Link>,
    health: Arc<Health>,
}

impl Ctx {
    fn deadline(&self, kind: CallKind) -> Option<Duration> {
        self.health.budgets().get(kind).map(deadline_for)
    }

    /// Runs one call on the live process. Process failures and plugin panics count against the plugin
    /// and stop the process; typed errors of the plugin's code pass through.
    fn call<R, Fut>(&self, what: &str, timeout: Option<Duration>, f: impl FnOnce(Client, Option<Duration>) -> Fut) -> Result<R>
    where
        Fut: Future<Output = Result<R, IpcError>>,
    {
        let id = self.link.id;
        let rt = runtime().map_err(|e| anyhow!("plugin \"{id}\" ({what}): {e}"))?;
        let lease = match self.link.sup.lease() {
            Ok(l) => l,
            Err(e) => {
                let message = format!("plugin \"{id}\" ({what}) could not run: {e}");
                self.health.fail(id, &message);
                return Err(anyhow!(message));
            }
        };
        let r = client::block_on(rt.handle(), f(lease.conn.client.clone(), timeout));
        match r {
            Ok(v) => Ok(v),
            Err(IpcError::Remote(e)) if e.code != code::CRASHED => Err(anyhow!("plugin \"{id}\" ({what}): {}", e.message)),
            Err(e) => {
                let message = match &e {
                    IpcError::Timeout => format!(
                        "plugin \"{id}\" ({what}) did not finish within {}; its process was stopped",
                        timeout.map_or("its deadline".into(), |t| format!("{:.1} s", t.as_secs_f64()))
                    ),
                    IpcError::Died(_) => format!("plugin \"{id}\" ({what}) crashed: {}", lease.conn.exit_text()),
                    IpcError::Remote(r) => format!("plugin \"{id}\" ({what}) crashed: {}", r.message),
                    other => format!("plugin \"{id}\" ({what}) failed: {other}"),
                };
                // A panic inside the process may have left it inconsistent: restart it too.
                let as_failure = match e {
                    IpcError::Remote(r) => IpcError::Died(r.message),
                    other => other,
                };
                self.link.sup.report(&lease, &as_failure);
                self.health.fail(id, &message);
                Err(anyhow!(message))
            }
        }
    }

    /// Delivers one event (a failure counts like any other call).
    fn send_event(&self, kind: &str, payload: serde_json::Value) {
        let ev = pb::HostEvent {
            kind: kind.into(),
            payload_json: payload.to_string(),
        };
        let _ = self.call("event", self.deadline(CallKind::Event), |mut c, t| async move {
            client::events(&mut c, vec![ev], t).await
        });
    }

    fn may_read_files(&self) -> bool {
        self.link.grants.contains(&Permission::ReadOpenedFiles)
    }

    /// What the plugin learns about the file: its name, its date and the profile, plus the full path only
    /// when the user granted "read files I open".
    fn import_context(&self, ctx: &ImportCtx) -> pb::ImportContext {
        let mut profile = ctx.profile.profile.clone();
        profile.pipeline.clear();
        pb::ImportContext {
            file_name: ctx.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            path: if self.may_read_files() {
                ctx.path.to_string_lossy().into_owned()
            } else {
                String::new()
            },
            file_date: ctx.file_date.map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default(),
            profile_json: serde_json::to_string(&profile).unwrap_or_default(),
        }
    }

    fn import(
        &self,
        what: &str,
        cap: &ImporterCap,
        ctx: &ImportCtx,
        tail: bool,
        parts: Vec<(pb::ChunkPart, Bytes)>,
    ) -> Result<(Imported, bool)> {
        let start = pb::ImportStart {
            importer_id: cap.id.clone(),
            ctx: Some(self.import_context(ctx)),
            total_bytes: parts.iter().map(|p| p.1.len() as u64).sum(),
            batch_rows: self.link.batch_rows.load(std::sync::atomic::Ordering::Relaxed) as u32,
        };
        let kind = if tail { CallKind::Tail } else { CallKind::Import };
        let (outcome, records) = decode_in_parallel(|on_batch| {
            self.call(what, self.deadline(kind), |mut c, t| async move {
                client::import_raw(&mut c, tail, start, parts, t, on_batch).await
            })
        })?;
        Ok((
            Imported {
                records,
                skipped: outcome.skipped as usize,
                ..Default::default()
            },
            outcome.tail_unsupported,
        ))
    }
}

/// Runs `call`, which hands each received batch (Arrow bytes, rows) to the callback it is given, while
/// helper threads decode the batches; returns the call's result and the records in order.
fn decode_in_parallel<T>(call: impl FnOnce(&mut dyn FnMut(Bytes, u32) -> Result<(), IpcError>) -> Result<T>) -> Result<(T, Vec<Record>)> {
    type Slot = Result<Vec<Record>, String>;
    let slots: Mutex<Vec<Option<Slot>>> = Mutex::new(Vec::new());
    let (tx, rx) = std::sync::mpsc::channel::<(usize, Bytes, u32)>();
    let rx = Mutex::new(rx);
    let result = std::thread::scope(|s| {
        for _ in 0..arrow::decode_parallelism() {
            s.spawn(|| loop {
                let job = rx.lock().unwrap_or_else(|p| p.into_inner()).recv();
                let Ok((i, bytes, rows)) = job else { return };
                let decoded = arrow::decode(bytes, &[]).map_err(|e| format!("{e:#}")).and_then(|d| {
                    if d.records.len() == rows as usize {
                        Ok(d.records)
                    } else {
                        Err(format!("a batch says {rows} rows but holds {}", d.records.len()))
                    }
                });
                slots.lock().unwrap_or_else(|p| p.into_inner())[i] = Some(decoded);
            });
        }
        let mut next = 0usize;
        let mut on_batch = |bytes: Bytes, rows: u32| -> Result<(), IpcError> {
            slots.lock().unwrap_or_else(|p| p.into_inner()).push(None);
            tx.send((next, bytes, rows))
                .map_err(|_| IpcError::Protocol("the decoders stopped".into()))?;
            next += 1;
            Ok(())
        };
        let r = call(&mut on_batch);
        drop(tx);
        r
    })?;
    let mut records = Vec::new();
    for slot in slots.into_inner().unwrap_or_else(|p| p.into_inner()) {
        match slot {
            Some(Ok(mut batch)) => {
                if records.is_empty() {
                    records = batch;
                } else {
                    records.append(&mut batch);
                }
            }
            Some(Err(e)) => return Err(anyhow!("the plugin sent an invalid record batch: {e}")),
            None => return Err(anyhow!("a record batch was not decoded")),
        }
    }
    Ok((result, records))
}

struct RemoteImporter {
    ctx: Ctx,
    cap: ImporterCap,
    id: &'static str,
}

impl Importer for RemoteImporter {
    fn id(&self) -> &'static str {
        self.id
    }

    fn sniff(&self, ctx: &ImportCtx, head: &str) -> u8 {
        let req = pb::SniffRequest {
            importer_id: self.cap.id.clone(),
            ctx: Some(self.ctx.import_context(ctx)),
            head: Bytes::copy_from_slice(head.as_bytes()),
        };
        self.ctx
            .call("sniff", self.ctx.deadline(CallKind::Sniff), |mut c, t| async move {
                client::sniff(&mut c, req, t).await
            })
            .unwrap_or(0)
    }

    fn import(&self, ctx: &ImportCtx, content: &str) -> Result<Imported> {
        let content = Bytes::copy_from_slice(content.as_bytes());
        self.ctx
            .import("import", &self.cap, ctx, false, vec![(pb::ChunkPart::Content, content)])
            .map(|r| r.0)
    }

    fn import_tail(&self, ctx: &ImportCtx, head: &str, tail: &str) -> Option<Result<Imported>> {
        if !self.cap.tail {
            return None;
        }
        let parts = vec![
            (pb::ChunkPart::Head, Bytes::copy_from_slice(head.as_bytes())),
            (pb::ChunkPart::Tail, Bytes::copy_from_slice(tail.as_bytes())),
        ];
        match self.ctx.import("import_tail", &self.cap, ctx, true, parts) {
            Ok((_, true)) => None,
            Ok((imported, false)) => Some(Ok(imported)),
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
    id: &'static str,
}

impl BinaryImporter for RemoteBinaryImporter {
    fn id(&self) -> &'static str {
        self.id
    }

    fn identity_fields(&self) -> Vec<String> {
        self.cap.identity_fields.clone()
    }

    fn sniff(&self, ctx: &ImportCtx, head: &[u8]) -> u8 {
        let req = pb::SniffRequest {
            importer_id: self.cap.id.clone(),
            ctx: Some(self.ctx.import_context(ctx)),
            head: Bytes::copy_from_slice(head),
        };
        self.ctx
            .call("sniff", self.ctx.deadline(CallKind::Sniff), |mut c, t| async move {
                client::sniff(&mut c, req, t).await
            })
            .unwrap_or(0)
    }

    fn import(&self, ctx: &ImportCtx, bytes: &[u8]) -> Result<Imported> {
        let content = Bytes::copy_from_slice(bytes);
        self.ctx
            .import("import", &self.cap, ctx, false, vec![(pb::ChunkPart::Content, content)])
            .map(|r| r.0)
    }
}

struct RemoteTransform {
    ctx: Ctx,
    id: String,
}

impl RecordTransform for RemoteTransform {
    fn id(&self) -> &str {
        &self.id
    }

    /// One record at a time works, but costs a round trip; the pipeline uses [`Self::apply_batch`].
    fn apply(&self, record: &mut Record, ctx: &TransformCtx) -> Verdict {
        self.apply_batch(std::slice::from_mut(record), ctx)
            .first()
            .copied()
            .unwrap_or(Verdict::Keep)
    }

    fn batched(&self) -> bool {
        true
    }

    /// Streams the records in Arrow batches: helper threads encode the next batches while the plugin works
    /// on earlier ones, and others decode the replies as they arrive.
    fn apply_batch(&self, records: &mut [Record], ctx: &TransformCtx) -> Vec<Verdict> {
        let n = records.len();
        if n == 0 {
            return Vec::new();
        }
        let rows_per_batch = match self.ctx.link.batch_rows.load(std::sync::atomic::Ordering::Relaxed) {
            0 => arrow::TRANSFORM_BATCH_ROWS,
            n => n,
        };
        let ranges = batch_ranges(n, rows_per_batch, |i| record_size(&records[i]));
        let (id, source_name) = (self.id.clone(), ctx.source_name.to_string());
        let mut notes: Vec<String> = Vec::new();
        let mut extras: Vec<(Vec<i32>, Vec<pb::RowMarks>)> = Vec::new();
        let input: &[Record] = records;
        let result = std::thread::scope(|scope| {
            let (tx, rx) = tokio::sync::mpsc::channel::<pb::TransformIn>(arrow::parallelism() + 1);
            let start = pb::TransformIn {
                item: Some(pb::transform_in::Item::Start(pb::TransformStart {
                    transform_id: id.clone(),
                    source_name,
                })),
            };
            let ranges_enc = ranges.clone();
            let encoder = scope.spawn(move || -> Result<()> {
                tx.blocking_send(start).map_err(|_| anyhow!("the transform call ended"))?;
                let mut failed = None;
                arrow::encode_batches(input, &ranges_enc, arrow::parallelism(), |bytes, rows| match bytes {
                    Ok(arrow) => tx
                        .blocking_send(pb::TransformIn {
                            item: Some(pb::transform_in::Item::Batch(pb::RecordBatch { arrow, rows })),
                        })
                        .is_ok(),
                    Err(e) => {
                        failed = Some(e);
                        false
                    }
                });
                failed.map_or(Ok(()), Err)
            });
            let expected: Vec<usize> = ranges.iter().map(|r| r.len()).collect();
            let (notes, extras) = (&mut notes, &mut extras);
            let decoded = decode_in_parallel(|on_batch| {
                self.ctx
                    .call("transform", self.ctx.deadline(CallKind::Import), |mut c, t| async move {
                        let mut req = tonic::Request::new(tokio_stream::wrappers::ReceiverStream::new(rx));
                        if let Some(t) = t {
                            req.set_timeout(t);
                        }
                        let fut = async {
                            let mut stream = c.transform(req).await.map_err(|s| client::error(&s, String::new))?.into_inner();
                            for rows in expected {
                                let m = tokio_stream::StreamExt::next(&mut stream)
                                    .await
                                    .ok_or_else(|| IpcError::Protocol("the transform answered fewer batches than it was sent".into()))?
                                    .map_err(|s| client::error(&s, String::new))?;
                                let b = m
                                    .batch
                                    .ok_or_else(|| IpcError::Protocol("a transform reply without a batch".into()))?;
                                if b.rows as usize != rows {
                                    return Err(IpcError::Protocol(format!("{} records back for a batch of {rows}", b.rows)));
                                }
                                on_batch(b.arrow, b.rows)?;
                                notes.extend(m.notes.iter().take(5).map(|n| n.chars().take(500).collect::<String>()));
                                extras.push((m.verdicts, m.marks));
                            }
                            Ok(())
                        };
                        match t {
                            Some(t) => tokio::time::timeout(t + Duration::from_millis(250), fut)
                                .await
                                .unwrap_or(Err(IpcError::Timeout)),
                            None => fut.await,
                        }
                    })
            });
            let encoded = encoder.join().unwrap_or_else(|_| Err(anyhow!("the encoder crashed")));
            decoded.and_then(|d| encoded.map(|_| d))
        });
        for note in notes {
            self.ctx.health.note(self.ctx.link.id, &note);
        }
        let mut verdicts = vec![Verdict::Keep; n];
        let Ok(((), back)) = result else {
            // Failure is already counted; records stay unchanged and kept (like a crashing in-process transform).
            return verdicts;
        };
        if back.len() != n {
            return verdicts;
        }
        for (i, mut r) in back.into_iter().enumerate() {
            let original = &mut records[i];
            // What IMT assigned stays IMT's.
            r.id = original.id;
            r.source = original.source;
            r.markers = original.markers;
            *original = r;
        }
        for (range, (vs, marks)) in ranges.into_iter().zip(extras) {
            let base = range.start;
            for (i, v) in vs.iter().take(range.len()).enumerate() {
                if *v == pb::Verdict::Drop as i32 {
                    verdicts[base + i] = Verdict::Drop;
                }
            }
            for m in marks {
                if (m.row as usize) < range.len() {
                    let record = &mut records[base + m.row as usize];
                    for kind in m.kinds.iter().take(64) {
                        ctx.mark_record(record, kind);
                    }
                }
            }
        }
        verdicts
    }
}

struct RemoteExporter {
    ctx: Ctx,
    id: &'static str,
    name: &'static str,
    extension: &'static str,
}

impl Exporter for RemoteExporter {
    fn id(&self) -> &'static str {
        self.id
    }
    fn name(&self) -> &'static str {
        self.name
    }
    fn extension(&self) -> &'static str {
        self.extension
    }

    /// Streams rows out in batches while writing the plugin's output as it arrives.
    fn export(&self, rows: &mut dyn Iterator<Item = ExportRow<'_>>, out: &mut dyn std::io::Write) -> Result<usize> {
        let deadline = self.ctx.deadline(CallKind::Export);
        let rt = runtime().map_err(|e| anyhow!("{e}"))?;
        let id = self.id;
        let call = self.ctx.call(
            "export",
            deadline,
            |mut c, t| async move { client::start_export(&mut c, id, t).await },
        )?;
        let (out_tx, out_rx) = std::sync::mpsc::channel::<Result<pb::export_out::Item, IpcError>>();
        let mut output = call.output;
        rt.spawn(async move {
            while let Some(m) = tokio_stream::StreamExt::next(&mut output).await {
                let item = m
                    .map_err(|s| client::error(&s, String::new))
                    .and_then(|m| m.item.ok_or_else(|| IpcError::Protocol("an empty export reply".into())));
                let stop = item.is_err();
                if out_tx.send(item).is_err() || stop {
                    return;
                }
            }
            let _ = out_tx.send(Err(IpcError::Protocol("the export ended without ExportDone".into())));
        });
        let written = |item: Result<pb::export_out::Item, IpcError>, out: &mut dyn std::io::Write| -> Result<Option<u64>> {
            match item.map_err(|e| anyhow!("plugin \"{}\" (export): {e}", self.ctx.link.id))? {
                pb::export_out::Item::Data(d) => {
                    out.write_all(&d)?;
                    Ok(None)
                }
                pb::export_out::Item::Done(d) => Ok(Some(d.rows)),
            }
        };
        let mut batch: Vec<ExportRow<'_>> = Vec::new();
        let mut sent = 0usize;
        let mut finished = false;
        loop {
            let next = rows.next();
            let full = batch.len() >= arrow::DEFAULT_BATCH_ROWS;
            if (next.is_none() || full) && !batch.is_empty() {
                let with: Vec<WithExtra<'_, 2>> = batch
                    .iter()
                    .map(|r| WithExtra {
                        record: &r.record,
                        extra: [r.source_name, r.group.as_str()],
                    })
                    .collect();
                let arrow = arrow::encode(&with, &["source_name", "group"])?;
                let msg = pb::ExportIn {
                    item: Some(pb::export_in::Item::Batch(pb::RecordBatch {
                        arrow,
                        rows: batch.len() as u32,
                    })),
                };
                sent += batch.len();
                batch.clear();
                let input = call.input.clone();
                if client::block_on(rt.handle(), async move { input.send(msg).await }).is_err() {
                    // The plugin ended the call: its answer says why.
                    break;
                }
                while let Ok(item) = out_rx.try_recv() {
                    if written(item, out)?.is_some() {
                        finished = true;
                    }
                }
            }
            match next {
                Some(r) => batch.push(r),
                None => break,
            }
        }
        drop(call.input);
        if finished {
            return Ok(sent);
        }
        let wait = deadline.unwrap_or(Duration::from_secs(3600));
        loop {
            match out_rx.recv_timeout(wait) {
                Ok(item) => {
                    if let Some(n) = written(item, out)? {
                        return Ok(n as usize);
                    }
                }
                Err(_) => {
                    let message = format!("plugin \"{}\" (export) stopped answering", self.ctx.link.id);
                    self.ctx.health.fail(self.ctx.link.id, &message);
                    self.ctx.link.sup.shutdown();
                    return Err(anyhow!(message));
                }
            }
        }
    }
}

struct RemoteAnnotator {
    ctx: Ctx,
    cap: imt_plugin_sdk::manifest::AnnotatorCap,
}

impl Annotator for RemoteAnnotator {
    fn id(&self) -> &str {
        &self.cap.id
    }

    /// Sends the records the annotator declared interest in, as batches, and returns its complete set.
    fn annotate(&self, view: &dyn AnnotationView, since: Option<RecordCursor>) -> Result<Vec<Annotation>> {
        let records = view.select(
            &ViewFilter {
                when: self.cap.interest.clone(),
                ..ViewFilter::default()
            },
            self.cap.limit,
        )?;
        let mut batches = Vec::new();
        for r in batch_ranges(records.len(), arrow::DEFAULT_BATCH_ROWS, |i| record_size(&records[i].record)) {
            let rows: Vec<WithExtra<'_, 2>> = records[r]
                .iter()
                .map(|v| WithExtra {
                    record: &v.record,
                    extra: [v.source.as_str(), v.instance.as_str()],
                })
                .collect();
            batches.push(arrow::encode(&rows, &["source_name", "instance"])?);
        }
        let range = view.time_range();
        let start = pb::AnnotateStart {
            annotator_id: self.cap.id.clone(),
            has_since: since.is_some(),
            since_id: since.map_or(0, |s| s.id),
            since_ts: since.and_then(|s| s.ts).unwrap_or(0),
            since_has_ts: since.is_some_and(|s| s.ts.is_some()),
            has_range: range.is_some(),
            range_from: range.map_or(0, |r| r.0),
            range_to: range.map_or(0, |r| r.1),
            instances: view.instances(),
        };
        let list = self
            .ctx
            .call("annotate", self.ctx.deadline(CallKind::Annotate), |mut c, t| async move {
                client::annotate(&mut c, start, batches, t).await
            })?;
        list.iter().map(annotation_from).collect()
    }
}

struct RemoteSettings {
    /// Until the plugin is registered: a ledger of its own.
    fallback: Ctx,
    /// The context of the registry the plugin registered in (its health ledger).
    registered: Arc<Mutex<Option<Ctx>>>,
    section: SettingsSection,
}

impl Configurable for RemoteSettings {
    fn settings_section(&self) -> SettingsSection {
        self.section.clone()
    }

    fn apply_settings(&self, values: &ValidValues) -> Result<()> {
        let json = values.to_json();
        let ctx = self
            .registered
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .unwrap_or_else(|| self.fallback.clone());
        ctx.call("apply_settings", Some(SETTINGS_DEADLINE), |mut c, t| async move {
            client::apply_settings(&mut c, &json, t).await
        })
    }
}

struct RemoteService {
    ctx: Ctx,
    id: String,
    version: String,
}

impl ByteService for RemoteService {
    fn call(&self, method: &str, payload: &[u8]) -> Result<Vec<u8>> {
        let call = pb::ServiceCall {
            service_id: self.id.clone(),
            version: self.version.clone(),
            method: method.into(),
            payload: Bytes::copy_from_slice(payload),
        };
        self.ctx
            .call("service", self.ctx.deadline(CallKind::Service), |mut c, t| async move {
                client::call_service(&mut c, call, t).await
            })
            .map(|b| b.to_vec())
    }
}

/// An out-of-process plugin; see the module docs.
pub struct RemotePlugin {
    link: Arc<Link>,
    /// The call context of the registry the plugin was last registered in (for event delivery).
    events_ctx: Arc<Mutex<Option<Ctx>>>,
    subscriptions: Mutex<Vec<(Arc<imt_core::events::EventBus>, imt_core::events::SubscriptionId)>>,
}

impl RemotePlugin {
    /// Starts the plugin process, performs the handshake and reads its marker kinds and settings schema.
    /// The supervisor restarts the process later as needed.
    pub fn start(spec: PluginSpec) -> Result<Arc<RemotePlugin>> {
        let grants: Vec<Permission> = spec.grants.iter().map(|g| g.permission.clone()).collect();
        let launch_spec = spec.clone();
        let sup = Supervisor::new(spec.supervisor.clone(), move || PluginConnection::launch(&launch_spec));
        let lease = sup.lease().map_err(|e| anyhow!("{}: {e}", spec.label))?;
        let manifest = lease.conn.manifest.clone();
        let rt = runtime().map_err(|e| anyhow!("{e}"))?;
        let mut c = lease.conn.client.clone();
        let timeout = Some(spec.handshake_timeout);
        let marker_kinds = if manifest.capabilities.marker_kinds {
            client::block_on(rt.handle(), client::marker_kinds(&mut c, timeout))
                .map_err(|e| anyhow!("plugin \"{}\": marker kinds: {e}", manifest.id))?
        } else {
            Vec::new()
        };
        let section = if manifest.capabilities.settings {
            let s = client::block_on(rt.handle(), client::settings_section(&mut c, timeout))
                .map_err(|e| anyhow!("plugin \"{}\": settings: {e}", manifest.id))?;
            Some(section_from(&manifest.id, &s)?)
        } else {
            None
        };
        drop(lease);
        Ok(Arc::new(RemotePlugin {
            link: Arc::new(Link {
                id: leak(&manifest.id),
                name: leak(&manifest.name),
                version: leak(&manifest.version),
                sup,
                manifest,
                grants,
                marker_kinds,
                section,
                batch_rows: std::sync::atomic::AtomicUsize::new(0),
            }),
            events_ctx: Arc::new(Mutex::new(None)),
            subscriptions: Mutex::new(Vec::new()),
        }))
    }

    /// Sets the rows per record batch for imports and transforms (0: the defaults,
    /// [`arrow::DEFAULT_BATCH_ROWS`] and [`arrow::TRANSFORM_BATCH_ROWS`]).
    pub fn set_batch_rows(&self, rows: usize) {
        self.link
            .batch_rows
            .store(rows.min(arrow::MAX_BATCH_ROWS), std::sync::atomic::Ordering::Relaxed);
    }

    pub fn remote_manifest(&self) -> &RemoteManifest {
        &self.link.manifest
    }

    /// Permissions the plugin asks for that the user has not granted.
    pub fn missing_permissions(&self) -> Vec<PermissionRequest> {
        self.link
            .manifest
            .permissions
            .iter()
            .filter(|p| !self.link.grants.contains(&p.permission))
            .cloned()
            .collect()
    }

    /// The process' state (running, restarts, quarantine).
    pub fn process_status(&self) -> ChildStatus {
        self.link.sup.status()
    }

    /// Lifts the process quarantine (pair with `PluginRegistry::reset_plugin`).
    pub fn reset(&self) {
        self.link.sup.reset();
    }

    /// Stops the process (started again on next use).
    pub fn shutdown(&self) {
        self.link.sup.shutdown();
    }

    fn event_kinds(&self) -> &[String] {
        &self.link.manifest.capabilities.events
    }
}

impl Plugin for RemotePlugin {
    fn id(&self) -> &'static str {
        self.link.id
    }

    fn name(&self) -> &'static str {
        self.link.name
    }

    fn version(&self) -> &'static str {
        self.link.version
    }

    fn manifest(&self) -> Manifest {
        let m = &self.link.manifest;
        let mut out = Manifest::new(self.link.id, self.link.version);
        for d in &m.requires {
            out = if d.optional {
                out.optionally(d.id.clone(), &d.req)
            } else {
                out.requires(d.id.clone(), &d.req)
            };
        }
        for (id, version) in &m.provides {
            out = out.provides(id.clone(), version);
        }
        out
    }

    fn register(&self, registry: &mut PluginRegistry) -> Result<()> {
        let missing = self.missing_permissions();
        if !missing.is_empty() {
            // Never run a plugin with permissions the user did not grant.
            self.link.sup.shutdown();
            let list: Vec<String> = missing.iter().map(|p| p.permission.describe()).collect();
            return Err(anyhow!(
                "needs permission: {} (grant it in Settings → Plugin isolation)",
                list.join("; ")
            ));
        }
        let ctx = Ctx {
            link: self.link.clone(),
            health: registry.health(),
        };
        let caps = &self.link.manifest.capabilities;
        for cap in &caps.importers {
            if cap.binary {
                registry.add_binary_importer(Box::new(RemoteBinaryImporter {
                    ctx: ctx.clone(),
                    id: leak(&cap.id),
                    cap: cap.clone(),
                }));
            } else {
                registry.add_importer(Box::new(RemoteImporter {
                    ctx: ctx.clone(),
                    id: leak(&cap.id),
                    cap: cap.clone(),
                }));
            }
        }
        for id in &caps.transforms {
            registry.add_transform(Arc::new(RemoteTransform {
                ctx: ctx.clone(),
                id: id.clone(),
            }))?;
        }
        for e in &caps.exporters {
            registry.add_exporter(Arc::new(RemoteExporter {
                ctx: ctx.clone(),
                id: leak(&e.id),
                name: leak(&e.name),
                extension: leak(&e.extension),
            }))?;
        }
        if !self.link.marker_kinds.is_empty() {
            let kinds = self.link.marker_kinds.iter().map(marker_kind_from).collect::<Result<Vec<_>>>()?;
            registry.add_marker_kinds(kinds)?;
        }
        for a in &caps.annotators {
            registry.add_annotator(Arc::new(RemoteAnnotator {
                ctx: ctx.clone(),
                cap: a.clone(),
            }))?;
        }
        for (id, version) in &caps.services {
            registry.provide::<dyn ByteService>(
                id,
                version,
                Arc::new(RemoteService {
                    ctx: ctx.clone(),
                    id: id.clone(),
                    version: version.clone(),
                }),
            )?;
        }
        *self.events_ctx.lock().unwrap_or_else(|p| p.into_inner()) = Some(ctx);
        Ok(())
    }

    fn configurable(&self) -> Option<Arc<dyn Configurable>> {
        let section = self.link.section.clone()?;
        // Settings are described before `register`: failures count against the registry the plugin then
        // registers in (a ledger of their own until then).
        Some(Arc::new(RemoteSettings {
            fallback: Ctx {
                link: self.link.clone(),
                health: Arc::new(Health::default()),
            },
            registered: self.events_ctx.clone(),
            section,
        }))
    }

    /// Forwards the event kinds the plugin subscribed to.
    fn activate(&self, ctx: &ActivationCtx) -> Result<()> {
        let Some(link) = self.events_ctx.lock().unwrap_or_else(|p| p.into_inner()).clone() else {
            return Ok(());
        };
        let bus = ctx.events().clone();
        let mut subs = Vec::new();
        for kind in self.event_kinds() {
            let c = link.clone();
            let id = match kind.as_str() {
                "recordsAppended" => bus.subscribe::<imt_core::events::RecordsAppended>(self.link.id, move |e| {
                    c.send_event("recordsAppended", serde_json::json!({"count": e.count, "sources": e.sources}))
                })?,
                "profileChanged" => bus.subscribe::<imt_core::events::ProfileChanged>(self.link.id, move |e| {
                    c.send_event("profileChanged", serde_json::json!({"name": e.name}))
                })?,
                "workspaceCleared" => bus.subscribe::<imt_core::events::WorkspaceCleared>(self.link.id, move |_| {
                    c.send_event("workspaceCleared", serde_json::json!({}))
                })?,
                _ => continue,
            };
            subs.push((bus.clone(), id));
        }
        self.subscriptions.lock().unwrap_or_else(|p| p.into_inner()).extend(subs);
        Ok(())
    }

    fn deactivate(&self) {
        for (bus, id) in self.subscriptions.lock().unwrap_or_else(|p| p.into_inner()).drain(..) {
            bus.unsubscribe(id);
        }
    }
}
