//! The command table (ADR 0016): every command the UI can invoke, declared once in [`crate::command_table!`]
//! with its kind (query, mutation, quiet), the windows that may call it and its largest request. The table
//! generates the invoke handler, gives the activity log its classification ([`kind_of`]) and drives the
//! runtime checks of [`guard`]: a pop-out panel gets a smaller set than its host window, the "AI is browsing"
//! banner only its own commands, and an oversized request is refused before its arguments are parsed.
//!
//! Every command is `async` (a test fails on a sync one): a sync command runs on the main thread, where
//! window creation deadlocks and file or lock waits freeze every window.

use crate::{CmdError, ErrorCode};
use tauri::ipc::{Invoke, InvokeBody};
use tauri::Runtime;

/// How a command is logged and what it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Read-only: at Debug while `queriesAtDebug` is on (the default), else Info.
    Query,
    /// Changes state or acts for the user: always at Info.
    Mutation,
    /// Polling and bookkeeping (autosave, progress): at Trace, or not at all.
    Quiet,
}

/// The windows a command may be called from (a set of bits).
pub type Windows = u8;
/// `main` and the other IMT windows (`w-2`, ...).
pub const MAIN: Windows = 1;
/// Panels popped out of a window (`p-<host>-<n>`).
pub const POPOUT: Windows = 2;
/// The "AI is browsing" banner over a browser window (`aib-<owner>`).
pub const BANNER: Windows = 4;
/// A window and its pop-outs.
pub const WINDOW: Windows = MAIN | POPOUT;

/// The largest request a command accepts unless its entry says otherwise.
pub const DEFAULT_LIMIT: usize = 256 * 1024;

/// One command of the table.
#[derive(Debug, Clone, Copy)]
pub struct Spec {
    /// The handler's path as written in the table (`workspace_commands :: open_paths`).
    pub path: &'static str,
    pub kind: Kind,
    pub windows: Windows,
    /// The largest request body in bytes.
    pub limit: usize,
}

impl Spec {
    /// The command's name on the wire (the last segment of its path).
    pub fn name(&self) -> &'static str {
        self.path.rsplit("::").next().unwrap_or(self.path).trim()
    }
}

/// `limit` as written in a table entry (none: [`DEFAULT_LIMIT`]).
pub const fn limit(given: &[usize]) -> usize {
    match given {
        [l] => *l,
        _ => DEFAULT_LIMIT,
    }
}

/// The command `name`'s entry.
pub fn spec(name: &str) -> Option<&'static Spec> {
    crate::COMMANDS.iter().find(|s| s.name() == name)
}

/// How `command` is logged. Plugin commands (`plugin:<name>|<cmd>`): the log plugin's forwarding and event
/// subscriptions are quiet, dialogs are user actions, the rest are queries. Unknown commands are mutations,
/// so nothing new goes unlogged.
pub fn kind_of(command: &str) -> Kind {
    if let Some(rest) = command.strip_prefix("plugin:") {
        return match rest.split('|').next().unwrap_or_default() {
            "log" | "event" => Kind::Quiet,
            "dialog" => Kind::Mutation,
            _ => Kind::Query,
        };
    }
    spec(command).map_or(Kind::Mutation, |s| s.kind)
}

/// The window class of a webview label.
pub fn class_of(label: &str) -> Option<Windows> {
    if label == crate::windows::MAIN || label.starts_with(crate::windows::LABEL_PREFIX) {
        Some(MAIN)
    } else if crate::panel_windows::is_popout(label) {
        Some(POPOUT)
    } else if crate::browser::is_banner_label(label) {
        Some(BANNER)
    } else {
        None
    }
}

/// Whether window `label` may call `command` with a request of `size` bytes; why not.
pub fn check(command: &str, label: &str, size: usize) -> Result<(), CmdError> {
    if command.starts_with("plugin:") {
        // Plugin commands are governed by the capabilities (`capabilities/*.json`).
        return Ok(());
    }
    let Some(spec) = spec(command) else {
        return Ok(()); // Unknown: Tauri answers that the command does not exist.
    };
    if !class_of(label).is_some_and(|c| spec.windows & c != 0) {
        return Err(CmdError::new(
            ErrorCode::Denied,
            format!("{command} is not available in this window"),
        ));
    }
    if size > spec.limit {
        return Err(CmdError::new(
            ErrorCode::Limit,
            format!("the request to {command} is too large ({size} bytes; at most {})", spec.limit),
        ));
    }
    Ok(())
}

/// The size of a request body, counting up to `cap` (a JSON body is measured as serialised).
fn body_size(body: &InvokeBody, cap: usize) -> usize {
    match body {
        InvokeBody::Raw(bytes) => bytes.len(),
        InvokeBody::Json(v) => json_size(v, cap),
    }
}

/// About the serialised size of `v`, stopping once past `cap` (so a huge request costs little to refuse).
fn json_size(v: &serde_json::Value, cap: usize) -> usize {
    use serde_json::Value;
    match v {
        Value::Null | Value::Bool(_) => 5,
        Value::Number(_) => 20,
        Value::String(s) => s.len() + 2,
        Value::Array(items) => {
            let mut n = 2;
            for i in items {
                n += json_size(i, cap) + 1;
                if n > cap {
                    break;
                }
            }
            n
        }
        Value::Object(map) => {
            let mut n = 2;
            for (k, i) in map {
                n += k.len() + 4 + json_size(i, cap);
                if n > cap {
                    break;
                }
            }
            n
        }
    }
}

/// Wraps the invoke handler with the table's checks (window class, request size); a refused call is
/// answered with the [`CmdError`] and logged.
pub fn guard<R: Runtime>(inner: impl Fn(Invoke<R>) -> bool + Send + Sync + 'static) -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
    move |invoke| {
        let command = invoke.message.command().to_string();
        let label = invoke.message.webview().label().to_string();
        let cap = spec(&command).map_or(DEFAULT_LIMIT, |s| s.limit);
        let size = body_size(invoke.message.payload(), cap.saturating_add(1));
        if let Err(e) = check(&command, &label, size) {
            log::warn!(Command = command.as_str(), Window = label.as_str(), ErrorCode = format!("{:?}", e.code).as_str(); "A command was refused: {}", e.message);
            invoke.resolver.reject(e);
            return true;
        }
        inner(invoke)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_has_one_entry_with_a_window_and_a_sane_limit() {
        let mut names: Vec<&str> = crate::COMMANDS.iter().map(Spec::name).collect();
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), total, "a command is listed twice");
        for s in crate::COMMANDS {
            assert!(s.windows != 0, "{} is callable from no window", s.name());
            assert!(s.limit >= 1024 && s.limit <= 64 * 1024 * 1024, "{}: {}", s.name(), s.limit);
            assert!(!s.name().contains(' '), "{}", s.path);
        }
        assert_eq!(spec("open_paths").map(|s| s.kind), Some(Kind::Mutation));
        assert_eq!(kind_of("query_page"), Kind::Query);
        assert_eq!(kind_of("save_session"), Kind::Quiet);
        assert_eq!(kind_of("no_such_command"), Kind::Mutation);
        assert_eq!(kind_of("plugin:log|log"), Kind::Quiet);
        assert_eq!(kind_of("plugin:dialog|message"), Kind::Mutation);
        assert_eq!(kind_of("plugin:window|title"), Kind::Query);
    }

    #[test]
    fn windows_get_their_own_set_and_requests_are_bounded() {
        assert_eq!(class_of("main"), Some(MAIN));
        assert_eq!(class_of("w-3"), Some(MAIN));
        assert_eq!(class_of("p-main-1"), Some(POPOUT));
        assert_eq!(class_of("aib-main"), Some(BANNER));
        assert_eq!(class_of("bw-main"), None);
        assert!(check("query_page", "p-main-1", 10).is_ok());
        // A pop-out panel cannot restart IMT or answer its host's prompts.
        let e = check("restart_app", "p-main-1", 10).unwrap_err();
        assert_eq!(e.code, ErrorCode::Denied);
        assert!(check("restart_app", "main", 10).is_ok());
        // The banner only stops what drives its browser.
        assert!(check("browser_driving", "aib-main", 10).is_ok());
        assert!(check("jobs_cancel", "aib-main", 10).is_ok());
        assert_eq!(check("open_paths", "aib-main", 10).unwrap_err().code, ErrorCode::Denied);
        assert!(check("query_page", "unknown-window", 10).is_err());
        assert!(check("plugin:log|log", "aib-main", 10).is_ok());
        assert!(check("no_such_command", "main", 10).is_ok());
        let e = check("query_page", "main", DEFAULT_LIMIT + 1).unwrap_err();
        assert_eq!(e.code, ErrorCode::Limit);
        assert!(
            check("save_session", "main", DEFAULT_LIMIT + 1).is_ok(),
            "the session carries the UI state"
        );
        let big = serde_json::json!({ "ui": "x".repeat(1000), "n": [1, 2, true, null] });
        let size = json_size(&big, usize::MAX);
        assert!((1000..1100).contains(&size), "{size}");
        let many = serde_json::Value::Array(vec![serde_json::Value::from("a".repeat(100)); 100]);
        assert!(json_size(&many, 500) < 1000, "stops past the cap");
        assert_eq!(body_size(&InvokeBody::Raw(vec![0; 7]), 10), 7);
    }

    /// The CSP refuses plugins, base and form targets and framing; no capability grants all of `core`.
    #[test]
    fn the_csp_and_capabilities_are_least_privilege() {
        let conf: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let csp = conf["app"]["security"]["csp"].as_str().unwrap();
        for d in [
            "object-src 'none'",
            "base-uri 'self'",
            "form-action 'none'",
            "frame-ancestors 'none'",
        ] {
            assert!(csp.contains(d), "{d} missing from {csp}");
        }
        assert!(!csp.contains("unsafe-eval"));
        for cap in [
            include_str!("../capabilities/default.json"),
            include_str!("../capabilities/popout.json"),
            include_str!("../capabilities/ai-browsing.json"),
        ] {
            let v: serde_json::Value = serde_json::from_str(cap).unwrap();
            for p in v["permissions"].as_array().unwrap() {
                let p = p.as_str().unwrap_or_default();
                assert!(p != "core:default" && !p.ends_with(":*"), "{p} is too broad");
            }
        }
    }

    /// Every command is async: a sync one runs on the main thread (window creation deadlocks there, and a
    /// wait for a lock or a file freezes every window).
    #[test]
    fn every_command_is_async() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut sync = Vec::new();
        let mut stack = vec![src];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                let lines: Vec<&str> = text.lines().collect();
                for (i, l) in lines.iter().enumerate() {
                    if l.trim() != "#[tauri::command]" {
                        continue;
                    }
                    let sig = lines[i + 1..].iter().take(5).find(|s| s.contains(" fn "));
                    if let Some(sig) = sig.filter(|s| !s.contains("async fn")) {
                        sync.push(format!("{}: {}", path.display(), sig.trim()));
                    }
                }
            }
        }
        assert!(sync.is_empty(), "sync commands (make them async):\n{}", sync.join("\n"));
    }
}
