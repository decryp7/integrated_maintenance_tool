//! Backend execution layer of the activity log: every IPC command, logged where Tauri dispatches it.
//!
//! [`logged`] wraps the invoke handler. Tauri 2 gives no generic hook on a command's result (the
//! `InvokeResolver`'s responder is private), so this layer logs the dispatch (command, window, operation
//! id, kind) and the UI's entry for the same `OperationId` carries the duration and outcome.
//!
//! The UI sends the operation id as the [`OP_HEADER`] header of the IPC request. It is validated (short,
//! `[A-Za-z0-9-]`) before it is logged or exposed; anything else is ignored. During a synchronous command
//! the id and window are available to [`super::domain_event`] through [`current_op`]; async commands take
//! an [`OpId`] argument instead.

use super::config::config;
use super::SOURCE_CONTEXT;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::ipc::{CommandArg, CommandItem, Invoke, InvokeError};
use tauri::Runtime;

/// IPC header carrying the UI's operation id.
pub const OP_HEADER: &str = "imt-op-id";
const MAX_OP_ID: usize = 64;
const MAX_COMMAND: usize = 96;

/// How a command is logged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    /// Changes state or acts for the user: always at Info.
    Mutation,
    /// Read-only; at Debug while `queriesAtDebug` is on (the default), else Info.
    Query,
    /// Polling and bookkeeping (autosave, progress, log forwarding): at Trace, or not at all.
    Quiet,
}

/// Commands that only read. Every command of `commands!` must be in exactly one of these lists or
/// [`MUTATIONS`] (a test enforces it), so a new command is classified on purpose.
const QUERIES: &[&str] = &[
    "workspace_summary",
    "query_page",
    "query_timeline",
    "query_facet",
    "query_chart",
    "preview_extractor",
    "source_context",
    "overview",
    "annotations",
    "query_runs",
    "preview_spans",
    "plugin_events",
    "tail_status",
    "live_state",
    "run_query",
    "run_query_stream",
    "folder_survey",
    "folder_setup_compose",
    "browser_state",
    "browser_event_query",
    "browser_sources",
    "browser_chat_status",
    "browser_driving",
    "check_query",
    "complete_query",
    "query_patterns",
    "query_exceptions",
    "analysis_coverage",
    "query_anomalies",
    "compare_windows",
    "field_stats",
    "correlate",
    "query_traces",
    "query_requests",
    "query_moment",
    "preview_parse",
    "infer_regex",
    "explain_regex",
    "config_diff_formats",
    "diff_configs",
    "diff_snapshots",
    "suggest_offsets",
    "profiling_sniff",
    "profiling_frame_paths",
    "profiling_list",
    "profiling_spans",
    "profiling_hotspots",
    "profiling_tree",
    "profiling_hot_path",
    "profiling_butterfly",
    "profiling_flame",
    "profiling_diff_flame",
    "profiling_function_diff",
    "profiling_threads",
    "profiling_findings",
    "profiling_moment",
    "profiling_heap_types",
    "profiling_heap_diff",
    "profiling_heap_growth",
    "profiling_heap_dominators",
    "profiling_heap_instances",
    "profiling_heap_paths",
    "get_profile",
    "validate_profile",
    "profile_history",
    "profile_version",
    "profile_templates",
    "demo_paths",
    "list_error_snapshots",
    "read_error_snapshot",
    "diagnostics_settings",
    "storage_usage",
    "elevation",
    "read_file_head",
    "tools_list",
    "agent_schema",
    "agent_config_load",
    "agent_config_validate",
    "agent_runs",
    "agent_run_manifest",
    "app_status",
    "plugin_isolation_status",
    "last_session",
    "settings_sections",
    "updates_status",
    "updates_check",
    "list_windows",
    "list_popouts",
    "background_tasks",
    "jobs_list",
    "jobs_wait",
    "jobs_unwait",
    "notifications_list",
    "ai_status",
    "ai_hardware",
    "ai_recommend",
    "ai_catalog",
    "ai_setup_plan",
    "ai_models",
    "ai_chat_sessions",
    "ai_chat_get",
    "ai_chat_history",
    "ai_suggested_prompts",
    "ai_runtimes",
    "ai_manual_download",
    "preview_redaction",
    "redaction_summary",
    "dashboard_tile",
    "estimate_bundle",
    "problem_report_preview",
    "diagnostics_report",
    "diagnostics_report_image",
];

/// Polling and bookkeeping the UI does on its own.
const QUIET: &[&str] = &[
    "take_pending_open",
    "import_progress",
    "save_session",
    "ai_chat_events_since",
    "set_window_title",
    "set_app_menu",
    "pointer_button_down",
    "jobs_record",
    "notifications_record",
];

/// Commands that change state or act for the user.
const MUTATIONS: &[&str] = &[
    "open_paths",
    "remove_source",
    "clear_workspace",
    "reload_workspace",
    "export_events",
    "set_live",
    "cancel_import",
    "cancel_query",
    "browser_open",
    "browser_nav",
    "browser_close",
    "browser_open_external",
    "browser_capture",
    "browser_discard_sources",
    "browser_set_read_along",
    "browser_set_agent",
    "browser_agent_stop",
    "browser_agent_continue",
    "browser_agent_respond",
    "browser_open_suggested",
    "save_project",
    "open_project",
    "mute_alerts",
    "unmute_alerts",
    "open_paths_live",
    "export_config_diff",
    "set_source_offset",
    "profiling_open",
    "profiling_close",
    "save_profile",
    "import_profile",
    "export_profile",
    "save_error_snapshot",
    "delete_error_snapshot",
    "open_snapshots_folder",
    "storage_clear",
    "storage_reveal",
    "open_source_file",
    "export_diagnostics",
    "set_diagnostics_settings",
    "restart_elevated",
    "tool_invoke",
    "agent_config_save",
    "agent_open_run",
    "restart_app",
    "pick_paths",
    "updates_download",
    "updates_cancel",
    "updates_install_on_quit",
    "updates_restart",
    "updates_open_releases",
    "plugin_reset",
    "plugin_set_enabled",
    "restore_windows",
    "set_settings",
    "reset_settings",
    "export_settings",
    "import_settings",
    "new_window",
    "close_window",
    "focus_window",
    "resolve_close",
    "resolve_open",
    "popout_panel",
    "dock_panel",
    "set_popout_on_top",
    "set_recent_projects",
    "stop_background_task",
    "jobs_cancel",
    "jobs_retry",
    "jobs_move",
    "jobs_clear_finished",
    "jobs_mark_seen",
    "jobs_result",
    "jobs_reveal",
    "jobs_restore",
    "notifications_mark_read",
    "notifications_mark_all_read",
    "notifications_dismiss",
    "notifications_restore",
    "ai_research",
    "browser_research_stop",
    "browser_open_and_read",
    "ai_setup_run",
    "ai_setup_cancel",
    "ai_model_activate",
    "ai_model_delete",
    "ai_model_add_local",
    "ai_reset",
    "ai_start",
    "ai_cancel",
    "ai_chat_new",
    "ai_chat_delete",
    "ai_chat_send",
    "ai_chat_cancel",
    "ai_chat_restore",
    "ai_chat_export_markdown",
    "ai_open_link",
    "ai_runtime_install",
    "ai_runtime_add_archives",
    "ai_manual_download_open",
    "ai_runtime_cancel",
    "ai_runtime_delete",
    "ai_runtime_choose",
    "set_redacted_view",
    "export_report",
    "save_visual",
    "create_bundle",
    "open_bundle",
    "save_problem_report",
    "open_activity_log",
];

/// How `command` is logged. Plugin commands (`plugin:<name>|<cmd>`): the log plugin's own forwarding and
/// event subscriptions are quiet, dialogs are user actions, the rest are queries. Unknown commands are
/// treated as mutations, so nothing new goes unlogged.
pub fn command_kind(command: &str) -> CommandKind {
    if let Some(rest) = command.strip_prefix("plugin:") {
        return match rest.split('|').next().unwrap_or_default() {
            "log" | "event" => CommandKind::Quiet,
            "dialog" => CommandKind::Mutation,
            _ => CommandKind::Query,
        };
    }
    if QUERIES.contains(&command) {
        CommandKind::Query
    } else if QUIET.contains(&command) {
        CommandKind::Quiet
    } else {
        CommandKind::Mutation
    }
}

/// True for a command the tables classify explicitly (see the coverage test).
#[cfg(test)]
pub(crate) fn is_classified(command: &str) -> bool {
    QUERIES.contains(&command) || QUIET.contains(&command) || MUTATIONS.contains(&command)
}

/// The operation id when it is well formed (it comes from the webview: never trusted as is).
pub fn valid_op_id(raw: &str) -> Option<String> {
    let ok = !raw.is_empty() && raw.len() <= MAX_OP_ID && raw.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    ok.then(|| raw.to_string())
}

fn safe_command(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | ':' | '|' | '-' | '.'))
        .take(MAX_COMMAND)
        .collect()
}

/// What the rate limiter decides for one entry.
#[derive(Debug, PartialEq, Eq)]
pub enum Admit {
    Log,
    /// Log it, after a summary of the `n` entries suppressed in the previous window.
    LogAfterSuppressed(u64),
    Suppress,
}

/// Bounds how often the same key (command and window) is logged: at most `burst` per `window`; the rest
/// are counted and summarised once the window has passed.
#[derive(Debug)]
pub struct RateLimiter {
    burst: u32,
    window: Duration,
    keys: HashMap<String, (Instant, u32, u64)>,
}

/// Most keys tracked (the oldest are dropped).
const MAX_KEYS: usize = 512;

impl RateLimiter {
    pub fn new(burst: u32, window: Duration) -> Self {
        RateLimiter {
            burst,
            window,
            keys: HashMap::new(),
        }
    }

    pub fn admit(&mut self, key: &str, now: Instant) -> Admit {
        if self.keys.len() >= MAX_KEYS && !self.keys.contains_key(key) {
            let window = self.window;
            self.keys.retain(|_, (start, _, _)| now.duration_since(*start) < window);
            if self.keys.len() >= MAX_KEYS {
                self.keys.clear();
            }
        }
        let entry = self.keys.entry(key.to_string()).or_insert((now, 0, 0));
        if now.duration_since(entry.0) >= self.window {
            let suppressed = entry.2;
            *entry = (now, 1, 0);
            return if suppressed > 0 {
                Admit::LogAfterSuppressed(suppressed)
            } else {
                Admit::Log
            };
        }
        if entry.1 < self.burst {
            entry.1 += 1;
            Admit::Log
        } else {
            entry.2 += 1;
            Admit::Suppress
        }
    }
}

thread_local! {
    static CURRENT: RefCell<Option<(Option<String>, String)>> = const { RefCell::new(None) };
}

/// The operation id of the synchronous command running on this thread, if the UI sent one.
pub fn current_op() -> Option<String> {
    CURRENT.with(|c| c.borrow().as_ref().and_then(|(op, _)| op.clone()))
}

/// The window of the synchronous command running on this thread.
pub(crate) fn current_window() -> Option<String> {
    CURRENT.with(|c| c.borrow().as_ref().map(|(_, w)| w.clone()))
}

struct CurrentGuard;
impl Drop for CurrentGuard {
    fn drop(&mut self) {
        CURRENT.with(|c| *c.borrow_mut() = None);
    }
}

/// One IPC request as logged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IpcCall {
    pub command: String,
    pub window: String,
    pub op: Option<String>,
}

impl IpcCall {
    fn of<R: Runtime>(invoke: &Invoke<R>) -> IpcCall {
        let op = invoke
            .message
            .headers()
            .get(OP_HEADER)
            .and_then(|v| v.to_str().ok())
            .and_then(valid_op_id);
        IpcCall {
            command: safe_command(invoke.message.command()),
            window: safe_command(invoke.message.webview_ref().label()),
            op,
        }
    }
}

fn level_of(kind: CommandKind) -> Option<log::Level> {
    match kind {
        CommandKind::Mutation => Some(log::Level::Info),
        CommandKind::Query if config().queries_at_debug => Some(log::Level::Debug),
        CommandKind::Query => Some(log::Level::Info),
        CommandKind::Quiet => Some(log::Level::Trace),
    }
}

/// Logs one dispatch (rate-limited per command and window).
pub(crate) fn log_call(call: &IpcCall, limiter: &Mutex<RateLimiter>, now: Instant) {
    let kind = command_kind(&call.command);
    // The log plugin's own forwarding is never logged (it would log every log line twice).
    if call.command.starts_with("plugin:log|") {
        return;
    }
    let Some(level) = level_of(kind) else { return };
    if level > log::max_level() {
        return;
    }
    let key = format!("{}@{}", call.command, call.window);
    let admit = limiter.lock().unwrap_or_else(|e| e.into_inner()).admit(&key, now);
    let kind_name = match kind {
        CommandKind::Mutation => "mutation",
        CommandKind::Query => "query",
        CommandKind::Quiet => "quiet",
    };
    let op = call.op.as_deref().unwrap_or("");
    let attributed = if call.op.is_some() { "attributed" } else { "unattributed" };
    // A command missing from the tables is logged as a mutation, flagged so it gets classified.
    let classified = kind != CommandKind::Mutation || MUTATIONS.contains(&call.command.as_str()) || call.command.starts_with("plugin:");
    match admit {
        Admit::Suppress => return,
        Admit::LogAfterSuppressed(n) => log::log!(target: SOURCE_CONTEXT, level,
            SourceContext = SOURCE_CONTEXT, Command = call.command.as_str(), Window = call.window.as_str(), Suppressed = n;
            "IPC {} from {} repeated {} more times", call.command, call.window, n),
        Admit::Log => {}
    }
    log::log!(target: SOURCE_CONTEXT, level,
        SourceContext = SOURCE_CONTEXT, Command = call.command.as_str(), Window = call.window.as_str(),
        OperationId = op, Kind = kind_name, Attribution = attributed, Classified = classified,
        MessageTemplate = "IPC {Command} from {Window}";
        "IPC {} from {}", call.command, call.window);
}

/// Wraps the invoke handler so every IPC command is logged (see the module docs). The wrapped handler is
/// called exactly once per request and its result returned unchanged; an unknown command is logged as a
/// warning.
pub fn logged<R: Runtime>(inner: impl Fn(Invoke<R>) -> bool + Send + Sync + 'static) -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
    let limiter = Mutex::new(RateLimiter::new(30, Duration::from_secs(1)));
    move |invoke| {
        let call = IpcCall::of(&invoke);
        // Structured context for everything logged while the command is dispatched (the CLEF layer turns the
        // span's fields into properties). Async commands continue on other threads, outside this span.
        let span = tracing::info_span!(
            "ipc",
            Command = call.command.as_str(),
            Window = call.window.as_str(),
            OperationId = call.op.as_deref().unwrap_or("")
        );
        let _entered = span.enter();
        log_call(&call, &limiter, Instant::now());
        CURRENT.with(|c| *c.borrow_mut() = Some((call.op.clone(), call.window.clone())));
        let _reset = CurrentGuard;
        let handled = inner(invoke);
        if !handled {
            log::warn!(target: SOURCE_CONTEXT, SourceContext = SOURCE_CONTEXT, Command = call.command.as_str(), Window = call.window.as_str(); "Unknown IPC command {}", call.command);
        }
        handled
    }
}

/// The calling operation's id, as a command argument (`op: OpId`): for async commands, where
/// [`current_op`] is not available. Never fails: a missing or malformed header is `OpId(None)`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpId(pub Option<String>);

impl OpId {
    pub fn as_deref(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

impl<'de, R: Runtime> CommandArg<'de, R> for OpId {
    fn from_command(command: CommandItem<'de, R>) -> Result<Self, InvokeError> {
        Ok(OpId(
            command
                .message
                .headers()
                .get(OP_HEADER)
                .and_then(|v| v.to_str().ok())
                .and_then(valid_op_id),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_registered_command_is_classified() {
        // The coverage guard: parse the `commands!` list of lib.rs; a new command must be classified here.
        // Checkouts may use CRLF (Windows, `text=auto`).
        let lib = include_str!("../lib.rs").replace("\r\n", "\n");
        let start = lib.find("macro_rules! commands").expect("commands! macro");
        let end = start + lib[start..].find("\n}\n").expect("end of commands!");
        let names: Vec<&str> = lib[start..end]
            .lines()
            .map(str::trim)
            .filter(|l| !l.starts_with("//") && l.ends_with(',') && l.contains("::"))
            .filter_map(|l| l.trim_end_matches(',').rsplit("::").next())
            .collect();
        assert!(names.len() > 150, "found only {} commands", names.len());
        let missing: Vec<&&str> = names.iter().filter(|n| !is_classified(n)).collect();
        assert!(missing.is_empty(), "classify these commands in activity/ipc.rs: {missing:?}");
        for list in [QUERIES, QUIET, MUTATIONS] {
            for n in list {
                let lists = [QUERIES, QUIET, MUTATIONS].iter().filter(|l| l.contains(n)).count();
                assert_eq!(lists, 1, "{n} is in {lists} lists");
            }
        }
    }

    #[test]
    fn kinds() {
        assert_eq!(command_kind("query_page"), CommandKind::Query);
        assert_eq!(command_kind("save_profile"), CommandKind::Mutation);
        assert_eq!(command_kind("save_session"), CommandKind::Quiet);
        assert_eq!(command_kind("brand_new_command"), CommandKind::Mutation);
        assert_eq!(command_kind("plugin:log|log"), CommandKind::Quiet);
        assert_eq!(command_kind("plugin:event|listen"), CommandKind::Quiet);
        assert_eq!(command_kind("plugin:dialog|open"), CommandKind::Mutation);
        assert_eq!(command_kind("plugin:window|title"), CommandKind::Query);
    }

    #[test]
    fn op_ids_are_validated() {
        assert_eq!(valid_op_id("3f2a-9c"), Some("3f2a-9c".into()));
        assert_eq!(valid_op_id(""), None);
        assert_eq!(valid_op_id("a b"), None);
        assert_eq!(valid_op_id("x\n{\"@l\":\"Fatal\"}"), None);
        assert_eq!(valid_op_id(&"a".repeat(65)), None);
        assert_eq!(safe_command("query_page\n<script>"), "query_pagescript");
        assert_eq!(OpId(Some("a".into())).as_deref(), Some("a"));
    }

    mod wrapper {
        use super::super::*;
        use crate::activity::{captured, domain_event, test_capture, Captured};
        use std::sync::{Arc, Mutex as StdMutex};
        use tauri::ipc::{CallbackFn, InvokeBody};
        use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY};
        use tauri::webview::InvokeRequest;

        fn request(cmd: &str, op: Option<&str>) -> InvokeRequest {
            let mut headers = tauri::http::HeaderMap::new();
            if let Some(op) = op {
                headers.insert(OP_HEADER, op.parse().unwrap());
            }
            InvokeRequest {
                cmd: cmd.into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: crate::test_support::ipc_url(),
                body: InvokeBody::Json(serde_json::json!({})),
                headers,
                invoke_key: INVOKE_KEY.to_string(),
            }
        }

        /// An app whose handler records the operation id it sees, resolves known commands and rejects `fail`.
        fn app(seen: Arc<StdMutex<Vec<Option<String>>>>) -> (tauri::App<MockRuntime>, tauri::WebviewWindow<MockRuntime>) {
            let app = mock_builder()
                .invoke_handler(logged(move |invoke: Invoke<MockRuntime>| {
                    seen.lock().unwrap().push(current_op());
                    match invoke.message.command() {
                        "unknown_thing" => false,
                        "fail" => {
                            invoke.resolver.reject("boom");
                            true
                        }
                        "save_profile" => {
                            domain_event("profile.saved", None, &[("Profile", "default"), ("ApiKey", "sk-1")]);
                            invoke.resolver.resolve(());
                            true
                        }
                        _ => {
                            invoke.resolver.resolve(());
                            true
                        }
                    }
                }))
                .build(mock_context(noop_assets()))
                .unwrap();
            let w = tauri::WebviewWindowBuilder::new(&app, "w-ipc", tauri::WebviewUrl::default())
                .build()
                .unwrap();
            (app, w)
        }

        fn ipc_entries(command: &str) -> Vec<Captured> {
            captured()
                .into_iter()
                .filter(|c| c.props.get("Command").map(String::as_str) == Some(command))
                .collect()
        }

        #[test]
        fn logs_every_command_with_window_op_and_kind() {
            let _g = test_capture();
            let seen = Arc::new(StdMutex::new(Vec::new()));
            let (_app, w) = app(seen.clone());
            assert!(get_ipc_response(&w, request("clear_workspace", Some("op-123"))).is_ok());
            assert!(get_ipc_response(&w, request("query_page", None)).is_ok());
            assert!(get_ipc_response(&w, request("fail", Some("op-9"))).is_err());
            let _ = get_ipc_response(&w, request("plugin:log|log", Some("op-1")));

            let m = &ipc_entries("clear_workspace")[0];
            assert_eq!(m.level, log::Level::Info);
            assert_eq!(m.message, "IPC clear_workspace from w-ipc");
            assert_eq!(m.props["OperationId"], "op-123");
            assert_eq!(m.props["Window"], "w-ipc");
            assert_eq!(m.props["Kind"], "mutation");
            assert_eq!(m.props["Attribution"], "attributed");
            assert_eq!(m.props["SourceContext"], "IMT.Activity");
            let q = &ipc_entries("query_page")[0];
            assert_eq!(
                (q.level, q.props["Kind"].as_str(), q.props["Attribution"].as_str()),
                (log::Level::Debug, "query", "unattributed")
            );
            // Unknown commands default to mutations, so nothing new goes unlogged.
            assert_eq!(ipc_entries("fail")[0].props["OperationId"], "op-9");
            assert_eq!(ipc_entries("fail")[0].props["Classified"], "false");
            assert_eq!(m.props["Classified"], "true");
            assert!(ipc_entries("plugin:log|log").is_empty(), "log forwarding is never logged");
            // The handler saw the operation id while it ran, and it is cleared afterwards.
            assert_eq!(seen.lock().unwrap()[0].as_deref(), Some("op-123"));
            assert_eq!(seen.lock().unwrap()[1], None);
            assert_eq!(current_op(), None);
        }

        #[test]
        fn rejects_malformed_op_ids_and_warns_on_unknown_commands() {
            let _g = test_capture();
            let (_app, w) = app(Arc::new(StdMutex::new(Vec::new())));
            let _ = get_ipc_response(&w, request("clear_workspace", Some("bad id {\"@l\":\"Fatal\"}")));
            let _ = get_ipc_response(&w, request("unknown_thing", Some("op-u")));
            assert_eq!(ipc_entries("clear_workspace")[0].props["OperationId"], "");
            let warn = captured()
                .into_iter()
                .find(|c| c.message == "Unknown IPC command unknown_thing")
                .expect("warning");
            assert_eq!(warn.level, log::Level::Warn);
        }

        #[test]
        fn domain_events_carry_the_commands_operation_id() {
            let _g = test_capture();
            let (_app, w) = app(Arc::new(StdMutex::new(Vec::new())));
            assert!(get_ipc_response(&w, request("save_profile", Some("op-save"))).is_ok());
            let e = captured().into_iter().find(|c| c.message == "profile.saved").expect("domain event");
            assert_eq!(e.props["OperationId"], "op-save");
            assert_eq!(e.props["Window"], "w-ipc");
            assert_eq!(e.props["Operation"], "profile.saved");
            assert_eq!(e.props["Kind"], "domain");
            assert_eq!(e.props["ApiKey"], "***");
            // It is a breadcrumb too.
            assert!(crate::activity::breadcrumbs(Some("w-ipc"), 5)
                .iter()
                .any(|b| b.operation == "profile.saved"));
            // Outside a command, an explicit id is used.
            domain_event("export.written", Some("op-x"), &[]);
            assert_eq!(
                captured().into_iter().find(|c| c.message == "export.written").unwrap().props["OperationId"],
                "op-x"
            );
        }

        #[test]
        fn query_text_and_paths_never_reach_the_log_in_normal_mode() {
            let _g = test_capture();
            crate::activity::set_config(crate::activity::ActivitySettings::default());
            let app = crate::test_support::test_app_with_commands(None);
            let w = crate::test_support::window(&app, crate::windows::MAIN);
            let mut req = request("run_query", Some("op-q"));
            req.body = InvokeBody::Json(
                serde_json::json!({ "request": { "query": { "text": "alice-secret-term", "ql": "user = \"alice-secret-term\"" } } }),
            );
            let _ = get_ipc_response(&w, req);
            let mut req = request("open_paths", Some("op-p"));
            req.body = InvokeBody::Json(serde_json::json!({ "paths": ["/Users/alice-secret-term/logs/app.log"] }));
            let _ = get_ipc_response(&w, req);
            let all = captured();
            assert!(all.iter().any(|c| c.props.get("Command").map(String::as_str) == Some("run_query")));
            let leaked: Vec<&Captured> = all
                .iter()
                .filter(|c| c.message.contains("alice-secret-term") || c.props.values().any(|v| v.contains("alice-secret-term")))
                .collect();
            assert!(leaked.is_empty(), "leaked: {leaked:?}");
        }

        #[test]
        fn quiet_and_query_levels_follow_the_settings() {
            let _g = test_capture();
            let limiter = Mutex::new(RateLimiter::new(30, Duration::from_secs(1)));
            let call = |command: &str| IpcCall {
                command: command.into(),
                window: "main".into(),
                op: None,
            };
            crate::activity::set_config(crate::activity::ActivitySettings {
                detail: crate::activity::Detail::Normal,
                queries_at_debug: false,
            });
            log_call(&call("query_facet"), &limiter, Instant::now());
            crate::activity::set_config(crate::activity::ActivitySettings::default());
            log_call(&call("save_session"), &limiter, Instant::now());
            assert_eq!(ipc_entries("query_facet")[0].level, log::Level::Info);
            assert_eq!(ipc_entries("save_session")[0].level, log::Level::Trace);
            // Below the configured level nothing is logged.
            log::set_max_level(log::LevelFilter::Info);
            log_call(&call("query_chart"), &limiter, Instant::now());
            log::set_max_level(log::LevelFilter::Trace);
            assert!(ipc_entries("query_chart").is_empty());
        }

        #[test]
        fn floods_are_rate_limited_and_summarised() {
            let _g = test_capture();
            let limiter = Mutex::new(RateLimiter::new(3, Duration::from_secs(1)));
            let call = IpcCall {
                command: "open_paths".into(),
                window: "main".into(),
                op: Some("op".into()),
            };
            let t0 = Instant::now();
            for _ in 0..10 {
                log_call(&call, &limiter, t0);
            }
            assert_eq!(ipc_entries("open_paths").len(), 3);
            log_call(&call, &limiter, t0 + Duration::from_secs(2));
            let all = ipc_entries("open_paths");
            assert_eq!(all.len(), 5);
            assert_eq!(all[3].props["Suppressed"], "7");
            assert!(all[3].message.contains("repeated 7 more times"));
        }
    }

    #[test]
    fn rate_limiter_bursts_then_summarises() {
        let t0 = Instant::now();
        let mut r = RateLimiter::new(3, Duration::from_secs(1));
        assert_eq!((0..3).map(|_| r.admit("k", t0)).filter(|a| *a == Admit::Log).count(), 3);
        assert_eq!(r.admit("k", t0), Admit::Suppress);
        assert_eq!(r.admit("k", t0 + Duration::from_millis(500)), Admit::Suppress);
        assert_eq!(r.admit("other", t0), Admit::Log);
        assert_eq!(r.admit("k", t0 + Duration::from_secs(1)), Admit::LogAfterSuppressed(2));
        assert_eq!(r.admit("k", t0 + Duration::from_secs(3)), Admit::Log);
        // Bounded number of keys.
        for i in 0..(MAX_KEYS * 2) {
            r.admit(&format!("k{i}"), t0 + Duration::from_secs(3));
        }
        assert!(r.keys.len() <= MAX_KEYS);
    }
}
