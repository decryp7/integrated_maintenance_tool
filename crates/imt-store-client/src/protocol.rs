//! The worker protocol over IMT's one IPC (ADR 0017): every call is a [`StoreRequest`] frame body (postcard);
//! the operation is typed ([`StoreOp`]), the port DTOs it carries and answers with cross as JSON text (they are
//! the ports' JSON-shaped types, unchanged). A failed call answers a [`imt_ipc::RemoteError`] whose `data` is a
//! [`WireError`], so the client gets the same [`StoreError`] back; anything else (a crash, a hang, garbage, an
//! oversized frame) is a failure of the worker process.

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

macro_rules! ops {
    ($($(#[$doc:meta])* $op:ident),* $(,)?) => {
        /// An operation on one open store; its request and reply are the port method's DTOs (as JSON, see the
        /// module documentation). The order is the wire encoding: add new operations at the end.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum StoreOp {
            $($(#[$doc])* $op),*
        }

        impl StoreOp {
            /// Every operation (tests: each one is dispatched).
            pub const ALL: &'static [StoreOp] = &[$(StoreOp::$op),*];

            /// The operation's name (logs).
            pub fn name(self) -> &'static str {
                match self {
                    $(StoreOp::$op => stringify!($op)),*
                }
            }
        }
    };
}

ops! {
    // Ingest
    /// [`Paths`] → [`Session`]
    Open,
    /// → [`Session`]
    Reload,
    /// [`Paths`] → [`Session`]
    Refresh,
    /// [`Source`] → ()
    RemoveSource,
    Clear,
    /// [`imt_core::api::WorkspaceState`] → [`Session`]
    Restore,
    /// [`Paths`] → events appended
    AppendTail,
    /// [`Session`] → [`imt_core::workspace::ProgressSnapshot`] of the store's running (or last) import
    SessionProgress,
    /// [`Session`] → ()
    SessionCancel,
    /// [`Session`] → [`imt_core::ports::ImportReport`]
    SessionWait,
    // Query
    Summary,
    Page,
    Timeline,
    Facet,
    Chart,
    FieldStats,
    SourceContext,
    /// Cancellable (the caller's cancel interrupts the statement).
    RunQuery,
    CheckQuery,
    CompleteQuery,
    Export,
    // Positions
    IdsAt,
    PositionOf,
    Positioned,
    // Analysis (cancellable)
    Patterns,
    ExceptionGroups,
    Anomalies,
    Compare,
    Correlate,
    Traces,
    Requests,
    Moment,
    Overview,
    Runs,
    // Profiles
    ActiveProfile,
    ValidateProfile,
    /// [`Profile`] → [`Session`]
    SetProfile,
    PreviewParse,
    PreviewExtractor,
    ProposeSpanRule,
    EvaluateSpanRule,
    // Diagnostics
    Problems,
    EvaluateFix,
    // Search
    Search,
    IndexStatus,
    // Projects
    /// [`imt_core::package::ProjectManifest`] → the manifest as saved (with the store's roots)
    SaveProject,
    /// [`ExportProject`] → bytes written
    ExportProject,
    /// [`imt_core::ports::EventsRequest`] → the events
    Events,
    /// [`imt_core::spans::SpanPreviewRequest`] → [`imt_core::spans::SpanPreview`]
    PreviewSpans,
    /// → [`imt_core::ports::ParserUse`]s
    Parsers,
    /// [`imt_core::ports::ParserDef`] → [`imt_core::ports::ParserChange`]
    ClassifyParser,
    /// [`AssignParser`] → [`Session`]
    AssignParser,
    /// [`imt_core::ports::Filter`] → [`imt_core::ports::PartialMatches`] (cancellable)
    PartialMatches,
    /// [`imt_core::parser::ParserSetup`] → the stale [`SourceId`]s (ADR 0026)
    SetParsers,
    /// → [`Session`]: re-reads the stale sources and the files no parser read
    RereadParsers,
    // The Store pane (ADR 0030)
    /// → [`imt_core::ports::StoreOverview`]
    StoreOverview,
    /// [`imt_core::ports::SegmentsRequest`] → [`imt_core::ports::SegmentsPage`]
    StoreSegments,
    /// → [`imt_core::ports::StoreFields`]
    StoreFields,
    /// → [`imt_core::ports::StoreIndex`]
    StoreIndex,
    /// → [`imt_core::ports::StoreMemory`], with the worker's process filled in
    StoreMemory,
    /// [`imt_core::ports::MaintenanceAction`] → [`imt_core::ports::MaintenanceReport`]
    StoreMaintain,
    /// → [`imt_core::ports::SqlSchema`]
    SqlSchema,
    /// [`imt_core::ports::SqlRequest`] → [`imt_core::ports::SqlResult`] (cancellable)
    SqlRun,
    /// [`imt_core::ports::SqlPageRequest`] → [`imt_core::ports::SqlPage`]
    SqlPage,
}

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
