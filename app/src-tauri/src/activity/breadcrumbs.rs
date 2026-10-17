//! The latest activity entries per window, kept in memory so an error, a crash, an error snapshot or a
//! problem report can show what the user did just before. Entries already follow the privacy rules
//! (they are what was logged), so keeping them adds nothing sensitive.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, VecDeque};

/// Entries kept per window.
pub const BREADCRUMBS_PER_WINDOW: usize = 200;
/// Entries attached to an error, a snapshot or a report.
pub const BREADCRUMBS_PER_ERROR: usize = 30;
/// Most windows tracked; the least recently active one is forgotten beyond it.
const MAX_WINDOWS: usize = 32;
/// Properties that are part of the entry itself, not parameters.
const RESERVED: &[&str] = &[
    "SourceContext",
    "Operation",
    "Trigger",
    "OperationId",
    "Outcome",
    "ErrorCode",
    "DurationMs",
    "View",
    "Window",
    "MessageTemplate",
    "Kind",
    "Severity",
];

/// One activity entry, as attached to errors and reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Breadcrumb {
    pub time: String,
    pub operation: String,
    pub trigger: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
    #[serde(default)]
    pub window: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, String>,
}

impl Breadcrumb {
    /// Builds an entry from a log record's properties (`Operation` is required).
    pub fn from_props(time: String, props: &BTreeMap<String, String>) -> Option<Breadcrumb> {
        let operation = props.get("Operation")?.clone();
        let get = |k: &str| props.get(k).filter(|v| !v.is_empty()).cloned();
        let params = props
            .iter()
            .filter(|(k, _)| !RESERVED.contains(&k.as_str()))
            .take(super::privacy::MAX_PARAMS)
            .map(|(k, v)| (k.clone(), super::privacy::safe_param(k, v)))
            .collect();
        Some(Breadcrumb {
            time,
            operation,
            trigger: get("Trigger").unwrap_or_else(|| "unknown".into()),
            operation_id: get("OperationId"),
            outcome: get("Outcome"),
            error_code: get("ErrorCode"),
            duration_ms: props.get("DurationMs").and_then(|d| d.parse().ok()),
            view: get("View"),
            window: get("Window").unwrap_or_else(|| crate::windows::MAIN.into()),
            params,
        })
    }
}

/// Ring buffers of the latest entries, per window.
#[derive(Debug, Default)]
pub struct Breadcrumbs {
    windows: HashMap<String, VecDeque<Breadcrumb>>,
    /// Window labels, least recently active first.
    order: VecDeque<String>,
}

impl Breadcrumbs {
    pub fn push(&mut self, b: Breadcrumb) {
        let window = b.window.clone();
        self.order.retain(|w| *w != window);
        self.order.push_back(window.clone());
        let ring = self.windows.entry(window).or_default();
        if ring.len() == BREADCRUMBS_PER_WINDOW {
            ring.pop_front();
        }
        ring.push_back(b);
        while self.order.len() > MAX_WINDOWS {
            if let Some(old) = self.order.pop_front() {
                self.windows.remove(&old);
            }
        }
    }

    /// The last `n` entries of `window`, oldest first.
    pub fn recent(&self, window: &str, n: usize) -> Vec<Breadcrumb> {
        self.windows
            .get(window)
            .map(|r| r.iter().skip(r.len().saturating_sub(n)).cloned().collect())
            .unwrap_or_default()
    }

    /// The last `n` entries over all windows, oldest first (for errors not tied to a window).
    pub fn recent_all(&self, n: usize) -> Vec<Breadcrumb> {
        let mut all: Vec<Breadcrumb> = self.windows.values().flatten().cloned().collect();
        all.sort_by(|a, b| a.time.cmp(&b.time));
        let skip = all.len().saturating_sub(n);
        all.split_off(skip)
    }

    /// The window's entries when it has any, else every window's.
    pub fn recent_for(&self, window: Option<&str>, n: usize) -> Vec<Breadcrumb> {
        match window {
            Some(w) if self.windows.contains_key(w) => self.recent(w, n),
            _ => self.recent_all(n),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crumb(window: &str, i: usize) -> Breadcrumb {
        let props: BTreeMap<String, String> = [
            ("Operation", format!("op{i}")),
            ("Trigger", "menu".into()),
            ("Window", window.into()),
            ("DurationMs", "12".into()),
            ("Files", "3".into()),
            ("ApiKey", "sk-123".into()),
            ("SourceContext", "IMT.Activity".into()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        Breadcrumb::from_props(format!("2026-10-17T08:00:{:02}.000000Z", i % 60), &props).unwrap()
    }

    #[test]
    fn parses_properties() {
        let b = crumb("main", 1);
        assert_eq!((b.operation.as_str(), b.trigger.as_str(), b.duration_ms), ("op1", "menu", Some(12)));
        assert_eq!(b.params.get("Files").map(String::as_str), Some("3"));
        assert_eq!(b.params.get("ApiKey").map(String::as_str), Some("***"));
        assert!(!b.params.contains_key("SourceContext"));
        assert!(Breadcrumb::from_props("t".into(), &BTreeMap::new()).is_none());
        let only_op: BTreeMap<String, String> = [("Operation".to_string(), "x".to_string())].into();
        let b = Breadcrumb::from_props("t".into(), &only_op).unwrap();
        assert_eq!((b.trigger.as_str(), b.window.as_str()), ("unknown", "main"));
    }

    #[test]
    fn rings_are_bounded_per_window() {
        let mut ring = Breadcrumbs::default();
        for i in 0..250 {
            ring.push(crumb("main", i));
        }
        ring.push(crumb("w2", 999));
        let main = ring.recent("main", 1000);
        assert_eq!(main.len(), BREADCRUMBS_PER_WINDOW);
        assert_eq!(main.last().unwrap().operation, "op249");
        assert_eq!(ring.recent("main", 2).len(), 2);
        assert_eq!(ring.recent("none", 2).len(), 0);
        assert_eq!(ring.recent_for(Some("w2"), 30).len(), 1);
        assert_eq!(ring.recent_for(Some("gone"), 30).len(), 30);
        assert_eq!(ring.recent_for(None, 5).len(), 5);
        for w in 0..40 {
            ring.push(crumb(&format!("w{w}"), w));
        }
        assert!(ring.windows.len() <= MAX_WINDOWS);
        assert!(ring.recent("main", 1).is_empty());
    }
}
