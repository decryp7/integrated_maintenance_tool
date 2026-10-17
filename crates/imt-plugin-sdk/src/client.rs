//! Calling a plugin process: typed wrappers over the generated client, shared by IMT's plugin host and the
//! conformance kit. Every call has a deadline (sent as `grpc-timeout` and enforced locally); every failure
//! is an [`IpcError`] (a typed plugin error is [`IpcError::Remote`], a broken connection a crash, a missed
//! deadline a hang).

use crate::arrow::{self, Decoded};
use crate::manifest::RemoteManifest;
use crate::{pb, CHUNK_BYTES, CONTRACT_MAJOR, CONTRACT_MINOR, MAX_MESSAGE_BYTES};
use bytes::Bytes;
use imt_ipc::client::AuthChannel;
use imt_ipc::IpcError;
use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;
use tonic::{Request, Status, Streaming};

pub type Client = pb::plugin_client::PluginClient<AuthChannel>;

/// A client over an authenticated channel, sized for record batches.
pub fn client(channel: AuthChannel) -> Client {
    pb::plugin_client::PluginClient::new(channel)
        .max_decoding_message_size(MAX_MESSAGE_BYTES)
        .max_encoding_message_size(MAX_MESSAGE_BYTES)
}

/// A failed call as an [`IpcError`]; `died` describes the process when the connection broke.
pub fn error(s: &Status, died: impl FnOnce() -> String) -> IpcError {
    imt_ipc::error::from_status(s, "the plugin", died)
}

fn closed() -> String {
    "the plugin closed the connection".into()
}

fn request<T>(msg: T, timeout: Option<Duration>) -> Request<T> {
    let mut r = Request::new(msg);
    if let Some(t) = timeout {
        r.set_timeout(t);
    }
    r
}

/// Runs `fut` within `timeout` (plus a small grace for the child's own deadline to arrive first).
async fn within<T>(timeout: Option<Duration>, fut: impl Future<Output = Result<T, IpcError>>) -> Result<T, IpcError> {
    match timeout {
        None => fut.await,
        Some(t) => tokio::time::timeout(t + Duration::from_millis(250), fut)
            .await
            .unwrap_or(Err(IpcError::Timeout)),
    }
}

fn remaining(deadline: Option<Instant>) -> Option<Duration> {
    deadline.map(|d| d.saturating_duration_since(Instant::now()))
}

/// The handshake: offers `granted` permissions, returns the parsed manifest.
pub async fn handshake(
    c: &mut Client,
    granted: &[crate::PermissionRequest],
    timeout: Option<Duration>,
) -> Result<RemoteManifest, IpcError> {
    let req = pb::HandshakeRequest {
        contract_major: CONTRACT_MAJOR,
        contract_minor: CONTRACT_MINOR,
        host_version: env!("CARGO_PKG_VERSION").into(),
        granted: granted.iter().map(crate::convert::permission_to).collect(),
    };
    let m = within(timeout, async {
        c.handshake(request(req, timeout))
            .await
            .map(|r| r.into_inner())
            .map_err(|s| error(&s, closed))
    })
    .await?;
    RemoteManifest::parse(&m, CONTRACT_MAJOR).map_err(|e| IpcError::Protocol(format!("{e:#}")))
}

pub async fn sniff(c: &mut Client, req: pb::SniffRequest, timeout: Option<Duration>) -> Result<u8, IpcError> {
    within(timeout, async {
        let r = c.sniff(request(req, timeout)).await.map_err(|s| error(&s, closed))?;
        Ok(r.into_inner().score.min(100) as u8)
    })
    .await
}

/// How an import ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ImportOutcome {
    pub skipped: u64,
    pub tail_unsupported: bool,
}

/// The content of an import call, split into chunks (zero-copy slices of `content`).
pub fn chunks(part: pb::ChunkPart, content: &Bytes) -> impl Iterator<Item = pb::ImportChunk> + '_ {
    (0..content.len().div_ceil(CHUNK_BYTES)).map(move |i| pb::ImportChunk {
        item: Some(pb::import_chunk::Item::Chunk(pb::Chunk {
            part: part as i32,
            data: content.slice(i * CHUNK_BYTES..((i + 1) * CHUNK_BYTES).min(content.len())),
        })),
    })
}

/// `Import` (or `ImportTail` when `tail`): sends `start` and `parts`, hands every decoded batch to
/// `on_batch` as it arrives.
pub async fn import(
    c: &mut Client,
    tail: bool,
    start: pb::ImportStart,
    parts: Vec<(pb::ChunkPart, Bytes)>,
    timeout: Option<Duration>,
    mut on_batch: impl FnMut(Decoded) -> Result<(), IpcError>,
) -> Result<ImportOutcome, IpcError> {
    import_raw(c, tail, start, parts, timeout, |bytes, rows| {
        let d = arrow::decode(bytes, &[]).map_err(|e| IpcError::Protocol(format!("{e:#}")))?;
        if d.records.len() != rows as usize {
            return Err(IpcError::Protocol(format!(
                "a batch says {rows} rows but holds {}",
                d.records.len()
            )));
        }
        on_batch(d)
    })
    .await
}

/// [`import`] handing each batch's Arrow bytes and row count to `on_batch` undecoded (so the caller can
/// decode batches in parallel).
pub async fn import_raw(
    c: &mut Client,
    tail: bool,
    start: pb::ImportStart,
    parts: Vec<(pb::ChunkPart, Bytes)>,
    timeout: Option<Duration>,
    mut on_batch: impl FnMut(Bytes, u32) -> Result<(), IpcError>,
) -> Result<ImportOutcome, IpcError> {
    within(timeout, async {
        let mut msgs = vec![pb::ImportChunk {
            item: Some(pb::import_chunk::Item::Start(start)),
        }];
        for (part, content) in &parts {
            msgs.extend(chunks(*part, content));
        }
        let outbound = tokio_stream::iter(msgs);
        let reply = if tail {
            c.import_tail(request(outbound, timeout)).await
        } else {
            c.import(request(outbound, timeout)).await
        };
        let mut stream = reply.map_err(|s| error(&s, closed))?.into_inner();
        while let Some(m) = stream.next().await {
            match m.map_err(|s| error(&s, closed))?.item {
                Some(pb::import_reply::Item::Batch(b)) => on_batch(b.arrow, b.rows)?,
                Some(pb::import_reply::Item::Done(d)) => {
                    return Ok(ImportOutcome {
                        skipped: d.skipped,
                        tail_unsupported: d.tail_unsupported,
                    })
                }
                None => return Err(IpcError::Protocol("an empty import reply".into())),
            }
        }
        Err(IpcError::Protocol("the import ended without ImportDone".into()))
    })
    .await
}

/// One transformed batch.
#[derive(Debug, Default)]
pub struct Transformed {
    pub records: Vec<imt_core::model::Record>,
    pub verdicts: Vec<imt_core::plugin::Verdict>,
    /// (row, marker kinds).
    pub marks: Vec<(usize, Vec<String>)>,
}

/// `Transform`: sends `batches` (already encoded) and returns the transformed batches, in order.
pub async fn transform(
    c: &mut Client,
    transform_id: &str,
    source_name: &str,
    batches: Vec<(Bytes, usize)>,
    timeout: Option<Duration>,
) -> Result<Vec<Transformed>, IpcError> {
    within(timeout, async {
        let expected: Vec<usize> = batches.iter().map(|b| b.1).collect();
        let mut msgs = vec![pb::TransformIn {
            item: Some(pb::transform_in::Item::Start(pb::TransformStart {
                transform_id: transform_id.into(),
                source_name: source_name.into(),
            })),
        }];
        msgs.extend(batches.into_iter().map(|(arrow, rows)| pb::TransformIn {
            item: Some(pb::transform_in::Item::Batch(pb::RecordBatch { arrow, rows: rows as u32 })),
        }));
        let mut stream = c
            .transform(request(tokio_stream::iter(msgs), timeout))
            .await
            .map_err(|s| error(&s, closed))?
            .into_inner();
        let mut out = Vec::with_capacity(expected.len());
        for rows in expected {
            let m = stream
                .next()
                .await
                .ok_or_else(|| IpcError::Protocol("the transform answered fewer batches than it was sent".into()))?
                .map_err(|s| error(&s, closed))?;
            let b = m
                .batch
                .ok_or_else(|| IpcError::Protocol("a transform reply without a batch".into()))?;
            let records = arrow::decode(b.arrow, &[])
                .map_err(|e| IpcError::Protocol(format!("{e:#}")))?
                .records;
            if records.len() != rows {
                return Err(IpcError::Protocol(format!(
                    "the transform returned {} records for a batch of {rows}",
                    records.len()
                )));
            }
            let verdicts = (0..rows)
                .map(|i| match m.verdicts.get(i).copied().map(pb::Verdict::try_from) {
                    Some(Ok(pb::Verdict::Drop)) => imt_core::plugin::Verdict::Drop,
                    _ => imt_core::plugin::Verdict::Keep,
                })
                .collect();
            let marks = m
                .marks
                .into_iter()
                .filter(|rm| (rm.row as usize) < rows)
                .map(|rm| (rm.row as usize, rm.kinds.into_iter().take(64).collect()))
                .collect();
            out.push(Transformed { records, verdicts, marks });
        }
        Ok(out)
    })
    .await
}

pub async fn marker_kinds(c: &mut Client, timeout: Option<Duration>) -> Result<Vec<pb::MarkerKind>, IpcError> {
    within(timeout, async {
        let r = c
            .marker_kinds(request(pb::MarkerKindsRequest {}, timeout))
            .await
            .map_err(|s| error(&s, closed))?;
        Ok(r.into_inner().kinds)
    })
    .await
}

pub async fn annotate(
    c: &mut Client,
    start: pb::AnnotateStart,
    batches: Vec<Bytes>,
    timeout: Option<Duration>,
) -> Result<Vec<pb::Annotation>, IpcError> {
    within(timeout, async {
        let mut msgs = vec![pb::AnnotateIn {
            item: Some(pb::annotate_in::Item::Start(start)),
        }];
        msgs.extend(batches.into_iter().map(|arrow| pb::AnnotateIn {
            item: Some(pb::annotate_in::Item::Batch(pb::RecordBatch { arrow, rows: 0 })),
        }));
        let r = c
            .annotate(request(tokio_stream::iter(msgs), timeout))
            .await
            .map_err(|s| error(&s, closed))?;
        let list = r.into_inner().annotations;
        if list.len() > 10 * imt_core::plugin::annotate::MAX_ANNOTATIONS {
            return Err(IpcError::Protocol("too many annotations".into()));
        }
        Ok(list)
    })
    .await
}

/// An export in progress: batches go in through `input`, the file's bytes come out of `output`.
pub struct ExportCall {
    pub input: mpsc::Sender<pb::ExportIn>,
    pub output: Streaming<pb::ExportOut>,
}

/// Starts `Export` for `exporter_id`.
pub async fn start_export(c: &mut Client, exporter_id: &str, timeout: Option<Duration>) -> Result<ExportCall, IpcError> {
    let (tx, rx) = mpsc::channel(4);
    tx.send(pb::ExportIn {
        item: Some(pb::export_in::Item::Start(pb::ExportStart {
            exporter_id: exporter_id.into(),
        })),
    })
    .await
    .map_err(|_| IpcError::Protocol("cannot start the export".into()))?;
    let output = within(timeout, async {
        c.export(request(ReceiverStream::new(rx), timeout))
            .await
            .map(|r| r.into_inner())
            .map_err(|s| error(&s, closed))
    })
    .await?;
    Ok(ExportCall { input: tx, output })
}

pub async fn settings_section(c: &mut Client, timeout: Option<Duration>) -> Result<pb::SettingsSection, IpcError> {
    within(timeout, async {
        let r = c
            .settings_section(request(pb::SettingsSectionRequest {}, timeout))
            .await
            .map_err(|s| error(&s, closed))?;
        r.into_inner()
            .section
            .ok_or_else(|| IpcError::Protocol("a settings reply without a section".into()))
    })
    .await
}

pub async fn apply_settings(c: &mut Client, values: &serde_json::Value, timeout: Option<Duration>) -> Result<(), IpcError> {
    within(timeout, async {
        c.apply_settings(request(
            pb::ApplySettingsRequest {
                values_json: values.to_string(),
            },
            timeout,
        ))
        .await
        .map_err(|s| error(&s, closed))?;
        Ok(())
    })
    .await
}

pub async fn call_service(c: &mut Client, call: pb::ServiceCall, timeout: Option<Duration>) -> Result<Bytes, IpcError> {
    within(timeout, async {
        let r = c.call_service(request(call, timeout)).await.map_err(|s| error(&s, closed))?;
        Ok(r.into_inner().payload)
    })
    .await
}

pub async fn events(c: &mut Client, events: Vec<pb::HostEvent>, timeout: Option<Duration>) -> Result<u64, IpcError> {
    within(timeout, async {
        let r = c
            .events(request(tokio_stream::iter(events), timeout))
            .await
            .map_err(|s| error(&s, closed))?;
        Ok(r.into_inner().received)
    })
    .await
}

/// `deadline` as the time left, for calls made one after another under one budget.
pub fn left(deadline: Option<Instant>) -> Option<Duration> {
    remaining(deadline)
}

struct ThreadWaker(std::thread::Thread);

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

/// Runs `fut` to completion on this thread, inside `rt`'s context (its timers and I/O are driven by the
/// runtime's own threads). Unlike `Runtime::block_on` it may be called from any thread, including one that
/// belongs to another runtime, so synchronous plugin traits can call async code wherever IMT runs them.
pub fn block_on<F: Future>(rt: &tokio::runtime::Handle, fut: F) -> F::Output {
    let _guard = rt.enter();
    let waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut fut = std::pin::pin!(fut);
    loop {
        if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::park();
    }
}
