//! Human-readable steps to reproduce, from breadcrumbs. Pure: the same breadcrumbs always give the same
//! steps (golden tests below). Uses only what the breadcrumbs carry, so it is as private as they are.

use super::breadcrumbs::Breadcrumb;

/// Most steps listed; older ones are summarised as "…".
const MAX_STEPS: usize = 30;

fn trigger_phrase(trigger: &str) -> Option<&'static str> {
    Some(match trigger {
        "menu" => "from the menu",
        "native-menu" => "from the app menu",
        "palette" => "from the command palette",
        "shortcut" => "with a keyboard shortcut",
        "context-menu" => "from a context menu",
        "button" => "with a button",
        "drag-drop" => "by drag and drop",
        "dialog" => "in a dialog",
        _ => return None,
    })
}

fn param<'a>(b: &'a Breadcrumb, key: &str) -> Option<&'a str> {
    b.params.get(key).map(String::as_str).filter(|v| !v.is_empty())
}

fn quoted_or_hash(b: &Breadcrumb, text: &str, hash: &str) -> String {
    match (param(b, text), param(b, hash)) {
        (Some(t), _) => format!("\"{t}\""),
        (None, Some(h)) => format!("(hash {h})"),
        _ => String::new(),
    }
}

/// One breadcrumb as a sentence, without numbering or outcome.
fn describe(b: &Breadcrumb) -> String {
    let op = b.operation.as_str();
    let panel = param(b, "Panel").or(b.view.as_deref());
    let the_panel = panel.map(|v| format!("the {v} panel")).unwrap_or_else(|| "a panel".into());
    let view = panel.unwrap_or("a panel");
    let s = match op {
        "file.open" => {
            let what = param(b, "Path").or(param(b, "File")).unwrap_or("files");
            match param(b, "Files") {
                Some(n) => format!("Opened {what} ({n} files)"),
                None => format!("Opened {what}"),
            }
        }
        "file.save" => format!("Saved {}", param(b, "Path").or(param(b, "File")).unwrap_or("a file")),
        "file.export" => format!(
            "Exported {}{}",
            param(b, "Format").map(|f| format!("{f} ")).unwrap_or_default(),
            param(b, "Path").map(|p| format!("to {p}")).unwrap_or_else(|| "events".into())
        ),
        "query.change" => format!("Applied query {}", quoted_or_hash(b, "Query", "QueryHash"))
            .trim_end()
            .to_string(),
        "search.change" => format!("Searched for {}", quoted_or_hash(b, "Search", "SearchHash"))
            .trim_end()
            .to_string(),
        "filter.change" => match param(b, "Filters") {
            Some(n) => format!("Changed the filters ({n} active)"),
            None => "Changed the filters".into(),
        },
        "timeRange.change" => "Changed the time range".into(),
        "selection.change" => match param(b, "RecordId") {
            Some(id) => format!("Selected event {id}"),
            None => "Changed the selection".into(),
        },
        "dock.open" => format!("Opened {the_panel}"),
        "dock.close" => format!("Closed {the_panel}"),
        "dock.popout" => format!("Popped out {view}"),
        "dock.dockBack" => format!("Docked {view} back"),
        "dock.autoHide" => format!("Auto-hid {view}"),
        "dock.layout" => "Applied a panel layout".into(),
        "settings.change" => match (param(b, "Section"), param(b, "Fields")) {
            (Some(s), Some(k)) => format!("Changed settings {s}: {k}"),
            (Some(s), None) => format!("Changed {s} settings"),
            _ => "Changed settings".into(),
        },
        "live.on" => "Turned live tail on".into(),
        "live.off" => "Turned live tail off".into(),
        "ai.ask" => "Asked the AI assistant".into(),
        "ai.applyProposal" => "Applied an AI proposal".into(),
        "browser.agent" => format!(
            "AI agent {} {} ({})",
            param(b, "Action").unwrap_or("acted"),
            param(b, "Site").filter(|s| !s.is_empty()).unwrap_or("in the browser"),
            param(b, "Outcome").unwrap_or("done")
        ),
        "browser.agent.stop" => "Stopped the AI agent".into(),
        "browser.capture" => "Added a web page to the AI chat".into(),
        "edit.undo" => "Undid the last change".into(),
        "edit.redo" => "Redid the last change".into(),
        _ if op.starts_with("ipc.") => format!("Ran {}", param(b, "Command").unwrap_or(&op[4..])),
        _ => match param(b, "Title") {
            Some(t) => format!("Clicked '{t}'"),
            None => format!("Ran {op}"),
        },
    };
    match trigger_phrase(&b.trigger) {
        Some(p) if !op.starts_with("dock.") || b.trigger != "button" => format!("{s} {p}"),
        _ => s,
    }
}

fn outcome(b: &Breadcrumb) -> String {
    match b.outcome.as_deref() {
        Some("error") => format!(" → error {}", b.error_code.as_deref().unwrap_or("E_UNKNOWN")),
        Some("cancelled") => " (cancelled)".into(),
        _ => String::new(),
    }
}

/// The steps, numbered, oldest first; consecutive identical steps are merged ("(×3)"). `failure`, when
/// given, is added as the last step (e.g. "Error E_IO: disk full").
pub fn steps(crumbs: &[Breadcrumb], failure: Option<&str>) -> Vec<String> {
    let mut lines: Vec<(String, usize)> = Vec::new();
    for b in crumbs {
        let line = format!("{}{}", describe(b), outcome(b));
        match lines.last_mut() {
            Some((last, n)) if *last == line => *n += 1,
            _ => lines.push((line, 1)),
        }
    }
    let skipped = lines.len().saturating_sub(MAX_STEPS);
    let mut out: Vec<String> = Vec::new();
    if skipped > 0 {
        out.push(format!("… ({skipped} earlier steps)"));
    }
    for (line, n) in lines.into_iter().skip(skipped) {
        out.push(if n > 1 { format!("{line} (×{n})") } else { line });
    }
    if let Some(f) = failure {
        out.push(f.to_string());
    }
    out.into_iter().enumerate().map(|(i, s)| format!("{}. {s}", i + 1)).collect()
}

/// [`steps`] as one text, one step per line.
pub fn steps_text(crumbs: &[Breadcrumb], failure: Option<&str>) -> String {
    steps(crumbs, failure).join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn b(op: &str, trigger: &str, params: &[(&str, &str)]) -> Breadcrumb {
        Breadcrumb {
            time: "2026-10-17T08:00:00Z".into(),
            operation: op.into(),
            trigger: trigger.into(),
            operation_id: None,
            outcome: None,
            error_code: None,
            duration_ms: None,
            view: None,
            window: "main".into(),
            params: params
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    #[test]
    fn agent_steps_say_what_the_ai_did() {
        let crumbs = vec![
            b(
                "browser.agent",
                "engine",
                &[("Action", "navigate"), ("Site", "learn.example.org"), ("Outcome", "approved")],
            ),
            b(
                "browser.agent",
                "engine",
                &[("Action", "click"), ("Site", ""), ("Outcome", "denied")],
            ),
            b("browser.agent.stop", "button", &[]),
            b("browser.capture", "button", &[]),
        ];
        let s = steps(&crumbs, None);
        assert!(s[0].starts_with("1. AI agent navigate learn.example.org (approved)"), "{s:?}");
        assert!(s[1].starts_with("2. AI agent click in the browser (denied)"), "{s:?}");
        assert!(
            s[2].starts_with("3. Stopped the AI agent") && s[3].starts_with("4. Added a web page to the AI chat"),
            "{s:?}"
        );
    }

    #[test]
    fn golden_journey() {
        let mut export = b("export.csv", "menu", &[("Title", "Export CSV")]);
        export.outcome = Some("error".into());
        export.error_code = Some("E_IO".into());
        let mut cancelled = b("file.export", "dialog", &[("Format", "csv")]);
        cancelled.outcome = Some("cancelled".into());
        let crumbs = vec![
            b("file.open", "drag-drop", &[("Path", "<dir#1a2b3c4d>/samples"), ("Files", "3")]),
            b("query.change", "unknown", &[("QueryHash", "9f86d081"), ("QueryLength", "12")]),
            b("selection.change", "unknown", &[("RecordId", "42")]),
            b("selection.change", "unknown", &[("RecordId", "42")]),
            b("dock.popout", "button", &[("Panel", "Inspector")]),
            b("settings.change", "dialog", &[("Section", "diagnostics"), ("Fields", "logLevel")]),
            export,
            cancelled,
        ];
        assert_eq!(
            steps(&crumbs, Some("Error E_IO: disk full")),
            vec![
                "1. Opened <dir#1a2b3c4d>/samples (3 files) by drag and drop",
                "2. Applied query (hash 9f86d081)",
                "3. Selected event 42 (×2)",
                "4. Popped out Inspector",
                "5. Changed settings diagnostics: logLevel in a dialog",
                "6. Clicked 'Export CSV' from the menu → error E_IO",
                "7. Exported csv events in a dialog (cancelled)",
                "8. Error E_IO: disk full",
            ]
        );
    }

    #[test]
    fn golden_detailed_and_fallbacks() {
        let crumbs = vec![
            b("query.change", "palette", &[("Query", "level:error"), ("QueryHash", "x")]),
            b("search.change", "unknown", &[("SearchHash", "abcd1234")]),
            b("ipc.save_profile", "program", &[("Command", "save_profile")]),
            b("ipc.reload", "program", &[]),
            b("plugin.thing", "shortcut", &[]),
            b("dock.close", "shortcut", &[]),
            b("timeRange.change", "unknown", &[]),
            b("filter.change", "unknown", &[("Filters", "2")]),
            b("live.on", "button", &[]),
            b("edit.undo", "shortcut", &[]),
            b("file.open", "native-menu", &[]),
            b("file.save", "context-menu", &[("File", "p.imtproj")]),
            b("dock.layout", "menu", &[]),
            b("ai.ask", "button", &[]),
        ];
        assert_eq!(
            steps(&crumbs, None),
            vec![
                "1. Applied query \"level:error\" from the command palette",
                "2. Searched for (hash abcd1234)",
                "3. Ran save_profile",
                "4. Ran reload",
                "5. Ran plugin.thing with a keyboard shortcut",
                "6. Closed a panel with a keyboard shortcut",
                "7. Changed the time range",
                "8. Changed the filters (2 active)",
                "9. Turned live tail on with a button",
                "10. Undid the last change with a keyboard shortcut",
                "11. Opened files from the app menu",
                "12. Saved p.imtproj from a context menu",
                "13. Applied a panel layout from the menu",
                "14. Asked the AI assistant with a button",
            ]
        );
        assert_eq!(steps_text(&[], None), "");
        let mut e = b("x", "unknown", &[]);
        e.outcome = Some("error".into());
        assert_eq!(steps(&[e], None), vec!["1. Ran x → error E_UNKNOWN"]);
    }

    #[test]
    fn long_histories_are_cut() {
        let crumbs: Vec<Breadcrumb> = (0..40).map(|i| b(&format!("op{i}"), "unknown", &[])).collect();
        let s = steps(&crumbs, None);
        assert_eq!(s.len(), MAX_STEPS + 1);
        assert_eq!(s[0], "1. … (10 earlier steps)");
        assert_eq!(s[MAX_STEPS], "31. Ran op39");
    }
}
