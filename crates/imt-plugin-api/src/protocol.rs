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
use imt_ipc::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

/// Most events in one streamed item (the engine's batch size, `imt_batch::BATCH_ROWS`).
pub const MAX_EVENTS_PER_ITEM: usize = 8192;
/// An item is sent once its events reach about this many bytes, whatever their count.
pub const ITEM_BYTES: usize = 4 << 20;
/// Largest piece of a host file one [`HostRequest::ReadFile`] returns.
pub const READ_CHUNK: u32 = 1 << 20;
/// Most annotations one annotator may return.
pub const MAX_ANNOTATIONS: usize = crate::annotate::MAX_ANNOTATIONS;

/// A file (or text) the host opened for the plugin: read it with [`HostRequest::ReadFile`] (see
/// [`crate::host_file::HostFile`]). The plugin never gets a path for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostFileRef {
    pub id: u64,
    pub len: u64,
}

/// The active profile's timestamp settings (the plugin builds its [`crate::time::TimestampParser`] from them).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimestampSettings {
    pub formats: Vec<String>,
    /// `UTC`, `local` or an offset (`+08:00`).
    pub zone: String,
    pub day_first: Option<bool>,
}

/// What an importer learns about a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileInfo {
    pub name: String,
    /// The path's trailing components, or the full path with [`crate::manifest::Permission::OpenedFilePaths`].
    pub path: String,
    /// `YYYY-MM-DD` of the last modification.
    pub date: Option<String>,
    pub timestamps: TimestampSettings,
}

/// Host → plugin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PluginRequest {
    /// → [`Json`] of the [`crate::manifest::Manifest`].
    Manifest,
    /// → `u8` (0–100). `head` is the start of the file: decoded text (UTF-8) for text importers, bytes for
    /// binary ones.
    Sniff { importer: Arc<str>, file: FileInfo, head: Vec<u8> },
    /// Items: `Vec<WireEvent>`; → [`ImportDone`]. `content` is decoded text (UTF-8) for text importers.
    Import {
        importer: Arc<str>,
        file: FileInfo,
        content: HostFileRef,
    },
    /// Items: `Vec<WireEvent>`; → [`TailDone`].
    ImportTail {
        importer: Arc<str>,
        file: FileInfo,
        head: HostFileRef,
        tail: HostFileRef,
    },
    /// → `u8`.
    SniffConfig { parser: Arc<str>, file_name: String, head: String },
    /// → `Result<ConfigTree, ConfigError>`.
    ParseConfig {
        parser: Arc<str>,
        content: HostFileRef,
        limits: Limits,
    },
    /// → `Vec<Annotation>`. While it runs, the plugin reads the events with the `View*` host requests under
    /// `ticket`.
    Annotate {
        annotator: Arc<str>,
        ticket: u64,
        since: Option<RecordCursor>,
    },
    /// → [`TransformOut`].
    Transform {
        transform: Arc<str>,
        source_name: String,
        rows: Vec<WireEvent>,
    },
    /// → `()`. Values the host validated against the plugin's section (the plugin validates again).
    ApplySettings { values: Json },
    /// → `Result<Json, ToolFail>`. Path arguments were checked against the user's choices and are canonical.
    InvokeTool {
        tool: Arc<str>,
        op: String,
        args: Json,
        data_dir: Option<String>,
    },
    /// → `Result<(), ToolFail>`.
    StopTask {
        tool: Arc<str>,
        task: String,
        data_dir: Option<String>,
    },
    /// → `u8`.
    SniffProfile {
        importer: Arc<str>,
        file_name: String,
        head: Vec<u8>,
    },
    /// Items: pieces (`Vec<u8>`) of the capture, encoded by the plugin's profile library; → [`ProfileDone`].
    ImportProfile {
        importer: Arc<str>,
        file_name: String,
        content: HostFileRef,
        limits: Json,
    },
    /// → `u8`.
    SniffDocument {
        importer: Arc<str>,
        file_name: String,
        head: Vec<u8>,
    },
    /// Items: pieces (`Vec<u8>`) of the document, encoded by the plugin's document library; → [`DocumentDone`].
    /// The plugin reads `content` by ranges.
    ImportDocument {
        importer: Arc<str>,
        file_name: String,
        content: HostFileRef,
        options: Json,
    },
}

/// The end of an import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportDone {
    pub skipped: u64,
}

/// The end of a live-tail import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TailDone {
    /// `false`: the importer cannot append; the host re-imports the whole file.
    pub supported: bool,
    pub skipped: u64,
}

/// The end of a profile import: the capture's encoded size and whether it is a heap snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileDone {
    pub bytes: u64,
    pub heap: bool,
}

/// The end of a document import: the encoded document's size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentDone {
    pub bytes: u64,
}

/// A failed tool call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolFail {
    pub reason: Option<FailureReason>,
    pub message: String,
}

/// A transform's answer: the rows (same count and order), a verdict per row (`true` keeps it), marks by row and
/// kind, and notes about problems.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TransformOut {
    pub rows: Vec<WireEvent>,
    pub keep: Vec<bool>,
    pub marks: Vec<(u32, String)>,
    pub notes: Vec<String>,
}

/// Plugin → host, while it answers a request (or, for task reports, at any time).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HostRequest {
    /// → `Vec<u8>` (at most `len` and [`READ_CHUNK`] bytes; fewer at the end).
    ReadFile { id: u64, offset: u64, len: u32 },
    /// → `Result<Vec<WireViewRecord>, String>`. `filter` is a JSON [`crate::annotate::ViewFilter`].
    ViewSelect { ticket: u64, filter: Json, limit: u32 },
    /// → `Vec<String>`.
    ViewInstances { ticket: u64 },
    /// → `Option<(i64, i64)>`.
    ViewTimeRange { ticket: u64 },
    /// → `()`.
    TaskStarted { tool: String, task: ToolTask },
    /// → `()`.
    TaskProgress { tool: String, task: String, fraction: Option<f32> },
    /// → `()`.
    TaskFinished { tool: String, task: String },
}

/// A field value on the wire (postcard cannot carry a `serde_json::Value`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WireValue {
    Null,
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
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
    pub id: u32,
    pub ts: Option<i64>,
    pub level: u8,
    pub message: String,
    pub template: Option<String>,
    pub exception: Option<String>,
    /// In the record's order.
    pub fields: Vec<(String, WireValue)>,
    pub line: u32,
    pub markers: u64,
}

impl WireEvent {
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
    pub event: WireEvent,
    pub source: String,
    pub instance: String,
}

/// Splits `events` into items of at most [`MAX_EVENTS_PER_ITEM`] events and about [`ITEM_BYTES`] bytes.
pub fn chunk_events(
    events: impl IntoIterator<Item = WireEvent>,
    mut emit: impl FnMut(Vec<WireEvent>) -> Result<(), imt_ipc::RemoteError>,
) -> Result<(), imt_ipc::RemoteError> {
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
        let bytes = imt_ipc::wire::encode(&w).unwrap();
        let back = imt_ipc::wire::decode::<WireEvent>(&bytes).unwrap().into_record();
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
            let b = imt_ipc::wire::encode(&r).unwrap();
            assert_eq!(imt_ipc::wire::decode::<PluginRequest>(&b).unwrap(), r);
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
            imt_ipc::wire::decode::<HostRequest>(&imt_ipc::wire::encode(&h).unwrap()).unwrap(),
            h
        );
    }
}
