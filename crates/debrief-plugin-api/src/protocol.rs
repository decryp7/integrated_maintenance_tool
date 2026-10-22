//! The plugin protocol (ADR 0017 §4): what the host asks a plugin ([`PluginRequest`]) and what a plugin asks the
//! host back while it answers ([`HostRequest`]). Each message is the `body` of one frame (postcard); replies are
//! documented per request. Values that are JSON by nature cross as [`Json`].
//!
//! Records cross as [`WireEvent`]s (the parsed form of an event row: time, level, message, template, exception,
//! fields, line) in items of at most [`MAX_EVENTS_PER_ITEM`] rows; the host applies the profile's pipeline and
//! builds its event batches from them.

use crate::annotate::RecordCursor;
use crate::config::Limits;
use crate::model::{Fields, Level, Record};
use crate::tools::{FailureReason, ToolTask};
use debrief_ipc::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

pub use debrief_ipc::frame::ProtocolVersion;

/// The plugin protocol this SDK speaks: sent in the handshake and in the [`crate::Manifest`]. Debrief serves a plugin
/// built for the same major version and a minor version it knows; otherwise it refuses the plugin with a message
/// saying whether the plugin or Debrief needs an update.
///
/// - **minor**: a request, reply, host request or manifest field was added at the end (older plugins keep working);
/// - **major**: anything else (a plugin built for another major version must be rebuilt).
///
/// The versions Debrief releases speak are listed in `docs/plugin-sdk.md` ("Versions"). 1.1 added the elevated
/// process' output streams ([`HostRequest::SaveOutput`], [`PluginRequest::PullOutput`]) and its forwarded
/// standard error ([`HostRequest::Log`]).
pub const PROTOCOL: ProtocolVersion = ProtocolVersion::new(1, 1);

/// Most events in one streamed item (the engine's batch size, `debrief_batch::BATCH_ROWS`).
pub const MAX_EVENTS_PER_ITEM: usize = 8192;
/// An item is sent once its events reach about this many bytes, whatever their count.
pub const ITEM_BYTES: usize = 4 << 20;
/// Largest piece of a host file one [`HostRequest::ReadFile`] returns.
pub const READ_CHUNK: u32 = 1 << 20;
/// Most annotations one annotator may return.
pub const MAX_ANNOTATIONS: usize = crate::annotate::MAX_ANNOTATIONS;
/// Largest piece of an output stream ([`PluginRequest::PullOutput`] item); the host refuses larger ones.
pub const OUTPUT_CHUNK: usize = 256 << 10;
/// Pieces of an output stream in flight: the host grants this many credits, so at most
/// `OUTPUT_CREDIT * OUTPUT_CHUNK` bytes are on their way at once.
pub const OUTPUT_CREDIT: u32 = 4;
/// Largest output stream the host accepts, whatever the plugin declares.
pub const MAX_OUTPUT_BYTES: u64 = 8 << 30;
/// Output streams one plugin may have open at once.
pub const MAX_OUTPUTS: usize = 8;
/// The folder of the host-chosen data folder that output streams land in: `<data>/captures/<folder>/<name>`.
pub const OUTPUT_ROOT: &str = "captures";
/// Longest folder or file name of an output stream.
pub const MAX_OUTPUT_NAME: usize = 200;
/// Longest forwarded standard-error line (bytes); longer lines are cut at a character boundary.
pub const MAX_LOG_LINE: usize = debrief_ipc::child_log::MAX_LINE;
/// Most lines in one [`HostRequest::Log`]; the host drops the rest.
pub const MAX_LOG_BATCH: usize = 64;
/// Most forwarded lines the host logs per start of the elevated process; later ones are counted and dropped.
pub const MAX_LOG_LINES: u64 = 10_000;

/// A file (or text) the host opened for the plugin: read it with [`HostRequest::ReadFile`] (see
/// [`crate::host_file::HostFile`]). The plugin never gets a path for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostFileRef {
    /// Id the host gave the file; quote it in [`HostRequest::ReadFile`].
    pub id: u64,
    /// Size of the file in bytes.
    pub len: u64,
}

/// The active profile's timestamp settings (the plugin builds its [`crate::time::TimestampParser`] from them).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimestampSettings {
    /// Extra timestamp formats (.NET style or strftime) tried before the built-in ones.
    pub formats: Vec<String>,
    /// `UTC`, `local` or an offset (`+08:00`).
    pub zone: String,
    /// Date order: `Some(true)` day first, `Some(false)` month first, `None` decided per value.
    pub day_first: Option<bool>,
}

/// What an importer learns about a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileInfo {
    /// File name without folders.
    pub name: String,
    /// The path's trailing components, or the full path with [`crate::manifest::Permission::OpenedFilePaths`].
    pub path: String,
    /// `YYYY-MM-DD` of the last modification.
    pub date: Option<String>,
    /// The profile's timestamp settings.
    pub timestamps: TimestampSettings,
}

/// Host → plugin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PluginRequest {
    /// → [`Json`] of the [`crate::manifest::Manifest`].
    Manifest,
    /// → `u8` (0–100). `head` is the start of the file: decoded text (UTF-8) for text importers, bytes for
    /// binary ones.
    Sniff {
        /// Importer id.
        importer: Arc<str>,
        /// The file being sniffed.
        file: FileInfo,
        /// The start of the file.
        head: Vec<u8>,
    },
    /// Items: `Vec<WireEvent>`; → [`ImportDone`]. `content` is decoded text (UTF-8) for text importers.
    Import {
        /// Importer id.
        importer: Arc<str>,
        /// The file being imported.
        file: FileInfo,
        /// The decoded file content, read with [`HostRequest::ReadFile`].
        content: HostFileRef,
    },
    /// Items: `Vec<WireEvent>`; → [`TailDone`].
    ImportTail {
        /// Importer id.
        importer: Arc<str>,
        /// The file whose tail is imported.
        file: FileInfo,
        /// The start of the file, for headers and detected format.
        head: HostFileRef,
        /// The appended text.
        tail: HostFileRef,
    },
    /// → `u8`.
    SniffConfig {
        /// Parser id.
        parser: Arc<str>,
        /// Name of the file.
        file_name: String,
        /// The decoded start of the file.
        head: String,
    },
    /// → `Result<ConfigTree, ConfigError>`.
    ParseConfig {
        /// Parser id.
        parser: Arc<str>,
        /// The file content, read with [`HostRequest::ReadFile`].
        content: HostFileRef,
        /// Size, depth and node limits to enforce.
        limits: Limits,
    },
    /// → `Vec<Annotation>`. While it runs, the plugin reads the events with the `View*` host requests under
    /// `ticket`.
    Annotate {
        /// Annotator id.
        annotator: Arc<str>,
        /// Identifies this run in the `View*` host requests.
        ticket: u64,
        /// The first record that is new since the previous run.
        since: Option<RecordCursor>,
    },
    /// → [`TransformOut`].
    Transform {
        /// Transform id.
        transform: Arc<str>,
        /// Display name of the file the rows came from.
        source_name: String,
        /// The rows to transform.
        rows: Vec<WireEvent>,
    },
    /// → `()`. Values the host validated against the plugin's section (the plugin validates again).
    ApplySettings {
        /// The settings values, keyed by setting id.
        values: Json,
    },
    /// → `Result<Json, ToolFail>`. Path arguments were checked against the user's choices and are canonical.
    InvokeTool {
        /// Tool id.
        tool: Arc<str>,
        /// The tool operation.
        op: String,
        /// Arguments of the operation.
        args: Json,
        /// The plugin's private data folder, when it has one.
        data_dir: Option<String>,
    },
    /// → `Result<(), ToolFail>`.
    StopTask {
        /// Tool id.
        tool: Arc<str>,
        /// Id of the task to stop.
        task: String,
        /// The plugin's private data folder, when it has one.
        data_dir: Option<String>,
    },
    /// → `u8`.
    SniffProfile {
        /// Importer id.
        importer: Arc<str>,
        /// Name of the file.
        file_name: String,
        /// The start of the file.
        head: Vec<u8>,
    },
    /// Items: pieces (`Vec<u8>`) of the capture, encoded by the plugin's profile library; → [`ProfileDone`].
    ImportProfile {
        /// Importer id.
        importer: Arc<str>,
        /// Name of the file.
        file_name: String,
        /// The file content, read with [`HostRequest::ReadFile`].
        content: HostFileRef,
        /// Import limits as JSON.
        limits: Json,
    },
    /// → `u8`.
    SniffDocument {
        /// The document importer.
        importer: Arc<str>,
        /// The file's name.
        file_name: String,
        /// The first bytes of the file.
        head: Vec<u8>,
    },
    /// Items: pieces (`Vec<u8>`) of the document, encoded by the plugin's document library; → [`DocumentDone`].
    /// The plugin reads `content` by ranges.
    ImportDocument {
        /// The document importer.
        importer: Arc<str>,
        /// The file's name.
        file_name: String,
        /// The file, read by ranges.
        content: HostFileRef,
        /// The host's import options.
        options: Json,
    },
    /// Items: pieces (`Vec<u8>`, at most [`OUTPUT_CHUNK`] bytes) of the output stream the plugin opened with
    /// [`HostRequest::SaveOutput`]; → [`OutputEnd`]. The host sends it while it answers that request, with
    /// [`OUTPUT_CREDIT`] credits.
    PullOutput {
        /// The plugin's id of the stream (from [`HostRequest::SaveOutput`]).
        stream: u64,
    },
}

/// The end of an output stream: what the plugin sent in all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputEnd {
    /// Bytes of the stream.
    pub bytes: u64,
}

/// An output stream the host saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedOutput {
    /// The file name (in `<data>/captures/<folder>`).
    pub name: String,
    /// Its size in bytes.
    pub bytes: u64,
}

/// The end of an import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportDone {
    /// Lines or entries skipped (not records).
    pub skipped: u64,
    /// The importer's aggregated problems ([`crate::importer::Imported::problems`]).
    pub problems: Vec<crate::problems::Problem>,
}

/// The end of a live-tail import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TailDone {
    /// `false`: the importer cannot append; the host re-imports the whole file.
    pub supported: bool,
    /// Lines or entries skipped (not records).
    pub skipped: u64,
    /// The importer's aggregated problems.
    pub problems: Vec<crate::problems::Problem>,
}

/// The end of a profile import: the capture's encoded size and whether it is a heap snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileDone {
    /// Size of the encoded capture in bytes.
    pub bytes: u64,
    /// Whether the capture is a heap snapshot (else a profile).
    pub heap: bool,
}

/// The end of a document import: the encoded document's size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentDone {
    /// Bytes of the encoded document.
    pub bytes: u64,
}

/// A failed tool call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolFail {
    /// Why it failed, when the tool classifies the failure.
    pub reason: Option<FailureReason>,
    /// Message shown to the user.
    pub message: String,
}

/// A transform's answer: the rows (same count and order), a verdict per row (`true` keeps it), marks by row and
/// kind, and notes about problems.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TransformOut {
    /// The transformed rows, in the input order.
    pub rows: Vec<WireEvent>,
    /// Per row: `true` keeps it, `false` drops it.
    pub keep: Vec<bool>,
    /// Marks as `(row index, marker kind)`.
    pub marks: Vec<(u32, String)>,
    /// Notes about problems met.
    pub notes: Vec<String>,
}

/// Plugin → host, while it answers a request (or, for task reports, at any time).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HostRequest {
    /// → `Vec<u8>` (at most `len` and [`READ_CHUNK`] bytes; fewer at the end).
    ReadFile {
        /// Id of the [`HostFileRef`].
        id: u64,
        /// Byte offset to read from.
        offset: u64,
        /// Most bytes to return.
        len: u32,
    },
    /// → `Result<Vec<WireViewRecord>, String>`. `filter` is a JSON [`crate::annotate::ViewFilter`].
    ViewSelect {
        /// The ticket of the running annotation.
        ticket: u64,
        /// A JSON [`crate::annotate::ViewFilter`].
        filter: Json,
        /// Most records to return.
        limit: u32,
    },
    /// → `Vec<String>`.
    ViewInstances {
        /// The ticket of the running annotation.
        ticket: u64,
    },
    /// → `Option<(i64, i64)>`.
    ViewTimeRange {
        /// The ticket of the running annotation.
        ticket: u64,
    },
    /// → `()`.
    TaskStarted {
        /// Tool id.
        tool: String,
        /// The task that started.
        task: ToolTask,
    },
    /// → `()`.
    TaskProgress {
        /// Tool id.
        tool: String,
        /// Task id.
        task: String,
        /// Progress from 0 to 1; `None` when unknown.
        fraction: Option<f32>,
    },
    /// → `()`.
    TaskFinished {
        /// Tool id.
        tool: String,
        /// Task id.
        task: String,
    },
    /// → `Result<Json, ToolFail>`: runs one of the plugin's own elevated tool operations in its elevated process
    /// (ADR 0017 §7); the host refuses any other operation. Only the plugin's normal process may send it.
    InvokeElevated {
        /// Tool id.
        tool: String,
        /// The elevated operation.
        op: String,
        /// Arguments of the operation.
        args: Json,
        /// How long the host waits, in milliseconds.
        timeout_ms: u64,
    },
    /// → `Result<SavedOutput, String>`, once the stream ended: the host saves an output stream of the plugin's
    /// elevated process as `<data>/captures/<folder>/<name>` in the data folder *it* chose for `tool`. While it
    /// answers, it pulls the bytes with [`PluginRequest::PullOutput`] (credits, bounded pieces, at most
    /// `max_bytes` and [`MAX_OUTPUT_BYTES`]). The host checks the names ([`check_output_name`]), never replaces a
    /// file, and removes the partial file of a stream that failed. Only the elevated process may send it.
    SaveOutput {
        /// Tool id (whose data folder the host uses).
        tool: String,
        /// The plugin's id of the stream, quoted back in [`PluginRequest::PullOutput`].
        stream: u64,
        /// One folder name under `captures`.
        folder: String,
        /// The file name.
        name: String,
        /// Most bytes the stream may have.
        max_bytes: u64,
    },
    /// → `()`: lines the elevated process wrote to its standard error (it has none the host can read), logged by
    /// the host in the plugin's elevated log. The host cuts lines at [`MAX_LOG_LINE`], takes at most
    /// [`MAX_LOG_BATCH`] lines per request and [`MAX_LOG_LINES`] per start of the process. Only the elevated
    /// process may send it.
    Log {
        /// The lines, without line ends.
        lines: Vec<String>,
        /// Lines the plugin dropped since its last request (its own bounds).
        dropped: u64,
    },
}

/// Whether `name` may be an output stream's folder or file name: one plain component of `[A-Za-z0-9._-]`, at
/// most [`MAX_OUTPUT_NAME`] bytes, not starting or ending with a dot, not a reserved Windows device name.
pub fn check_output_name(what: &str, name: &str) -> Result<(), String> {
    let plain = name.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'));
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    let numbered = stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.as_bytes()[3].is_ascii_digit();
    let reserved = numbered || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL");
    if name.is_empty() || name.len() > MAX_OUTPUT_NAME || !plain || name.starts_with('.') || name.ends_with('.') || reserved {
        return Err(format!(
            "invalid output {what} {name:?}: use one name of letters, digits, '-', '_' and '.' (at most {MAX_OUTPUT_NAME})"
        ));
    }
    Ok(())
}

/// `line` cut to at most `max` bytes at a character boundary.
pub fn cut_line(line: &str, max: usize) -> &str {
    if line.len() <= max {
        return line;
    }
    let mut end = max;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    &line[..end]
}

/// A field value on the wire (postcard cannot carry a `serde_json::Value`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WireValue {
    /// JSON null.
    Null,
    /// A boolean.
    Bool(bool),
    /// A signed integer.
    Int(i64),
    /// An unsigned integer too large for `Int`.
    UInt(u64),
    /// A floating-point number.
    Float(f64),
    /// A string.
    Text(String),
    /// An array or object, as JSON text.
    Json(String),
}

impl From<&Value> for WireValue {
    fn from(v: &Value) -> Self {
        match v {
            Value::Null => WireValue::Null,
            Value::Bool(b) => WireValue::Bool(*b),
            Value::Number(n) => match (n.as_i64(), n.as_u64(), n.as_f64()) {
                (Some(i), _, _) => WireValue::Int(i),
                (_, Some(u), _) => WireValue::UInt(u),
                (_, _, Some(f)) => WireValue::Float(f),
                _ => WireValue::Json(n.to_string()),
            },
            Value::String(s) => WireValue::Text(s.clone()),
            other => WireValue::Json(other.to_string()),
        }
    }
}

impl From<WireValue> for Value {
    fn from(v: WireValue) -> Self {
        match v {
            WireValue::Null => Value::Null,
            WireValue::Bool(b) => Value::Bool(b),
            WireValue::Int(i) => Value::from(i),
            WireValue::UInt(u) => Value::from(u),
            WireValue::Float(f) => serde_json::Number::from_f64(f).map(Value::Number).unwrap_or(Value::Null),
            WireValue::Text(s) => Value::String(s),
            WireValue::Json(j) => serde_json::from_str(&j).unwrap_or(Value::String(j)),
        }
    }
}

/// A record on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireEvent {
    /// Id assigned by the workspace.
    pub id: u32,
    /// Timestamp in milliseconds since the Unix epoch (UTC).
    pub ts: Option<i64>,
    /// The level as its index in [`Level::ALL`].
    pub level: u8,
    /// Rendered message.
    pub message: String,
    /// Unrendered message template.
    pub template: Option<String>,
    /// Exception or stack-trace text.
    pub exception: Option<String>,
    /// In the record's order.
    pub fields: Vec<(String, WireValue)>,
    /// One-based line number in the source file.
    pub line: u32,
    /// The marker mask ([`Record::markers`]).
    pub markers: u64,
}

impl WireEvent {
    /// The wire form of `r` (its `source` is not sent).
    pub fn of(r: &Record) -> WireEvent {
        WireEvent {
            id: r.id,
            ts: r.ts,
            level: r.level as u8,
            message: r.message.clone(),
            template: r.template.clone(),
            exception: r.exception.clone(),
            fields: r.fields.iter().map(|(k, v)| (k.clone(), WireValue::from(v))).collect(),
            line: r.line,
            markers: r.markers,
        }
    }

    /// The record (a level code out of range is `Information`).
    pub fn into_record(self) -> Record {
        let mut fields = Fields::new();
        for (k, v) in self.fields {
            fields.insert(k, Value::from(v));
        }
        Record {
            id: self.id,
            ts: self.ts,
            level: Level::ALL.get(self.level as usize).copied().unwrap_or(Level::Information),
            message: self.message,
            template: self.template,
            exception: self.exception,
            fields,
            source: 0,
            line: self.line,
            markers: self.markers,
        }
    }

    /// Bytes this event takes on the wire, roughly (for sizing items).
    pub fn approx_bytes(&self) -> usize {
        64 + self.message.len()
            + self.template.as_ref().map_or(0, String::len)
            + self.exception.as_ref().map_or(0, String::len)
            + self
                .fields
                .iter()
                .map(|(k, v)| {
                    k.len()
                        + 9
                        + match v {
                            WireValue::Text(s) | WireValue::Json(s) => s.len(),
                            _ => 0,
                        }
                })
                .sum::<usize>()
    }
}

/// A record as an annotator's view returns it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireViewRecord {
    /// The event.
    pub event: WireEvent,
    /// Display name of its source file.
    pub source: String,
    /// Its instance (timeline lane) key.
    pub instance: String,
}

/// Splits `events` into items of at most [`MAX_EVENTS_PER_ITEM`] events and about [`ITEM_BYTES`] bytes.
pub fn chunk_events(
    events: impl IntoIterator<Item = WireEvent>,
    mut emit: impl FnMut(Vec<WireEvent>) -> Result<(), debrief_ipc::RemoteError>,
) -> Result<(), debrief_ipc::RemoteError> {
    let mut item = Vec::new();
    let mut bytes = 0usize;
    for e in events {
        bytes += e.approx_bytes();
        item.push(e);
        if item.len() >= MAX_EVENTS_PER_ITEM || bytes >= ITEM_BYTES {
            emit(std::mem::take(&mut item))?;
            bytes = 0;
        }
    }
    if !item.is_empty() {
        emit(item)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn records_roundtrip_through_wire_events_in_field_order() {
        let mut r = Record::new(7);
        r.id = 3;
        r.ts = Some(1_700_000_000_000);
        r.level = Level::Error;
        r.message = "boom — café".into();
        r.template = Some("t".into());
        r.exception = Some("trace".into());
        r.markers = 5;
        for (k, v) in [
            ("z", json!(null)),
            ("a", json!(true)),
            ("n", json!(-4)),
            ("u", json!(u64::MAX)),
            ("f", json!(1.5)),
            ("s", json!("x")),
            ("o", json!({"k": [1, 2]})),
        ] {
            r.fields.insert(k.into(), v);
        }
        let w = WireEvent::of(&r);
        let bytes = debrief_ipc::wire::encode(&w).unwrap();
        let back = debrief_ipc::wire::decode::<WireEvent>(&bytes).unwrap().into_record();
        assert_eq!(back.fields.keys().collect::<Vec<_>>(), ["z", "a", "n", "u", "f", "s", "o"]);
        assert_eq!(
            serde_json::to_value(&back.fields).unwrap(),
            serde_json::to_value(&r.fields).unwrap()
        );
        assert_eq!(
            (back.id, back.ts, back.level, back.line, back.markers),
            (3, r.ts, Level::Error, 7, 5)
        );
        assert_eq!((back.message, back.template, back.exception), (r.message, r.template, r.exception));
        assert!(w.approx_bytes() > 64);
        let mut odd = w;
        odd.level = 99;
        assert_eq!(odd.into_record().level, Level::Information);
        assert_eq!(Value::from(WireValue::Float(f64::NAN)), Value::Null);
        assert_eq!(Value::from(WireValue::Json("not json".into())), json!("not json"));
    }

    #[test]
    fn events_are_chunked_by_count_and_size() {
        let e = WireEvent::of(&Record::new(1));
        let mut sizes = Vec::new();
        chunk_events(std::iter::repeat_n(e.clone(), MAX_EVENTS_PER_ITEM * 2 + 1), |c| {
            sizes.push(c.len());
            Ok(())
        })
        .unwrap();
        assert_eq!(sizes, [MAX_EVENTS_PER_ITEM, MAX_EVENTS_PER_ITEM, 1]);
        let mut big = e;
        big.message = "x".repeat(ITEM_BYTES);
        let mut n = 0;
        chunk_events(vec![big.clone(), big], |c| {
            assert_eq!(c.len(), 1);
            n += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(n, 2);
        assert!(chunk_events(Vec::new(), |_| panic!("nothing to send")).is_ok());
    }

    #[test]
    fn requests_roundtrip() {
        let reqs = vec![
            PluginRequest::Manifest,
            PluginRequest::Import {
                importer: "evtx".into(),
                file: FileInfo {
                    name: "a.evtx".into(),
                    path: "logs/a.evtx".into(),
                    date: Some("2026-10-20".into()),
                    timestamps: TimestampSettings::default(),
                },
                content: HostFileRef { id: 1, len: 10 },
            },
            PluginRequest::InvokeTool {
                tool: "t".into(),
                op: "go".into(),
                args: json!({"a": 1}).into(),
                data_dir: None,
            },
        ];
        for r in reqs {
            let b = debrief_ipc::wire::encode(&r).unwrap();
            assert_eq!(debrief_ipc::wire::decode::<PluginRequest>(&b).unwrap(), r);
        }
        let h = HostRequest::TaskStarted {
            tool: "t".into(),
            task: ToolTask {
                id: "a".into(),
                label: "A".into(),
                kind: "k".into(),
                stoppable: true,
            },
        };
        assert_eq!(
            debrief_ipc::wire::decode::<HostRequest>(&debrief_ipc::wire::encode(&h).unwrap()).unwrap(),
            h
        );
    }
}
