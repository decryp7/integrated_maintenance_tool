//! Tools: typed request/response operations a plugin contributes and the
//! shell dispatches generically (ADR 0002), such as controlling Performance
//! Monitor, running the process monitor or collecting a diagnostics bundle.
//!
//! A [`Tool`] describes itself once ([`ToolInfo`]): its operations, each with
//! an argument [`Schema`] and a time budget, the platforms it runs on and
//! whether it usually needs administrator rights. The host
//! ([`ToolHost`], from [`crate::PluginRegistry::tools`]) then
//!
//! * lists every tool with its availability: available, unsupported on this
//!   platform (with the reason), or refused because its plugin is inactive,
//!   quarantined or not responding (with the plugin's reason);
//! * validates the arguments against the operation's schema before the tool
//!   sees them, so a tool only parses well-formed input;
//! * runs the operation under the plugin's guard on a helper thread: a panic
//!   becomes an error counted towards quarantine, and an operation that runs
//!   past its budget is abandoned and marks the plugin hung.
//!
//! Long-running work (a running monitor, a started collector set) is reported
//! through [`ToolCtx::tasks`] so the shell can list it (tray, status bar) and
//! stop it later through [`ToolHost::stop`], which calls [`Tool::stop`].

use crate::guard::{CallKind, Guard};
use anyhow::Result;
use serde::Serialize;
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

/// An operating system a tool can run on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Windows,
    Macos,
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

    pub fn label(self) -> &'static str {
        match self {
            Platform::Windows => "Windows",
            Platform::Macos => "macOS",
            Platform::Linux => "Linux",
        }
    }
}

/// The shape of an operation's arguments. Checked by the host before the
/// tool is called; the tool still parses them into its own types.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Schema {
    /// Any JSON value; the tool validates it itself (e.g. a nested config).
    Any,
    Bool,
    Int {
        min: i64,
        max: i64,
    },
    Text {
        #[serde(rename = "maxLen")]
        max_len: usize,
    },
    /// One of these strings.
    Enum {
        values: Vec<String>,
    },
    List {
        items: Box<Schema>,
        #[serde(rename = "maxItems")]
        max_items: usize,
    },
    Object {
        fields: Vec<Field>,
        /// Whether names not in `fields` are accepted.
        additional: bool,
    },
}

/// One named member of an [`Schema::Object`].
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    pub name: String,
    pub schema: Schema,
    /// Optional fields may be missing or `null`.
    pub required: bool,
}

impl Schema {
    pub fn text(max_len: usize) -> Schema {
        Schema::Text { max_len }
    }

    pub fn int(min: i64, max: i64) -> Schema {
        Schema::Int { min, max }
    }

    pub fn one_of<I: IntoIterator<Item = S>, S: Into<String>>(values: I) -> Schema {
        Schema::Enum {
            values: values.into_iter().map(Into::into).collect(),
        }
    }

    pub fn list(items: Schema, max_items: usize) -> Schema {
        Schema::List {
            items: Box::new(items),
            max_items,
        }
    }

    /// An object without fields that refuses unknown names (add fields with
    /// [`Schema::field`] and [`Schema::optional`]).
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

    fn kind(&self) -> &'static str {
        match self {
            Schema::Any => "any value",
            Schema::Bool => "true or false",
            Schema::Int { .. } => "a whole number",
            Schema::Text { .. } | Schema::Enum { .. } => "text",
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
    pub message: String,
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.at, self.message)
    }
}

/// One operation of a tool.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpSpec {
    pub name: String,
    pub title: String,
    pub args: Schema,
    /// Time the operation may take before it is abandoned and its plugin
    /// marked hung; `None` means unlimited.
    #[serde(rename = "budgetMs", serialize_with = "millis")]
    pub budget: Option<Duration>,
}

fn millis<S: serde::Serializer>(d: &Option<Duration>, s: S) -> std::result::Result<S::Ok, S::Error> {
    match d {
        Some(d) => s.serialize_some(&(d.as_millis() as u64)),
        None => s.serialize_none(),
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
        }
    }

    pub fn args(mut self, schema: Schema) -> OpSpec {
        self.args = schema;
        self
    }

    pub fn budget(mut self, budget: Option<Duration>) -> OpSpec {
        self.budget = budget;
        self
    }
}

/// What a tool is and what it can do.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    /// Stable id, unique across plugins (`"perfmon.sets"`).
    pub id: String,
    pub title: String,
    pub description: String,
    /// Platforms it works on; empty means every platform.
    pub platforms: Vec<Platform>,
    /// Usually needs administrator rights (the UI offers an elevated restart).
    pub needs_elevation: bool,
    pub ops: Vec<OpSpec>,
}

impl ToolInfo {
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

    pub fn describe(mut self, text: &str) -> ToolInfo {
        self.description = text.into();
        self
    }

    pub fn only_on(mut self, platforms: &[Platform]) -> ToolInfo {
        self.platforms = platforms.to_vec();
        self
    }

    pub fn needs_elevation(mut self) -> ToolInfo {
        self.needs_elevation = true;
        self
    }

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
    fn unsupported_on(&self, platform: Platform) -> Option<String> {
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

fn check_id(what: &str, id: &str) -> Result<(), String> {
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolTask {
    /// Stable id, unique per task (`"monitor"`, `"perfmon:IMT"`).
    pub id: String,
    pub label: String,
    /// A hint for the shell's icon (`"process-monitor"`, `"perfmon"`, ...).
    pub kind: String,
    /// Whether [`Tool::stop`] can stop it (the tray then offers Stop).
    pub stoppable: bool,
}

/// Receives a tool's background work. The shell lists it and offers a stop
/// action that calls [`ToolHost::stop`] with the same tool and task id.
pub trait TaskSink: Send + Sync {
    fn started(&self, tool: &str, task: &ToolTask);
    /// `None`: unknown; otherwise 0..=1.
    fn progress(&self, tool: &str, task: &str, fraction: Option<f32>);
    fn finished(&self, tool: &str, task: &str);
}

/// A sink that drops everything (tests, hosts without a task list).
pub struct NoTasks;

impl TaskSink for NoTasks {
    fn started(&self, _: &str, _: &ToolTask) {}
    fn progress(&self, _: &str, _: &str, _: Option<f32>) {}
    fn finished(&self, _: &str, _: &str) {}
}

/// What the host gives a tool for one call. Cheap to clone; a tool may keep
/// a clone to report the end of background work it started.
#[derive(Clone)]
pub struct ToolCtx {
    tool: String,
    data_dir: Option<PathBuf>,
    sink: Arc<dyn TaskSink>,
}

impl ToolCtx {
    /// A context for calls by the host; `data_dir` is the app's data folder
    /// (default location of tool output).
    pub fn new(data_dir: Option<PathBuf>, sink: Arc<dyn TaskSink>) -> ToolCtx {
        ToolCtx {
            tool: String::new(),
            data_dir,
            sink,
        }
    }

    /// The same context for calls of `tool` (the host scopes it before
    /// every call, so task reports name their tool).
    pub fn for_tool(&self, tool: &str) -> ToolCtx {
        ToolCtx {
            tool: tool.into(),
            ..self.clone()
        }
    }

    /// The app's data folder, when known.
    pub fn data_dir(&self) -> Option<&Path> {
        self.data_dir.as_deref()
    }

    /// Reports background work this tool started.
    pub fn task_started(&self, task: ToolTask) {
        self.sink.started(&self.tool, &task);
    }

    pub fn task_progress(&self, task: &str, fraction: Option<f32>) {
        self.sink.progress(&self.tool, task, fraction);
    }

    pub fn task_finished(&self, task: &str) {
        self.sink.finished(&self.tool, task);
    }
}

/// A typed operation provider. Every method must honour its contract
/// without panicking; errors are returned as messages for the user.
pub trait Tool: Send + Sync {
    /// The same id as [`ToolInfo::id`].
    fn id(&self) -> &str;
    /// Asked once, at registration.
    fn describe(&self) -> ToolInfo;
    /// Runs `op` with `args` that already passed the op's schema. Returns
    /// the operation's JSON result.
    fn invoke(&self, op: &str, args: Value, ctx: &ToolCtx) -> Result<Value>;
    /// Stops background work this tool reported through [`ToolCtx::task_started`].
    fn stop(&self, task: &str, ctx: &ToolCtx) -> Result<()> {
        let _ = ctx;
        anyhow::bail!("tool \"{}\" has no background task \"{task}\" to stop", self.id())
    }
}

/// A tool as registered: its description and the guard of its plugin.
pub(crate) struct Registered {
    pub owner: String,
    pub guard: Option<Guard>,
    pub info: ToolInfo,
    pub tool: Arc<dyn Tool>,
}

/// Whether a tool can be called now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Availability {
    Available,
    /// Not on this platform.
    Unsupported {
        reason: String,
    },
    /// Its plugin is inactive, quarantined or not responding.
    PluginUnavailable {
        reason: String,
    },
}

/// A tool as listed for the UI.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolEntry {
    #[serde(flatten)]
    pub info: ToolInfo,
    /// The plugin that contributes it.
    pub plugin: String,
    pub availability: Availability,
}

/// Why a tool call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolError {
    /// No installed (enabled) plugin contributes this tool.
    UnknownTool(String),
    UnknownOp {
        tool: String,
        op: String,
        known: Vec<String>,
    },
    /// The tool does not run on this platform.
    Unsupported(String),
    InvalidArgs {
        tool: String,
        op: String,
        error: SchemaError,
    },
    /// The plugin refused (inactive, quarantined, hung) or the call failed, crashed or overran.
    Failed(String),
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToolError::UnknownTool(t) => write!(f, "no installed plugin provides the tool \"{t}\""),
            ToolError::UnknownOp { tool, op, known } => {
                write!(f, "tool \"{tool}\" has no operation \"{op}\" (it has: {})", known.join(", "))
            }
            ToolError::Unsupported(reason) => write!(f, "{reason}"),
            ToolError::InvalidArgs { tool, op, error } => write!(f, "{tool} {op}: {error}"),
            ToolError::Failed(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for ToolError {}

/// The tools of a registry, for the shell: listing, dispatch and stopping
/// background work. Cheap to clone and usable without the workspace lock.
#[derive(Clone)]
pub struct ToolHost {
    tools: Arc<[Arc<Registered>]>,
    platform: Platform,
}

impl fmt::Debug for ToolHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolHost")
            .field("tools", &self.tools.iter().map(|t| t.info.id.as_str()).collect::<Vec<_>>())
            .field("platform", &self.platform)
            .finish()
    }
}

impl ToolHost {
    pub(crate) fn new(tools: &[Arc<Registered>]) -> ToolHost {
        ToolHost {
            tools: tools.into(),
            platform: Platform::current(),
        }
    }

    /// The same host judging availability for another platform (tests).
    pub fn on_platform(mut self, platform: Platform) -> ToolHost {
        self.platform = platform;
        self
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Every registered tool with its availability, in registration order.
    pub fn list(&self) -> Vec<ToolEntry> {
        self.tools
            .iter()
            .map(|t| ToolEntry {
                info: t.info.clone(),
                plugin: t.owner.clone(),
                availability: self.availability(t),
            })
            .collect()
    }

    fn availability(&self, t: &Registered) -> Availability {
        if let Some(reason) = t.info.unsupported_on(self.platform) {
            return Availability::Unsupported { reason };
        }
        match t.guard.as_ref().map(|g| g.health().availability(g.owner())) {
            Some(Err(reason)) => Availability::PluginUnavailable { reason },
            _ => Availability::Available,
        }
    }

    fn find(&self, tool: &str) -> std::result::Result<&Arc<Registered>, ToolError> {
        self.tools
            .iter()
            .find(|t| t.info.id == tool)
            .ok_or_else(|| ToolError::UnknownTool(tool.chars().take(MAX_ID_LEN).collect()))
    }

    /// Validates `args` against the op's schema and runs the op under the
    /// plugin's guard and the op's budget.
    pub fn invoke(&self, tool: &str, op: &str, args: Value, ctx: &ToolCtx) -> std::result::Result<Value, ToolError> {
        let t = self.find(tool)?;
        let spec = t.info.ops.iter().find(|o| o.name == op).ok_or_else(|| ToolError::UnknownOp {
            tool: t.info.id.clone(),
            op: op.chars().take(MAX_ID_LEN).collect(),
            known: t.info.ops.iter().map(|o| o.name.clone()).collect(),
        })?;
        if let Some(reason) = t.info.unsupported_on(self.platform) {
            return Err(ToolError::Unsupported(reason));
        }
        let invalid = |error| ToolError::InvalidArgs {
            tool: t.info.id.clone(),
            op: spec.name.clone(),
            error,
        };
        let size = serde_json::to_vec(&args).map(|v| v.len()).unwrap_or(usize::MAX);
        if size > MAX_ARGS_BYTES {
            return Err(invalid(SchemaError {
                at: "args".into(),
                message: format!("are too large ({size} bytes, at most {MAX_ARGS_BYTES})"),
            }));
        }
        // No arguments at all is the same as an empty object.
        let args = if args.is_null() { Value::Object(Default::default()) } else { args };
        spec.args.validate(&args).map_err(invalid)?;
        let (inner, ctx, op_name) = (t.tool.clone(), ctx.for_tool(&t.info.id), spec.name.clone());
        let what = format!("tool {} {}", t.info.id, spec.name);
        let run = move || inner.invoke(&op_name, args, &ctx);
        match &t.guard {
            Some(g) => g.call_detached_within(CallKind::Tool, spec.budget, &what, run),
            None => crate::guard::catch(&what, run),
        }
        .map_err(|e| ToolError::Failed(format!("{e:#}")))
    }

    /// Stops background work of `tool` (from the tray or status bar).
    pub fn stop(&self, tool: &str, task: &str, ctx: &ToolCtx) -> std::result::Result<(), ToolError> {
        let t = self.find(tool)?;
        let (inner, ctx, id) = (t.tool.clone(), ctx.for_tool(&t.info.id), task.to_string());
        let what = format!("tool {} stop", t.info.id);
        let run = move || inner.stop(&id, &ctx);
        match &t.guard {
            Some(g) => g.call_detached_within(CallKind::Tool, Some(DEFAULT_OP_BUDGET), &what, run),
            None => crate::guard::catch(&what, run),
        }
        .map_err(|e| ToolError::Failed(format!("{e:#}")))
    }
}

#[cfg(test)]
mod tests;
