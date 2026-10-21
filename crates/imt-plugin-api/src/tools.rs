//! Tools: typed request/response operations a plugin contributes and the shell dispatches generically
//! (PerfMon control, the process monitor, the diagnostics collector, .NET capture).
//!
//! A [`Tool`] describes itself once ([`ToolInfo`]): its operations, each with an argument [`Schema`], a time
//! budget and whether it needs administrator rights. The host validates the arguments against the schema
//! before the tool sees them — including **path arguments** ([`Schema::Path`]), which must be paths the user
//! chose (the calling window's grants) and arrive canonicalised — and runs the operation under the plugin's
//! guard.
//!
//! Long-running work (a running monitor, a started collector set) is reported through [`ToolCtx`]'s task
//! methods so the shell can list it (tray, status bar) and stop it later ([`Tool::stop`]).

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Budget of an operation that declares none.
pub const DEFAULT_OP_BUDGET: Duration = Duration::from_secs(30);
/// Largest argument document accepted (serialized JSON).
pub const MAX_ARGS_BYTES: usize = 1 << 20;
/// Longest tool id, operation and task id.
pub const MAX_ID_LEN: usize = 64;
/// Longest path argument, in characters.
pub const MAX_PATH_LEN: usize = 4096;

/// An operating system a tool can run on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    /// Microsoft Windows.
    Windows,
    /// Apple macOS.
    Macos,
    /// Linux and other Unixes.
    Linux,
}

impl Platform {
    /// The platform this binary was built for (other Unixes count as Linux).
    pub const fn current() -> Platform {
        #[cfg(windows)]
        return Platform::Windows;
        #[cfg(target_os = "macos")]
        return Platform::Macos;
        #[cfg(not(any(windows, target_os = "macos")))]
        return Platform::Linux;
    }

    /// The platform's display name.
    pub fn label(self) -> &'static str {
        match self {
            Platform::Windows => "Windows",
            Platform::Macos => "macOS",
            Platform::Linux => "Linux",
        }
    }
}

/// What a path argument names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PathKind {
    /// A file.
    File,
    /// A folder.
    Dir,
}

/// What a tool does with a path argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PathAccess {
    /// The tool reads the path.
    Read,
    /// The tool writes to the path.
    Write,
}

/// The shape of an operation's arguments. Checked by the host before the tool is called; the tool still
/// parses them into its own types.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Schema {
    /// Any JSON value; the tool validates it itself (e.g. a nested config).
    Any,
    /// A boolean.
    Bool,
    /// A whole number in `min..=max`.
    Int {
        /// Smallest accepted value.
        min: i64,
        /// Largest accepted value.
        max: i64,
    },
    /// Text of at most `max_len` characters.
    Text {
        /// Longest accepted text, in characters.
        #[serde(rename = "maxLen")]
        max_len: usize,
    },
    /// One of these strings.
    Enum {
        /// The accepted strings.
        values: Vec<String>,
    },
    /// A list of items that all match `items`.
    List {
        /// The schema every item must match.
        items: Box<Schema>,
        /// Most items accepted.
        #[serde(rename = "maxItems")]
        max_items: usize,
    },
    /// An object with the named members.
    Object {
        /// The accepted members.
        fields: Vec<Field>,
        /// Whether names not in `fields` are accepted.
        additional: bool,
    },
    /// An absolute path the user chose: the host checks it against the calling window's path grants and
    /// passes it on canonicalised. A path the user did not choose is refused before the tool runs.
    Path {
        /// Whether the path is a file or a folder.
        kind: PathKind,
        /// Whether the tool reads or writes it.
        access: PathAccess,
    },
}

/// One named member of an [`Schema::Object`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    /// The member's name.
    pub name: String,
    /// The schema its value must match.
    pub schema: Schema,
    /// Optional fields may be missing or `null`.
    pub required: bool,
}

impl Schema {
    /// Text of at most `max_len` characters.
    pub fn text(max_len: usize) -> Schema {
        Schema::Text { max_len }
    }

    /// A whole number in `min..=max`.
    pub fn int(min: i64, max: i64) -> Schema {
        Schema::Int { min, max }
    }

    /// One of `values`.
    pub fn one_of<I: IntoIterator<Item = S>, S: Into<String>>(values: I) -> Schema {
        Schema::Enum {
            values: values.into_iter().map(Into::into).collect(),
        }
    }

    /// A list of at most `max_items` items that match `items`.
    pub fn list(items: Schema, max_items: usize) -> Schema {
        Schema::List {
            items: Box::new(items),
            max_items,
        }
    }

    /// A path the user chose, read by the tool.
    pub fn path(kind: PathKind, access: PathAccess) -> Schema {
        Schema::Path { kind, access }
    }

    /// An object without fields that refuses unknown names (add fields with [`Schema::field`] and
    /// [`Schema::optional`]).
    pub fn object() -> Schema {
        Schema::Object {
            fields: Vec::new(),
            additional: false,
        }
    }

    fn with_field(mut self, name: &str, schema: Schema, required: bool) -> Schema {
        if let Schema::Object { fields, .. } = &mut self {
            fields.push(Field {
                name: name.into(),
                schema,
                required,
            });
        }
        self
    }

    /// Adds a required field (objects only; ignored otherwise).
    pub fn field(self, name: &str, schema: Schema) -> Schema {
        self.with_field(name, schema, true)
    }

    /// Adds an optional field (objects only; ignored otherwise).
    pub fn optional(self, name: &str, schema: Schema) -> Schema {
        self.with_field(name, schema, false)
    }

    /// Accepts names that are not declared (objects only).
    pub fn open(mut self) -> Schema {
        if let Schema::Object { additional, .. } = &mut self {
            *additional = true;
        }
        self
    }

    /// Checks `value`; the error names the offending location (`args.template.name`).
    pub fn validate(&self, value: &Value) -> std::result::Result<(), SchemaError> {
        self.check(value, "args")
    }

    fn check(&self, value: &Value, at: &str) -> std::result::Result<(), SchemaError> {
        let fail = |message: String| Err(SchemaError { at: at.into(), message });
        match (self, value) {
            (Schema::Any, _) | (Schema::Bool, Value::Bool(_)) => Ok(()),
            (Schema::Int { min, max }, Value::Number(n)) => match n.as_i64() {
                Some(i) if (*min..=*max).contains(&i) => Ok(()),
                Some(i) => fail(format!("must be between {min} and {max} (got {i})")),
                None => fail(format!("must be a whole number between {min} and {max}")),
            },
            (Schema::Text { max_len }, Value::String(s)) => match s.chars().count() {
                n if n > *max_len => fail(format!("must be at most {max_len} characters (got {n})")),
                _ => Ok(()),
            },
            (Schema::Path { .. }, Value::String(s)) => {
                if s.chars().count() > MAX_PATH_LEN || s.contains('\0') {
                    fail(format!("must be a path of at most {MAX_PATH_LEN} characters"))
                } else if !Path::new(s).is_absolute() {
                    fail("must be an absolute path".into())
                } else {
                    Ok(())
                }
            }
            (Schema::Enum { values }, Value::String(s)) if values.contains(s) => Ok(()),
            (Schema::Enum { values }, Value::String(s)) => fail(format!("\"{s}\" is not one of: {}", values.join(", "))),
            (Schema::List { items, max_items }, Value::Array(a)) => {
                if a.len() > *max_items {
                    return fail(format!("must have at most {max_items} items (got {})", a.len()));
                }
                a.iter().enumerate().try_for_each(|(i, v)| items.check(v, &format!("{at}[{i}]")))
            }
            (Schema::Object { fields, additional }, Value::Object(map)) => {
                for f in fields {
                    match map.get(&f.name) {
                        None | Some(Value::Null) if f.required => {
                            return Err(SchemaError {
                                at: format!("{at}.{}", f.name),
                                message: "is required".into(),
                            })
                        }
                        None | Some(Value::Null) => {}
                        Some(v) => f.schema.check(v, &format!("{at}.{}", f.name))?,
                    }
                }
                match map.keys().find(|k| !*additional && !fields.iter().any(|f| &f.name == *k)) {
                    Some(k) => Err(SchemaError {
                        at: format!("{at}.{k}"),
                        message: "is not a known argument".into(),
                    }),
                    None => Ok(()),
                }
            }
            (expected, got) => fail(format!("must be {}, not {}", expected.kind(), kind_of(got))),
        }
    }

    /// Checks every [`Schema::Path`] argument in `value` (which already passed [`Schema::validate`]) with
    /// `check` and replaces it by what `check` returns (the canonical path).
    pub fn check_paths(
        &self,
        value: &mut Value,
        check: &dyn Fn(&str, PathKind, PathAccess) -> std::result::Result<PathBuf, String>,
    ) -> std::result::Result<(), SchemaError> {
        self.paths_at(value, "args", check)
    }

    fn paths_at(
        &self,
        value: &mut Value,
        at: &str,
        check: &dyn Fn(&str, PathKind, PathAccess) -> std::result::Result<PathBuf, String>,
    ) -> std::result::Result<(), SchemaError> {
        match (self, value) {
            (Schema::Path { kind, access }, Value::String(s)) => {
                let p = check(s, *kind, *access).map_err(|message| SchemaError { at: at.into(), message })?;
                *s = p.to_string_lossy().into_owned();
                Ok(())
            }
            (Schema::List { items, .. }, Value::Array(a)) => a
                .iter_mut()
                .enumerate()
                .try_for_each(|(i, v)| items.paths_at(v, &format!("{at}[{i}]"), check)),
            (Schema::Object { fields, .. }, Value::Object(map)) => {
                for f in fields {
                    if let Some(v) = map.get_mut(&f.name) {
                        if !v.is_null() {
                            f.schema.paths_at(v, &format!("{at}.{}", f.name), check)?;
                        }
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Whether the schema has a path argument anywhere.
    pub fn has_paths(&self) -> bool {
        match self {
            Schema::Path { .. } => true,
            Schema::List { items, .. } => items.has_paths(),
            Schema::Object { fields, .. } => fields.iter().any(|f| f.schema.has_paths()),
            _ => false,
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Schema::Any => "any value",
            Schema::Bool => "true or false",
            Schema::Int { .. } => "a whole number",
            Schema::Text { .. } | Schema::Enum { .. } => "text",
            Schema::Path { .. } => "a path",
            Schema::List { .. } => "a list",
            Schema::Object { .. } => "an object",
        }
    }
}

fn kind_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "true/false",
        Value::Number(_) => "a number",
        Value::String(_) => "text",
        Value::Array(_) => "a list",
        Value::Object(_) => "an object",
    }
}

/// Where and why arguments do not match an operation's schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaError {
    /// `args`, `args.template.name`, `args.counters[2]`, ...
    pub at: String,
    /// What is wrong with the value.
    pub message: String,
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.at, self.message)
    }
}

/// One operation of a tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpSpec {
    /// Operation id, as passed to `invoke`.
    pub name: String,
    /// Name shown to the user.
    pub title: String,
    /// The schema its arguments must match.
    pub args: Schema,
    /// Time the operation may take before it is abandoned and its plugin marked hung; `None` means unlimited.
    #[serde(rename = "budgetMs", with = "millis")]
    pub budget: Option<Duration>,
    /// It needs administrator rights: the host runs it in the plugin's elevated process (ADR 0017 §7),
    /// started through the administrator prompt on first use.
    #[serde(default)]
    pub elevated: bool,
    /// A read-only status operation the UI polls: the shell runs it through the quiet `tool_poll`
    /// (not logged per call) and refuses `tool_poll` for any other operation.
    #[serde(default)]
    pub poll: bool,
}

mod millis {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(d: &Option<Duration>, s: S) -> Result<S::Ok, S::Error> {
        match d {
            Some(d) => s.serialize_some(&(d.as_millis() as u64)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
        Ok(Option::<u64>::deserialize(d)?.map(Duration::from_millis))
    }
}

impl OpSpec {
    /// An operation taking no arguments, with [`DEFAULT_OP_BUDGET`].
    pub fn new(name: &str, title: &str) -> OpSpec {
        OpSpec {
            name: name.into(),
            title: title.into(),
            args: Schema::object(),
            budget: Some(DEFAULT_OP_BUDGET),
            elevated: false,
            poll: false,
        }
    }

    /// Marks a read-only status operation that may be polled (see [`OpSpec::poll`]).
    pub fn polling(mut self) -> OpSpec {
        self.poll = true;
        self
    }

    /// Sets the argument schema.
    pub fn args(mut self, schema: Schema) -> OpSpec {
        self.args = schema;
        self
    }

    /// Sets the time the operation may take before it is abandoned; `None` means unlimited.
    pub fn budget(mut self, budget: Option<Duration>) -> OpSpec {
        self.budget = budget;
        self
    }

    /// Runs in the plugin's elevated process.
    pub fn elevated(mut self) -> OpSpec {
        self.elevated = true;
        self
    }
}

/// What a tool is and what it can do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    /// Stable id, unique across plugins (`"perfmon.sets"`).
    pub id: String,
    /// Name shown to the user.
    pub title: String,
    /// What the tool does, shown under its title.
    pub description: String,
    /// Platforms it works on; empty means every platform.
    pub platforms: Vec<Platform>,
    /// Usually needs administrator rights (the UI offers an elevated restart).
    pub needs_elevation: bool,
    /// The operations it offers.
    pub ops: Vec<OpSpec>,
}

impl ToolInfo {
    /// Creates a tool with no operations, available on every platform.
    pub fn new(id: &str, title: &str) -> ToolInfo {
        ToolInfo {
            id: id.into(),
            title: title.into(),
            description: String::new(),
            platforms: Vec::new(),
            needs_elevation: false,
            ops: Vec::new(),
        }
    }

    /// Sets the description.
    pub fn describe(mut self, text: &str) -> ToolInfo {
        self.description = text.into();
        self
    }

    /// Restricts the tool to `platforms`.
    pub fn only_on(mut self, platforms: &[Platform]) -> ToolInfo {
        self.platforms = platforms.to_vec();
        self
    }

    /// Marks the tool as usually needing administrator rights.
    pub fn needs_elevation(mut self) -> ToolInfo {
        self.needs_elevation = true;
        self
    }

    /// Adds an operation.
    pub fn op(mut self, op: OpSpec) -> ToolInfo {
        self.ops.push(op);
        self
    }

    /// Ids, operation names and schemas are well formed; the error says what to fix.
    pub fn check(&self) -> Result<(), String> {
        check_id("tool id", &self.id)?;
        if self.ops.is_empty() {
            return Err(format!("tool \"{}\" has no operations", self.id));
        }
        for (i, op) in self.ops.iter().enumerate() {
            check_id(&format!("tool \"{}\": operation name", self.id), &op.name)?;
            if self.ops[..i].iter().any(|o| o.name == op.name) {
                return Err(format!("tool \"{}\": operation \"{}\" is declared twice", self.id, op.name));
            }
        }
        Ok(())
    }

    /// Why the tool cannot run on `platform`, if it cannot.
    pub fn unsupported_on(&self, platform: Platform) -> Option<String> {
        if self.platforms.is_empty() || self.platforms.contains(&platform) {
            return None;
        }
        let on: Vec<&str> = self.platforms.iter().map(|p| p.label()).collect();
        Some(format!(
            "{} is only available on {} (this is {})",
            self.title,
            on.join(" and "),
            platform.label()
        ))
    }
}

/// Checks an id: 1–[`MAX_ID_LEN`] characters of a-z, 0-9, '.', '-', '_' or ':'.
pub fn check_id(what: &str, id: &str) -> Result<(), String> {
    let ok =
        !id.is_empty() && id.len() <= MAX_ID_LEN && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':'));
    if ok {
        Ok(())
    } else {
        Err(format!(
            "{what} \"{id}\" must be 1-{MAX_ID_LEN} characters of a-z, 0-9, '.', '-', '_' or ':'"
        ))
    }
}

/// Background work a tool started (shown in the tray and status bar).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolTask {
    /// Stable id, unique per task (`"monitor"`, `"perfmon:IMT"`).
    pub id: String,
    /// Display name of the task.
    pub label: String,
    /// A hint for the shell's icon (`"process-monitor"`, `"perfmon"`, ...).
    pub kind: String,
    /// Whether [`Tool::stop`] can stop it (the tray then offers Stop).
    pub stoppable: bool,
}

/// Receives a tool's background work. The shell lists it and offers a stop action that calls the tool's
/// [`Tool::stop`] with the same tool and task id.
pub trait TaskSink: Send + Sync {
    /// Called when a task starts.
    fn started(&self, tool: &str, task: &ToolTask);
    /// `None`: unknown; otherwise 0..=1.
    fn progress(&self, tool: &str, task: &str, fraction: Option<f32>);
    /// Called when a task ends (finished, failed or stopped).
    fn finished(&self, tool: &str, task: &str);
}

/// A sink that drops everything (tests, hosts without a task list).
pub struct NoTasks;

impl TaskSink for NoTasks {
    fn started(&self, _: &str, _: &ToolTask) {}
    fn progress(&self, _: &str, _: &str, _: Option<f32>) {}
    fn finished(&self, _: &str, _: &str) {}
}

/// Decides whether a path argument names something the user chose (the shell's path grants).
pub trait PathPolicy: Send + Sync {
    /// The canonical path when `path` is granted for `access`; the error says why not.
    fn check(&self, path: &str, kind: PathKind, access: PathAccess) -> std::result::Result<PathBuf, String>;
}

/// Refuses every path (a context without grants).
pub struct NoPaths;

impl PathPolicy for NoPaths {
    fn check(&self, path: &str, _: PathKind, _: PathAccess) -> std::result::Result<PathBuf, String> {
        Err(format!("\"{path}\" was not chosen in IMT; choose it with a dialog first"))
    }
}

/// Runs the calling plugin's own elevated operations ([`OpSpec::elevated`]) in its elevated process, through the
/// host (ADR 0017 §7): the host starts that process (the administrator prompt on Windows) and accepts only
/// operations the plugin declared elevated. A prompt the user declines is a [`ToolFailure::denied`].
pub trait ElevatedOps: Send + Sync {
    /// Runs one of the plugin's elevated tool operations in its elevated process and returns its result; fails after `timeout`.
    fn invoke(&self, tool: &str, op: &str, args: Value, timeout: Duration) -> Result<Value>;
}

/// What the host gives a tool for one call. Cheap to clone; a tool may keep a clone to report the end of
/// background work it started.
#[derive(Clone)]
pub struct ToolCtx {
    tool: String,
    data_dir: Option<PathBuf>,
    sink: Arc<dyn TaskSink>,
    paths: Arc<dyn PathPolicy>,
    elevated: Option<Arc<dyn ElevatedOps>>,
}

impl ToolCtx {
    /// A context for calls by the host; `data_dir` is the app's data folder (default location of tool
    /// output). Path arguments are refused until [`ToolCtx::with_paths`] gives a policy.
    pub fn new(data_dir: Option<PathBuf>, sink: Arc<dyn TaskSink>) -> ToolCtx {
        ToolCtx {
            tool: String::new(),
            data_dir,
            sink,
            paths: Arc::new(NoPaths),
            elevated: None,
        }
    }

    /// The same context able to run the plugin's elevated operations through `elevated`.
    pub fn with_elevated(mut self, elevated: Arc<dyn ElevatedOps>) -> ToolCtx {
        self.elevated = Some(elevated);
        self
    }

    /// How to run the plugin's elevated operations; `None` where there is no elevated process (in the
    /// elevated process itself, in tests, in hosts without one).
    pub fn elevated(&self) -> Option<&Arc<dyn ElevatedOps>> {
        self.elevated.as_ref()
    }

    /// The same context checking path arguments with `paths`.
    pub fn with_paths(mut self, paths: Arc<dyn PathPolicy>) -> ToolCtx {
        self.paths = paths;
        self
    }

    /// The same context for calls of `tool` (the host scopes it before every call, so task reports name
    /// their tool).
    pub fn for_tool(&self, tool: &str) -> ToolCtx {
        ToolCtx {
            tool: tool.into(),
            ..self.clone()
        }
    }

    /// The tool this context is scoped to ("" before [`ToolCtx::for_tool`]).
    pub fn tool(&self) -> &str {
        &self.tool
    }

    /// The app's data folder, when known.
    pub fn data_dir(&self) -> Option<&Path> {
        self.data_dir.as_deref()
    }

    /// The path policy of the calling window.
    pub fn paths(&self) -> &Arc<dyn PathPolicy> {
        &self.paths
    }

    /// The task sink (for hosts that forward task reports).
    pub fn sink(&self) -> &Arc<dyn TaskSink> {
        &self.sink
    }

    /// Reports background work this tool started.
    pub fn task_started(&self, task: ToolTask) {
        self.sink.started(&self.tool, &task);
    }

    /// Reports progress of `task` from 0 to 1; `None` when unknown.
    pub fn task_progress(&self, task: &str, fraction: Option<f32>) {
        self.sink.progress(&self.tool, task, fraction);
    }

    /// Reports that `task` finished.
    pub fn task_finished(&self, task: &str) {
        self.sink.finished(&self.tool, task);
    }
}

/// A typed operation provider. Every method must honour its contract without panicking; errors are returned
/// as messages for the user (a [`ToolFailure`] in the chain keeps its reason).
pub trait Tool: Send + Sync {
    /// The same id as [`ToolInfo::id`].
    fn id(&self) -> &str;
    /// Asked once, at registration.
    fn describe(&self) -> ToolInfo;
    /// Runs `op` with `args` that already passed the op's schema (paths checked and canonical). Returns the
    /// operation's JSON result.
    fn invoke(&self, op: &str, args: Value, ctx: &ToolCtx) -> Result<Value>;
    /// Stops background work this tool reported through [`ToolCtx::task_started`].
    fn stop(&self, task: &str, ctx: &ToolCtx) -> Result<()> {
        let _ = ctx;
        anyhow::bail!("tool \"{}\" has no background task \"{task}\" to stop", self.id())
    }
}

/// Why a tool's operation failed, when the tool knows it (see [`ToolFailure`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FailureReason {
    /// Missing rights.
    Denied,
    /// A program it runs is not installed.
    Missing,
}

/// A typed failure a tool returns (inside its `anyhow::Error`) so the host reports it as denied or missing
/// instead of a plain failure; the shell never reads the message to tell them apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolFailure {
    /// Why it failed.
    pub reason: FailureReason,
    /// Message shown to the user.
    pub message: String,
}

impl ToolFailure {
    /// A failure of kind [`FailureReason::Denied`].
    pub fn denied(message: impl Into<String>) -> ToolFailure {
        ToolFailure {
            reason: FailureReason::Denied,
            message: message.into(),
        }
    }

    /// A failure of kind [`FailureReason::Missing`].
    pub fn missing(message: impl Into<String>) -> ToolFailure {
        ToolFailure {
            reason: FailureReason::Missing,
            message: message.into(),
        }
    }
}

impl fmt::Display for ToolFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ToolFailure {}

/// The reason a failed call carries: a [`ToolFailure`] or an OS "permission denied" in its chain.
pub fn failure_reason(e: &anyhow::Error) -> Option<FailureReason> {
    e.chain().find_map(|c| {
        if let Some(t) = c.downcast_ref::<ToolFailure>() {
            return Some(t.reason);
        }
        c.downcast_ref::<std::io::Error>()
            .filter(|io| io.kind() == std::io::ErrorKind::PermissionDenied)
            .map(|_| FailureReason::Denied)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn path_arguments_are_validated_and_checked_by_the_host() {
        let s = Schema::object()
            .field("xmlPath", Schema::path(PathKind::File, PathAccess::Write))
            .optional("inputs", Schema::list(Schema::path(PathKind::File, PathAccess::Read), 4));
        assert!(s.has_paths() && !Schema::object().field("x", Schema::Bool).has_paths());
        let abs = if cfg!(windows) { "C:\\out\\set.xml" } else { "/out/set.xml" };
        let mut args = json!({"xmlPath": abs, "inputs": [abs]});
        s.validate(&args).unwrap();
        // Relative paths and NULs are refused by the schema itself.
        assert_eq!(
            s.validate(&json!({"xmlPath": "rel/x"})).unwrap_err().message,
            "must be an absolute path"
        );
        assert!(s.validate(&json!({"xmlPath": format!("{abs}\0")})).is_err());
        // The host's policy decides, and its canonical answer replaces the argument.
        let seen = std::sync::Mutex::new(Vec::new());
        s.check_paths(&mut args, &|p, k, a| {
            seen.lock().unwrap().push((p.to_string(), k, a));
            Ok(PathBuf::from(format!("{p}.canonical")))
        })
        .unwrap();
        assert_eq!(args["xmlPath"], format!("{abs}.canonical"));
        assert_eq!(args["inputs"][0], format!("{abs}.canonical"));
        assert_eq!(seen.lock().unwrap()[0].2, PathAccess::Write);
        let err = s
            .check_paths(&mut json!({"xmlPath": abs}), &|p, k, a| NoPaths.check(p, k, a))
            .unwrap_err();
        assert_eq!(err.at, "args.xmlPath");
        assert!(err.message.contains("not chosen"));
    }

    #[test]
    fn descriptions_roundtrip_as_json() {
        let info = ToolInfo::new("perfmon.templates", "PerfMon")
            .describe("d")
            .only_on(&[Platform::Windows])
            .needs_elevation()
            .op(OpSpec::new("generate", "Generate")
                .args(Schema::object().field("n", Schema::int(1, 5)).open())
                .budget(None)
                .elevated());
        info.check().unwrap();
        let back: ToolInfo = serde_json::from_value(serde_json::to_value(&info).unwrap()).unwrap();
        assert_eq!(back, info);
        assert!(back.ops[0].elevated && back.ops[0].budget.is_none());
        assert!(info.unsupported_on(Platform::Linux).unwrap().contains("only available on Windows"));
        assert_eq!(info.unsupported_on(Platform::Windows), None);
        assert!(ToolInfo::new("x", "X").check().is_err());
        assert!(check_id("id", "bad id").is_err());
    }

    #[test]
    fn contexts_scope_tasks_and_failures_keep_their_reason() {
        struct Rec(std::sync::Mutex<Vec<String>>);
        impl TaskSink for Rec {
            fn started(&self, tool: &str, task: &ToolTask) {
                self.0.lock().unwrap().push(format!("{tool}:{}", task.id));
            }
            fn progress(&self, tool: &str, task: &str, _: Option<f32>) {
                self.0.lock().unwrap().push(format!("{tool}:{task}:p"));
            }
            fn finished(&self, tool: &str, task: &str) {
                self.0.lock().unwrap().push(format!("{tool}:{task}:f"));
            }
        }
        let rec = Arc::new(Rec(Default::default()));
        let ctx = ToolCtx::new(None, rec.clone()).for_tool("t");
        assert_eq!(ctx.tool(), "t");
        ctx.task_started(ToolTask {
            id: "a".into(),
            label: "A".into(),
            kind: "k".into(),
            stoppable: true,
        });
        ctx.task_progress("a", Some(0.5));
        ctx.task_finished("a");
        assert_eq!(*rec.0.lock().unwrap(), ["t:a", "t:a:p", "t:a:f"]);
        assert!(ctx.data_dir().is_none());
        let e = anyhow::Error::new(ToolFailure::denied("no rights")).context("starting");
        assert_eq!(failure_reason(&e), Some(FailureReason::Denied));
        let io = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        assert_eq!(failure_reason(&io), Some(FailureReason::Denied));
        assert_eq!(failure_reason(&anyhow::anyhow!("x")), None);
        assert_eq!(ToolFailure::missing("m").to_string(), "m");
    }
}
