//! The declared contract of every dialog (ADR 0054, Paddock ADR 0009): its kind, modality, minimum size and the IPC
//! commands its window may call. The TypeScript registry (`dialog-registry.ts`) is the source; `dialog-contracts.json`
//! is generated from it by a unit test (which fails when the copy is stale) and read here, the same way
//! `generated_types.rs` mirrors `api.types.ts`.
//!
//! This module has no Debrief dependency and is written to move to the shared `paddock-dialogs` crate unchanged: it
//! parses a contract file, validates it once (parse, don't validate) and answers "may dialog X call command Y?". The
//! guard in `commands.rs` asks it for every call from a dialog window.

use serde::Deserialize;
use std::collections::BTreeMap;

/// The four kinds of dialog (see the ADR): the kind decides modality limits, size, buttons and keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Decision,
    Form,
    Viewer,
    Tool,
}

impl Kind {
    /// Whether a dialog of this kind may be `modality`: a decision is always modal, a viewer and a tool never, a form either.
    pub fn allows(self, modality: Modality) -> bool {
        match self {
            Kind::Decision => modality == Modality::Modal,
            Kind::Form => true,
            Kind::Viewer | Kind::Tool => modality == Modality::Modeless,
        }
    }
}

/// Whether a dialog blocks the window that opened it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Modality {
    Modal,
    Modeless,
}

/// A window size in logical px.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

/// One dialog's contract.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Contract {
    pub kind: Kind,
    pub modality: Modality,
    /// A modal without a Cancel: Escape and a close request leave it open.
    pub required_choice: bool,
    /// One window per distinct input.
    pub instanced: bool,
    /// The smallest window of the dialog (the kind's floor, e.g. a viewer's 680x560).
    pub min_size: Option<Size>,
    /// The commands its window may call beyond the base ones; a trailing `*` is a prefix.
    pub commands: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct File {
    version: u32,
    base: Vec<String>,
    dialogs: BTreeMap<String, Contract>,
}

/// All contracts of an app plus the commands every dialog window may call.
#[derive(Debug, Clone, PartialEq)]
pub struct Contracts {
    base: Vec<String>,
    dialogs: BTreeMap<String, Contract>,
}

fn covers(declared: &[String], command: &str) -> bool {
    declared.iter().any(|d| match d.strip_suffix('*') {
        Some(prefix) => command.starts_with(prefix),
        None => d == command,
    })
}

fn valid_name(n: &str) -> bool {
    let n = n.strip_suffix('*').unwrap_or(n);
    !n.is_empty() && n.len() <= 64 && n.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

impl Contracts {
    /// Parses a contract file; fails on a version it does not know, a kind/modality combination the kind forbids, a
    /// required choice that is not modal, or a malformed id or command name.
    pub fn parse(json: &str) -> Result<Self, String> {
        let file: File = serde_json::from_str(json).map_err(|e| format!("dialog contracts: {e}"))?;
        if file.version != 1 {
            return Err(format!("dialog contracts: unknown version {}", file.version));
        }
        for c in &file.base {
            if !valid_name(c) {
                return Err(format!("dialog contracts: bad base command {c:?}"));
            }
        }
        for (id, c) in &file.dialogs {
            if id.is_empty() || id.len() > 64 || !id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
                return Err(format!("dialog contracts: bad dialog id {id:?}"));
            }
            if !c.kind.allows(c.modality) {
                return Err(format!("dialog contracts: {id}: a {:?} dialog cannot be {:?}", c.kind, c.modality));
            }
            if c.required_choice && c.modality != Modality::Modal {
                return Err(format!("dialog contracts: {id}: only a modal can be a required choice"));
            }
            if let Some(bad) = c.commands.iter().find(|n| !valid_name(n)) {
                return Err(format!("dialog contracts: {id}: bad command {bad:?}"));
            }
        }
        Ok(Self {
            base: file.base,
            dialogs: file.dialogs,
        })
    }

    /// No dialogs at all (what an app falls back to when its contract file is unusable).
    pub fn empty() -> Self {
        Self {
            base: Vec::new(),
            dialogs: BTreeMap::new(),
        }
    }

    /// The contract of dialog `id`.
    pub fn get(&self, id: &str) -> Option<&Contract> {
        self.dialogs.get(id)
    }

    /// Every dialog id, sorted (for the tests that walk the registry).
    #[cfg(test)]
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.dialogs.keys().map(String::as_str)
    }

    /// The commands every dialog window may call (for the tests that check them against the table).
    #[cfg(test)]
    pub fn base(&self) -> &[String] {
        &self.base
    }

    /// Whether the window of dialog `id` may call `command`: a base command, or one its contract declares. An
    /// unknown dialog may call nothing.
    pub fn allows(&self, id: &str, command: &str) -> bool {
        self.get(id)
            .is_some_and(|c| covers(&self.base, command) || covers(&c.commands, command))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "version": 1,
        "base": ["dialog_close"],
        "dialogs": {
            "about": { "kind": "viewer", "modality": "modeless", "requiredChoice": false, "instanced": false,
                       "minSize": { "width": 680, "height": 560 }, "commands": ["app_status", "ui_*"] },
            "close-prompt": { "kind": "decision", "modality": "modal", "requiredChoice": true, "instanced": false,
                              "minSize": null, "commands": [] }
        }
    }"#;

    #[test]
    fn a_dialog_may_call_the_base_commands_and_its_own_only() {
        let c = Contracts::parse(SAMPLE).unwrap();
        assert!(c.allows("about", "dialog_close") && c.allows("about", "app_status") && c.allows("about", "ui_sync_x"));
        assert!(!c.allows("about", "resolve_close"), "another dialog's command");
        assert!(c.allows("close-prompt", "dialog_close") && !c.allows("close-prompt", "app_status"));
        assert!(!c.allows("unknown", "dialog_close"), "an unknown dialog may call nothing");
        assert_eq!(
            c.get("about").and_then(|a| a.min_size),
            Some(Size {
                width: 680.0,
                height: 560.0
            })
        );
        assert_eq!(c.ids().collect::<Vec<_>>(), ["about", "close-prompt"]);
        assert_eq!(c.base(), ["dialog_close"]);
    }

    #[test]
    fn a_contract_file_that_contradicts_itself_is_refused() {
        let with = |from: &str, to: &str| SAMPLE.replace(from, to);
        for (what, json) in [
            (
                "a modeless decision",
                with(
                    r#""kind": "decision", "modality": "modal""#,
                    r#""kind": "decision", "modality": "modeless""#,
                ),
            ),
            (
                "a modal viewer",
                with(
                    r#""kind": "viewer", "modality": "modeless""#,
                    r#""kind": "viewer", "modality": "modal""#,
                ),
            ),
            (
                "an unknown field",
                with(r#""requiredChoice": true"#, r#""requiredChoice": true, "x": 1"#),
            ),
            ("version 2", with(r#""version": 1"#, r#""version": 2"#)),
            ("a bad command", with(r#""app_status""#, r#""App Status""#)),
            ("a bad id", with(r#""about""#, r#""../about""#)),
            ("not json", "{".into()),
        ] {
            assert!(Contracts::parse(&json).is_err(), "{what}");
        }
    }

    #[test]
    fn only_a_modal_can_be_a_required_choice() {
        let json = SAMPLE.replace(
            r#""kind": "viewer", "modality": "modeless", "requiredChoice": false"#,
            r#""kind": "form", "modality": "modeless", "requiredChoice": true"#,
        );
        assert!(Contracts::parse(&json).unwrap_err().contains("required choice"));
    }

    #[test]
    fn kinds_limit_modality() {
        assert!(Kind::Decision.allows(Modality::Modal) && !Kind::Decision.allows(Modality::Modeless));
        assert!(Kind::Form.allows(Modality::Modal) && Kind::Form.allows(Modality::Modeless));
        for k in [Kind::Viewer, Kind::Tool] {
            assert!(k.allows(Modality::Modeless) && !k.allows(Modality::Modal));
        }
    }
}
