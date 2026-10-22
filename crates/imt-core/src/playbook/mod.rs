//! Investigation playbooks (ADR 0039): saved, shareable step lists, like a runbook. A playbook is a versioned JSON
//! file (`debrief.playbook`, see `docs/playbooks.md` for the schema); its steps are queries, filters, views to open,
//! checks with an expected outcome, or manual steps. The engine validates a playbook when it is read (untrusted
//! input), runs a step against the open data ([`run_step`]) and keeps the recorded results of a run ([`PlaybookRun`],
//! saved in the project). Reading never panics and every list is bounded.

mod library;
mod run;
mod templates;

pub use library::{export_to, import_from, list_library, remove_from_library, save_to_library, Library};
pub use run::{run_step, PlaybookRecord, PlaybookRun, StepRecord, StepResult, StepStatus, MAX_RUNS};
pub use templates::templates;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

/// The file format version this build writes and reads.
pub const PLAYBOOK_VERSION: u32 = 1;
/// Most steps in a playbook.
pub const MAX_STEPS: usize = 100;
/// Most tags on a playbook.
pub const MAX_TAGS: usize = 10;
/// Largest playbook file (bytes).
pub const MAX_FILE_BYTES: usize = 256 * 1024;
const MAX_TITLE: usize = 120;
const MAX_TEXT: usize = 4_000;
const MAX_QL: usize = 2_000;
const MAX_ID: usize = 60;

fn version_1() -> u32 {
    PLAYBOOK_VERSION
}

/// A saved investigation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Playbook {
    #[serde(default = "version_1")]
    pub version: u32,
    /// Lowercase letters, digits and `-` (also the file name in the library).
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub steps: Vec<Step>,
}

/// One step: what to read, and what the engine or the user does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    /// Unique within the playbook; same alphabet as the playbook id.
    pub id: String,
    pub title: String,
    /// What to look for and why (plain text or Markdown).
    #[serde(default)]
    pub instruction: String,
    pub action: Action,
}

/// What a step does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Action {
    /// Nothing to compute: the user does it (or reads it) and marks it done.
    Manual,
    /// Runs a query and shows what it found.
    Query { ql: String },
    /// Sets a filter as the current view (the engine checks it and counts the matches).
    Filter { ql: String },
    /// Opens a view (pane) of the app by its id.
    OpenView { view: String },
    /// Runs a query and passes or fails by what is expected.
    Check { ql: String, expect: Expect },
}

/// What a check expects of the number of matching events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Expect {
    /// No event matches.
    NoMatch,
    /// At least one event matches.
    AnyMatch,
    /// At least `n` events match.
    AtLeast { n: usize },
    /// At most `n` events match.
    AtMost { n: usize },
}

/// Whether `s` is a valid id: 1 to 60 of `a-z`, `0-9`, `-`, not starting or ending with `-`.
pub fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_ID
        && !s.starts_with('-')
        && !s.ends_with('-')
        && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn text(what: &str, s: &str, max: usize) -> Result<()> {
    if s.chars().count() > max {
        bail!("{what} is longer than {max} characters");
    }
    Ok(())
}

impl Playbook {
    /// Reads a playbook from untrusted JSON (a file, the UI, a project) and checks every limit. Playbooks of a newer
    /// version are refused with a message that says to update; older ones are read as they are.
    pub fn parse(value: Value) -> Result<Playbook> {
        if let Some(v) = value.get("version").and_then(Value::as_u64) {
            if v > u64::from(PLAYBOOK_VERSION) {
                bail!("this playbook was saved by a newer version of the app (format {v}); update the app to use it");
            }
        }
        let p: Playbook = serde_json::from_value(value).context("not a valid playbook")?;
        p.check()?;
        Ok(p)
    }

    /// Reads a playbook file's text.
    pub fn parse_str(s: &str) -> Result<Playbook> {
        if s.len() > MAX_FILE_BYTES {
            bail!("a playbook file is at most {} KB", MAX_FILE_BYTES / 1024);
        }
        Playbook::parse(serde_json::from_str(s).context("the file is not JSON")?)
    }

    /// The pretty JSON the library saves and `Export` writes.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Checks the limits and ids (the engine re-validates whatever the UI sends).
    pub fn check(&self) -> Result<()> {
        if !valid_id(&self.id) {
            bail!(
                "the playbook id must be 1 to {MAX_ID} lowercase letters, digits or dashes (got \"{}\")",
                self.id.chars().take(40).collect::<String>()
            );
        }
        if self.title.trim().is_empty() {
            bail!("a playbook needs a title");
        }
        text("the title", &self.title, MAX_TITLE)?;
        text("the description", &self.description, MAX_TEXT)?;
        if self.tags.len() > MAX_TAGS || self.tags.iter().any(|t| t.trim().is_empty() || t.chars().count() > 40) {
            bail!("a playbook has at most {MAX_TAGS} tags of 1 to 40 characters");
        }
        if self.steps.is_empty() {
            bail!("a playbook needs at least one step");
        }
        if self.steps.len() > MAX_STEPS {
            bail!("a playbook has at most {MAX_STEPS} steps");
        }
        let mut seen = BTreeSet::new();
        for (i, s) in self.steps.iter().enumerate() {
            let n = i + 1;
            if !valid_id(&s.id) {
                bail!("step {n}: the id must be lowercase letters, digits or dashes");
            }
            if !seen.insert(s.id.as_str()) {
                bail!("step {n}: the id \"{}\" is used twice", s.id);
            }
            if s.title.trim().is_empty() {
                bail!("step {n} needs a title");
            }
            text(&format!("step {n}'s title"), &s.title, MAX_TITLE)?;
            text(&format!("step {n}'s instruction"), &s.instruction, MAX_TEXT)?;
            match &s.action {
                Action::Manual => {}
                Action::Query { ql } | Action::Filter { ql } | Action::Check { ql, .. } => {
                    if ql.trim().is_empty() {
                        bail!("step {n} needs a query");
                    }
                    text(&format!("step {n}'s query"), ql, MAX_QL)?;
                }
                Action::OpenView { view } => {
                    if view.is_empty()
                        || view.len() > 60
                        || !view
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
                    {
                        bail!("step {n}: \"{}\" is not a view id", view.chars().take(40).collect::<String>());
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({
            "version": 1, "id": "slow-requests", "title": "Slow requests", "tags": ["perf"],
            "steps": [
                {"id": "look", "title": "Look", "action": {"type": "manual"}},
                {"id": "errors", "title": "Errors", "instruction": "x", "action": {"type": "query", "ql": "level >= error"}},
                {"id": "none", "title": "No crashes", "action": {"type": "check", "ql": "level = fatal", "expect": {"kind": "noMatch"}}},
                {"id": "view", "title": "Open", "action": {"type": "openView", "view": "problems"}}
            ]
        })
    }

    #[test]
    fn a_valid_playbook_round_trips() {
        let p = Playbook::parse(sample()).unwrap();
        assert_eq!(p.steps.len(), 4);
        let again = Playbook::parse_str(&p.to_json()).unwrap();
        assert_eq!(p, again);
        // The version defaults to the current one.
        let mut v = sample();
        v.as_object_mut().unwrap().remove("version");
        assert_eq!(Playbook::parse(v).unwrap().version, PLAYBOOK_VERSION);
    }

    #[test]
    fn invalid_playbooks_say_what_to_fix() {
        type Edit = Box<dyn Fn(&mut Value)>;
        let cases: Vec<(Edit, &str)> = vec![
            (Box::new(|v| v["version"] = json!(2)), "newer version"),
            (Box::new(|v| v["id"] = json!("Bad Id")), "lowercase letters"),
            (Box::new(|v| v["id"] = json!("../etc")), "lowercase letters"),
            (Box::new(|v| v["title"] = json!(" ")), "needs a title"),
            (Box::new(|v| v["tags"] = json!([""])), "tags"),
            (Box::new(|v| v["steps"] = json!([])), "at least one step"),
            (Box::new(|v| v["steps"][1]["id"] = json!("look")), "used twice"),
            (Box::new(|v| v["steps"][1]["id"] = json!("A")), "step 2: the id"),
            (Box::new(|v| v["steps"][1]["title"] = json!("")), "step 2 needs a title"),
            (Box::new(|v| v["steps"][1]["action"]["ql"] = json!(" ")), "needs a query"),
            (Box::new(|v| v["steps"][3]["action"]["view"] = json!("a b")), "not a view id"),
            (
                Box::new(|v| v["steps"][1]["instruction"] = json!("x".repeat(5000))),
                "instruction is longer",
            ),
            (
                Box::new(|v| v["steps"][2]["action"]["expect"] = json!({"kind": "sometimes"})),
                "not a valid playbook",
            ),
        ];
        for (edit, want) in cases {
            let mut v = sample();
            edit(&mut v);
            let e = format!("{:#}", Playbook::parse(v).unwrap_err());
            assert!(e.contains(want), "{want}: {e}");
        }
        let many: Vec<Value> = (0..=MAX_STEPS)
            .map(|i| json!({"id": format!("s{i}"), "title": "t", "action": {"type": "manual"}}))
            .collect();
        let mut v = sample();
        v["steps"] = json!(many);
        assert!(Playbook::parse(v).unwrap_err().to_string().contains("at most 100 steps"));
        assert!(Playbook::parse_str("not json").is_err());
        assert!(Playbook::parse_str(&" ".repeat(MAX_FILE_BYTES + 1)).is_err());
    }

    #[test]
    fn ids_are_file_name_safe() {
        for ok in ["a", "a-b", "x9"] {
            assert!(valid_id(ok), "{ok}");
        }
        for bad in ["", "-a", "a-", "A", "a_b", "a/b", "a.b", &"a".repeat(61)] {
            assert!(!valid_id(bad), "{bad}");
        }
    }
}
