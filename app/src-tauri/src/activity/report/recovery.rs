//! The report's `session-recovery.json`: whether the previous run closed properly, the running marker it
//! left (pid and start time only) and which windows' sessions a crash restore offers (labels only, never
//! the sessions' paths or queries).

use crate::lifecycle::{os_probe, MarkerFile, SessionStore};
use serde_json::{json, Value};
use std::path::Path;

/// Largest marker file read.
const MAX_MARKER: u64 = 4096;

fn marker(data: &Path) -> Value {
    let path = data.join("running.marker");
    let file = match std::fs::metadata(&path) {
        Err(_) => MarkerFile::Missing,
        Ok(m) if m.len() > MAX_MARKER => MarkerFile::Unreadable,
        Ok(_) => std::fs::read(&path)
            .map(|b| MarkerFile::parse(&String::from_utf8_lossy(&b)))
            .unwrap_or(MarkerFile::Unreadable),
    };
    match file {
        MarkerFile::Missing => json!({ "state": "missing" }),
        MarkerFile::Unreadable => json!({ "state": "unreadable" }),
        MarkerFile::Found(m) => {
            let alive = os_probe(m.pid).is_some_and(|now| m.started_at.is_none_or(|then| now.abs_diff(then) <= 1));
            json!({ "state": "found", "pid": m.pid, "startedAt": m.started_at, "processAlive": alive })
        }
    }
}

/// The recovery information from Debrief's data folder (none without one). `previous_crash` is what this run
/// decided at start, when known.
pub fn session_recovery(data: Option<&Path>, previous_crash: Option<bool>) -> Value {
    let Some(data) = data else {
        return json!({ "previousCrash": previous_crash, "runningMarker": null, "recoverableSessions": [] });
    };
    let sessions: Vec<String> = SessionStore::new(data.to_path_buf())
        .recovered()
        .into_iter()
        .map(|(label, _)| label)
        .collect();
    json!({
        "previousCrash": previous_crash,
        "runningMarker": marker(data),
        "recoverableSessions": sessions,
    })
}

/// One line for the report's file list.
pub fn summary(v: &Value) -> String {
    let sessions = v["recoverableSessions"].as_array().map(Vec::len).unwrap_or(0);
    let crash = match v["previousCrash"].as_bool() {
        Some(true) => "The previous run did not close properly",
        Some(false) => "The previous run closed properly",
        None => match v["runningMarker"]["state"].as_str() {
            Some("found") if v["runningMarker"]["processAlive"] == json!(false) => "Debrief did not close properly last time",
            Some("found") => "Debrief is running",
            Some("unreadable") => "The running marker is damaged",
            _ => "No crash marker",
        },
    };
    format!("{crash}; {sessions} session{} to restore", if sessions == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_and_sessions_are_reported_without_paths() {
        let d = crate::activity::report::tests::temp("recovery");
        let v = session_recovery(Some(&d), None);
        assert_eq!(v["runningMarker"]["state"], "missing");
        assert_eq!(summary(&v), "No crash marker; 0 sessions to restore");
        // A dead process's marker (pid 0 never runs Debrief) and one recovered session.
        std::fs::write(d.join("running.marker"), r#"{"version":1,"pid":4000000000,"startedAt":1}"#).unwrap();
        let s = d.join("recovered").join("main");
        std::fs::create_dir_all(&s).unwrap();
        std::fs::write(s.join("last-session.debriefproj"), "{}").unwrap();
        std::fs::write(d.join("recovered").join("manifest.json"), r#"{"version":2,"windows":["main"]}"#).unwrap();
        let v = session_recovery(Some(&d), None);
        assert_eq!(
            (v["runningMarker"]["pid"].as_u64(), v["runningMarker"]["processAlive"].as_bool()),
            (Some(4_000_000_000), Some(false))
        );
        assert_eq!(v["recoverableSessions"], json!(["main"]));
        assert!(!v.to_string().contains("debriefproj"));
        assert_eq!(summary(&v), "Debrief did not close properly last time; 1 session to restore");
        std::fs::write(d.join("running.marker"), "garbage").unwrap();
        assert_eq!(session_recovery(Some(&d), None)["runningMarker"]["state"], "unreadable");
        assert_eq!(
            summary(&session_recovery(Some(&d), Some(true))),
            "The previous run did not close properly; 1 session to restore"
        );
        assert_eq!(
            summary(&session_recovery(None, Some(false))),
            "The previous run closed properly; 0 sessions to restore"
        );
        std::fs::remove_dir_all(d).unwrap();
    }
}
