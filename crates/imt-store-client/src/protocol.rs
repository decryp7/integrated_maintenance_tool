//! The worker protocol over IMT's one IPC (ADR 0017): every call is a [`StoreRequest`] frame body (postcard);
//! the operation is typed ([`StoreOp`]), the port DTOs it carries and answers with cross as JSON text (they are
//! the ports' JSON-shaped types, unchanged). A failed call answers a [`imt_ipc::RemoteError`] whose `data` is a
//! [`WireError`], so the client gets the same [`StoreError`] back; anything else (a crash, a hang, garbage, an
//! oversized frame) is a failure of the worker process.

pub use crate::port_table::{ty, wire_names, WIRE_VERSION};
use imt_core::batch::SourceId;
use imt_core::ports::StoreError;
use imt_core::Profile;
use imt_ipc::RemoteError;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::path::PathBuf;

/// Largest frame either way (a page of 5,000 rows with long messages fits).
pub const MAX_MESSAGE_BYTES: usize = 64 << 20;

/// Initial credit of a streamed scan (batches of at most 8,192 rows in flight).
pub const SCAN_CREDIT: u32 = 4;

/// How often a cancellable call (the query language, the analyses) sends its progress while it runs: as stream
/// items, each a [`imt_core::ql::QueryProgress`] (postcard), sent only when it changed.
pub const PROGRESS_EVERY: std::time::Duration = std::time::Duration::from_millis(150);

use crate::{store_ops_enum, store_port_table};

store_port_table!(store_ops_enum, [StoreOp]);

/// "Parse with…": reads one source with the library's parser `parser` (`None`: the automatic choice; ADR 0026).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignParser {
    pub source: SourceId,
    pub parser: Option<String>,
}

/// What the app asks the worker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StoreRequest {
    /// An [`OpenStore`] → [`Handle`].
    OpenStore(imt_ipc::Json),
    CloseStore {
        store: u64,
    },
    /// → [`WorkerStatus`].
    Status,
    /// `op` on the open store `store`, with its request DTO; the reply is the operation's DTO as JSON.
    Call {
        store: u64,
        op: StoreOp,
        body: imt_ipc::Json,
    },
    /// A streamed `Query::scan` of the events matching a filter (JSON): items are Arrow IPC streams of one
    /// event batch each ([`crate::arrow_wire`]), the answer `()`.
    Scan {
        store: u64,
        filter: imt_ipc::Json,
    },
    /// An [`OpenProject`] → [`OpenedProject`].
    OpenProject(imt_ipc::Json),
    /// The installed third-party plugins now (a list of [`ThirdPartyPlugin`]) → (): a plugin whose grants changed
    /// stops and starts again on next use in the sandbox of the new grants; a plugin no longer listed stops.
    SetPlugins(imt_ipc::Json),
}

/// An installed third-party plugin (ADR 0027) whose importers take part in the worker's imports. The worker starts
/// it itself, contained exactly as the app does, from its install record in `plugins_dir`, and only while the
/// record is enabled and names the program with the SHA-256 `sha256` (the one the app loaded at its start: an
/// update or a reinstall takes effect at the next start of IMT, as in the app). `granted` is what the user grants
/// now; the worker refuses grants the record does not ask for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThirdPartyPlugin {
    /// The install folder (`<app data>/plugins`).
    pub plugins_dir: PathBuf,
    /// The plugin's id.
    pub id: String,
    /// Its program's SHA-256, hex.
    pub sha256: String,
    /// What it is granted now.
    pub granted: Vec<imt_plugin_api::manifest::Permission>,
}

/// Opens a project folder `<name>.imt` in place, or an exported `.imtz` extracted into a new folder under
/// `dest_root`; the project's profile overrides are merged over `profile`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenProject {
    pub path: PathBuf,
    pub dest_root: PathBuf,
    pub profile: Profile,
    #[serde(default)]
    pub disabled_plugins: Vec<String>,
    #[serde(default)]
    pub safe_mode: bool,
    #[serde(default)]
    pub third_party: Vec<ThirdPartyPlugin>,
    #[serde(default)]
    pub limits: Option<Limits>,
    /// The parser library (ADR 0026); absent: the built-in parsers.
    #[serde(default)]
    pub parsers: Option<imt_core::parser::ParserSetup>,
}

/// An opened project: its store's handle and folder, and its manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedProject {
    pub store: u64,
    pub dir: PathBuf,
    pub manifest: imt_core::package::ProjectManifest,
}

/// Error code of every store error (the kind is in [`WireError`]).
pub const STORE_ERROR: i32 = -32050;

/// Opens (or creates) the store in a folder.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenStore {
    pub dir: PathBuf,
    pub profile: Profile,
    /// A new, empty store (an existing one there is replaced).
    pub create: bool,
    /// Plugins the user turned off (their importers are not installed in the worker's registry).
    #[serde(default)]
    pub disabled_plugins: Vec<String>,
    /// Safe mode: no plugin is started; only the built-in importers read files.
    #[serde(default)]
    pub safe_mode: bool,
    /// Installed third-party plugins whose importers are installed in the worker's registry too.
    #[serde(default)]
    pub third_party: Vec<ThirdPartyPlugin>,
    /// DuckDB memory limit and threads, parse threads (defaults when absent).
    #[serde(default)]
    pub limits: Option<Limits>,
    /// The parser library (ADR 0026); absent: the built-in parsers.
    #[serde(default)]
    pub parsers: Option<imt_core::parser::ParserSetup>,
}

/// How the worker uses memory, chosen when it starts (the user's Low memory mode setting).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MemoryMode {
    /// The budgets of ADR 0014 at full speed.
    #[default]
    Normal,
    /// Freed memory goes back to the system at once; fewer parse threads, a smaller database cache and index
    /// writer (imports 20-40% slower).
    Low,
}

/// The worker's command-line argument that selects low memory mode.
pub const LOW_MEMORY_ARG: &str = "--low-memory";

impl MemoryMode {
    /// The worker's arguments for this mode.
    pub fn args(self) -> Vec<String> {
        match self {
            MemoryMode::Normal => Vec::new(),
            MemoryMode::Low => vec![LOW_MEMORY_ARG.to_string()],
        }
    }

    /// The mode the worker's arguments select (the others belong to the IPC bootstrap).
    pub fn from_args(args: impl IntoIterator<Item = String>) -> MemoryMode {
        if args.into_iter().any(|a| a == LOW_MEMORY_ARG) {
            MemoryMode::Low
        } else {
            MemoryMode::Normal
        }
    }
}

/// The worker's command-line argument that carries its [`StoreTuning`] (`--tuning=<json>`).
pub const TUNING_ARG: &str = "--tuning=";

/// Longest `--tuning` argument the worker reads (a spill folder path and a few numbers).
const MAX_TUNING_ARG: usize = 8 * 1024;

/// The store worker's knobs from Settings ▸ Performance (ADR 0031), sent on its command line and applied to every
/// store it opens; each one left out keeps the memory mode's value. Parsed once at the worker's edge
/// ([`StoreTuning::from_args`]) and bounded there ([`StoreTuning::bounded`]).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StoreTuning {
    /// Parse threads of an import.
    pub parse_threads: Option<usize>,
    /// DuckDB query threads.
    pub duck_threads: Option<usize>,
    /// DuckDB's memory limit, MB.
    pub memory_limit_mb: Option<u64>,
    /// The full-text index writer's heap, MB (shared by its threads).
    pub index_heap_mb: Option<u64>,
    /// The full-text index writer's threads.
    pub index_threads: Option<usize>,
    /// Where DuckDB spills what does not fit in its memory limit (a folder per store below it); absent: the store
    /// folder's `tmp`.
    pub spill_dir: Option<PathBuf>,
    /// The most DuckDB may spill, MB (`max_temp_directory_size`); absent: DuckDB's default.
    pub max_spill_mb: Option<u64>,
    /// Freed memory DuckDB's allocator may keep cached, MB (`allocator_flush_threshold`).
    pub allocator_flush_mb: Option<u64>,
    /// DuckDB's allocator background threads (`allocator_background_threads`).
    pub allocator_background_threads: Option<bool>,
    /// Arenas of DuckDB's own allocator where it brings jemalloc (Linux), bounded to 1..=256; absent: the worker's
    /// default (2). Ignored on other systems.
    pub allocator_arenas: Option<usize>,
    /// The memory cap of the worker's plugin processes whose manifests name none, MB.
    pub plugin_memory_mb: Option<u64>,
    /// Memory caps the user set for single plugins, MB, by plugin id.
    pub plugin_memory_overrides_mb: std::collections::BTreeMap<String, u64>,
    /// The CPU share of the worker's plugin processes, percent.
    pub plugin_cpu_percent: Option<u8>,
}

/// Bounds of [`StoreTuning`]'s numbers (a hand-edited or hostile command line cannot ask for absurd values).
pub const MAX_TUNING_THREADS: usize = 256;
/// The most memory or spill any one tuning value names, MB (1 TiB).
pub const MAX_TUNING_MB: u64 = 1 << 20;
/// The most per-plugin memory caps a tuning carries.
pub const MAX_PLUGIN_OVERRIDES: usize = 256;

impl StoreTuning {
    /// The worker's arguments for this tuning (none when nothing is set).
    pub fn args(&self) -> Vec<String> {
        if *self == StoreTuning::default() {
            return Vec::new();
        }
        serde_json::to_string(self)
            .map(|json| vec![format!("{TUNING_ARG}{json}")])
            .unwrap_or_default()
    }

    /// The tuning the worker's arguments carry: none when absent; an unreadable one is ignored with a message on
    /// standard error (the worker's log), never a failed start.
    pub fn from_args(args: impl IntoIterator<Item = String>) -> StoreTuning {
        let Some(json) = args.into_iter().find_map(|a| a.strip_prefix(TUNING_ARG).map(str::to_string)) else {
            return StoreTuning::default();
        };
        if json.len() > MAX_TUNING_ARG {
            eprintln!("imt-store-worker: the tuning argument is too long and is ignored");
            return StoreTuning::default();
        }
        match serde_json::from_str::<StoreTuning>(&json) {
            Ok(t) => t.bounded(),
            Err(e) => {
                eprintln!("imt-store-worker: the tuning argument is ignored ({e})");
                StoreTuning::default()
            }
        }
    }

    /// Every number within its bounds (threads 1..=256, sizes 1 MB..=1 TiB) and a relative spill folder dropped.
    pub fn bounded(self) -> StoreTuning {
        let threads = |n: Option<usize>| n.map(|n| n.clamp(1, MAX_TUNING_THREADS));
        let mb = |n: Option<u64>| n.map(|n| n.clamp(1, MAX_TUNING_MB));
        StoreTuning {
            parse_threads: threads(self.parse_threads),
            duck_threads: threads(self.duck_threads),
            memory_limit_mb: mb(self.memory_limit_mb),
            index_heap_mb: mb(self.index_heap_mb),
            index_threads: threads(self.index_threads),
            spill_dir: self.spill_dir.filter(|d| d.is_absolute()),
            max_spill_mb: mb(self.max_spill_mb),
            allocator_flush_mb: mb(self.allocator_flush_mb),
            allocator_background_threads: self.allocator_background_threads,
            allocator_arenas: threads(self.allocator_arenas),
            plugin_memory_mb: mb(self.plugin_memory_mb),
            plugin_memory_overrides_mb: self
                .plugin_memory_overrides_mb
                .into_iter()
                .take(MAX_PLUGIN_OVERRIDES)
                .map(|(id, m)| (id, m.clamp(1, MAX_TUNING_MB)))
                .collect(),
            plugin_cpu_percent: self.plugin_cpu_percent.map(|p| p.clamp(1, 100)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    pub parse_threads: usize,
    pub duck_threads: usize,
    pub memory_limit: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Handle {
    pub store: u64,
    /// The worker's [`WIRE_VERSION`]; the client refuses a worker that speaks another.
    pub wire: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Session {
    pub session: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Paths {
    pub paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Source {
    pub id: SourceId,
}

/// Saves the project and writes it as an `.imtz` to `out`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportProject {
    pub manifest: imt_core::package::ProjectManifest,
    pub out: PathBuf,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct IdsAt {
    pub from: u64,
    pub len: usize,
}

/// What the worker process is doing (tests and the supervisor's view).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkerStatus {
    pub pid: u32,
    pub rss_bytes: u64,
    pub stores: usize,
    /// The mode the worker started in.
    #[serde(default)]
    pub memory_mode: MemoryMode,
    /// The tuning the worker started with.
    #[serde(default)]
    pub tuning: StoreTuning,
}

/// A [`StoreError`] on the wire.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum WireError {
    Unsupported {
        operation: String,
        phase: String,
    },
    Invalid {
        message: String,
    },
    /// An invalid query-language text, with the span to underline.
    Query {
        message: String,
        offset: usize,
        len: usize,
        suggestions: Vec<String>,
    },
    NotFound {
        message: String,
    },
    Busy {
        message: String,
    },
    Cancelled,
    Io {
        message: String,
    },
    Engine {
        message: String,
    },
    Damaged {
        message: String,
    },
    Locked {
        message: String,
    },
}

impl From<&StoreError> for WireError {
    fn from(e: &StoreError) -> Self {
        match e {
            StoreError::Unsupported { operation, phase } => WireError::Unsupported {
                operation: operation.to_string(),
                phase: phase.to_string(),
            },
            StoreError::Invalid(m) => WireError::Invalid { message: m.clone() },
            StoreError::Query(q) => WireError::Query {
                message: q.message.clone(),
                offset: q.offset,
                len: q.len,
                suggestions: q.suggestions.clone(),
            },
            StoreError::NotFound(m) => WireError::NotFound { message: m.clone() },
            StoreError::Busy(m) => WireError::Busy { message: m.clone() },
            StoreError::Cancelled => WireError::Cancelled,
            StoreError::Io(m) => WireError::Io { message: m.clone() },
            StoreError::Engine(m) => WireError::Engine { message: m.clone() },
            StoreError::Damaged(m) => WireError::Damaged { message: m.clone() },
            StoreError::Locked(m) => WireError::Locked { message: m.clone() },
        }
    }
}

impl From<WireError> for StoreError {
    fn from(e: WireError) -> Self {
        match e {
            WireError::Unsupported { operation, phase } => StoreError::Unsupported {
                operation: Cow::Owned(operation),
                phase: Cow::Owned(phase),
            },
            WireError::Invalid { message } => StoreError::Invalid(message),
            WireError::Query {
                message,
                offset,
                len,
                suggestions,
            } => StoreError::Query(imt_core::ql::QlError {
                message,
                offset,
                len,
                suggestions,
            }),
            WireError::NotFound { message } => StoreError::NotFound(message),
            WireError::Busy { message } => StoreError::Busy(message),
            WireError::Cancelled => StoreError::Cancelled,
            WireError::Io { message } => StoreError::Io(message),
            WireError::Engine { message } => StoreError::Engine(message),
            WireError::Damaged { message } => StoreError::Damaged(message),
            WireError::Locked { message } => StoreError::Locked(message),
        }
    }
}

/// A store error as the typed error of a failed call.
pub fn to_remote(e: &StoreError) -> RemoteError {
    let mut r = RemoteError::new(STORE_ERROR, e.to_string());
    r.data = serde_json::to_value(WireError::from(e)).ok().map(Into::into);
    r
}

/// The store error a typed error carries (a malformed one is an engine error with its message).
pub fn from_remote(r: RemoteError) -> StoreError {
    r.data
        .and_then(|d| serde_json::from_value::<WireError>(d.into_inner()).ok())
        .map(StoreError::from)
        .unwrap_or(StoreError::Engine(r.message))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire is the port table's row order: a release only ever appends operations. Reordering or removing one
    /// changes the encoding, so it bumps `WIRE_VERSION` and this snapshot.
    #[test]
    fn the_wire_format_is_append_only() {
        const V1: &[&str] = &[
            "Clear",
            "Open",
            "Reload",
            "Refresh",
            "RemoveSource",
            "Restore",
            "AppendTail",
            "Summary",
        ];
        assert_eq!(WIRE_VERSION, 1);
        let names = wire_names();
        assert_eq!(&names[..V1.len()], V1, "the operations' order is the wire encoding");
        let unique: std::collections::HashSet<_> = names.iter().collect();
        assert_eq!(unique.len(), names.len(), "an operation is listed twice");
        let ping = imt_ipc::wire::encode(&StoreOp::Clear).unwrap();
        assert_eq!(ping, [0], "the first operation encodes as index 0");
    }

    #[test]
    fn the_memory_mode_goes_to_the_worker_as_an_argument() {
        for mode in [MemoryMode::Normal, MemoryMode::Low] {
            assert_eq!(MemoryMode::from_args(mode.args()), mode);
        }
        assert_eq!(MemoryMode::default(), MemoryMode::Normal, "off by default");
        let bootstrap = ["imt-store-worker", "--bootstrap", "f", "--low-memory"].map(String::from);
        assert_eq!(MemoryMode::from_args(bootstrap), MemoryMode::Low);
        let old: WorkerStatus = serde_json::from_str(r#"{"pid":1,"rssBytes":2,"stores":0}"#).unwrap();
        assert_eq!(old.memory_mode, MemoryMode::Normal);
    }

    #[test]
    fn the_tuning_goes_to_the_worker_as_one_bounded_argument() {
        assert!(StoreTuning::default().args().is_empty(), "nothing set: no argument");
        let t = StoreTuning {
            parse_threads: Some(6),
            memory_limit_mb: Some(256),
            spill_dir: Some(std::env::temp_dir().join("spill")),
            allocator_background_threads: Some(false),
            ..StoreTuning::default()
        };
        let mut args = vec!["imt-store-worker".to_string(), "--low-memory".into()];
        args.extend(t.args());
        assert_eq!(StoreTuning::from_args(args), t);
        assert_eq!(StoreTuning::from_args(["x".to_string()]), StoreTuning::default());
        // Unreadable or oversized: ignored, never a failed start.
        assert_eq!(StoreTuning::from_args([format!("{TUNING_ARG}{{nope")]), StoreTuning::default());
        let long = format!("{TUNING_ARG}{{\"spillDir\":\"{}\"}}", "a".repeat(MAX_TUNING_ARG));
        assert_eq!(StoreTuning::from_args([long]), StoreTuning::default());
        // Out-of-range numbers are bounded; a relative spill folder is dropped.
        let wild = StoreTuning::from_args([format!(
            r#"{TUNING_ARG}{{"parseThreads":0,"duckThreads":100000,"memoryLimitMb":0,"maxSpillMb":99999999999,"spillDir":"rel","pluginCpuPercent":250,"pluginMemoryOverridesMb":{{"a":0}}}}"#
        )]);
        assert_eq!(
            (
                wild.parse_threads,
                wild.duck_threads,
                wild.memory_limit_mb,
                wild.max_spill_mb,
                wild.spill_dir
            ),
            (Some(1), Some(MAX_TUNING_THREADS), Some(1), Some(MAX_TUNING_MB), None)
        );
        assert_eq!((wild.plugin_cpu_percent, wild.plugin_memory_overrides_mb["a"]), (Some(100), 1));
        let arenas = |n: u32| StoreTuning::from_args([format!(r#"{TUNING_ARG}{{"allocatorArenas":{n}}}"#)]).allocator_arenas;
        assert_eq!(
            (arenas(0), arenas(2), arenas(100_000)),
            (Some(1), Some(2), Some(MAX_TUNING_THREADS))
        );
        let old: WorkerStatus = serde_json::from_str(r#"{"pid":1,"rssBytes":2,"stores":0}"#).unwrap();
        assert_eq!(old.tuning, StoreTuning::default());
    }

    #[test]
    fn store_errors_survive_the_wire() {
        for e in [
            StoreError::unsupported("chart", "R2c"),
            StoreError::Invalid("bad".into()),
            StoreError::NotFound("source 3".into()),
            StoreError::Busy("import".into()),
            StoreError::Cancelled,
            StoreError::Io("disk".into()),
            StoreError::Engine("x".into()),
            StoreError::Query(imt_core::ql::QlError {
                message: "unknown field".into(),
                offset: 3,
                len: 4,
                suggestions: vec!["Host".into()],
            }),
        ] {
            let r = to_remote(&e);
            let back: RemoteError = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
            assert_eq!(from_remote(back), e);
        }
        assert_eq!(from_remote(RemoteError::new(1, "plain")), StoreError::Engine("plain".into()));
    }
}
