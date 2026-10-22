//! A redacted copy of a project (ADR 0041): which sources can be copied, the names and profile the copy carries, and
//! the request and report of the store operation that writes it.
//!
//! The copy is built by reading the redacted text of every copyable source into a new store, so every derived thing
//! (event columns, field statistics, diagnostics samples, histograms, the full-text index) is computed from redacted
//! text only. Sources that cannot be rebuilt from text (binary formats, CSV with its header, whole-file XML importers)
//! are left out, and the report says which and why.

use super::RuleHits;
use crate::api::{FileKind, SourceInfo};
use crate::package::ProjectManifest;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

/// The name of the file inside the project folder that holds the UI's state (notes, chats, saved queries, reports).
pub const UI_FILE: &str = "ui.json";

/// Importers whose events are one text record per `raw` (so `raw` joined by newlines is the file again).
const TEXT_IMPORTERS: &[&str] = &["text", "json"];

/// `redacted_export_project`: where to write the copy and how to redact.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedactedExportRequest {
    pub cfg: super::RedactCfg,
    pub manifest: ProjectManifest,
    pub out: PathBuf,
    /// The contents of `ui.json` for the copy, already redacted by the caller (it never sees the originals).
    pub ui_json: Option<String>,
}

/// A source the copy leaves out, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExcludedSource {
    pub name: String,
    pub reason: String,
}

/// What a redacted copy contains.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedactedExport {
    /// Size of the written `.imtz`.
    pub bytes: u64,
    pub sources: usize,
    pub events: usize,
    pub excluded: Vec<ExcludedSource>,
    /// Matches per rule over the copied text.
    pub hits: Vec<RuleHits>,
}

/// Why `source` cannot be copied, or `None` when it can.
pub fn excluded_reason(source: &SourceInfo) -> Option<String> {
    excluded_reason_of(&source.importer, source.file_kind)
}

/// [`excluded_reason`] from the importer and the file kind.
pub fn excluded_reason_of(importer: &str, kind: FileKind) -> Option<String> {
    if kind == FileKind::Binary {
        return Some("a binary format cannot be redacted".into());
    }
    if !TEXT_IMPORTERS.contains(&importer) {
        return Some(format!("the {importer} format cannot be rebuilt from redacted text"));
    }
    None
}

/// A file name for `redacted` (a source's redacted display name) under the copy's folder: each folder level made safe on
/// every file system, unique among `taken` (compared ignoring case).
pub fn copy_path(redacted: &str, taken: &mut HashSet<String>) -> String {
    let parts: Vec<String> = redacted
        .split(['/', '\\'])
        .filter(|p| !p.is_empty() && *p != "." && *p != "..")
        .map(|p| {
            let s: String = p
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || matches!(c, '.' | '_' | '-' | ' ' | '(' | ')') {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            let s = s.trim().chars().take(80).collect::<String>();
            if s.is_empty() || s.chars().all(|c| c == '.') {
                "_".into()
            } else {
                s
            }
        })
        .collect();
    let base = if parts.is_empty() {
        "source.log".to_string()
    } else {
        parts.join("/")
    };
    let mut candidate = base.clone();
    let mut n = 2;
    while !taken.insert(candidate.to_lowercase()) {
        candidate = match base.rsplit_once('.') {
            Some((stem, ext)) if !stem.ends_with('/') => format!("{stem} ({n}).{ext}"),
            _ => format!("{base} ({n})"),
        };
        n += 1;
    }
    candidate
}

/// The profile overrides of the copy: the original's without its `[redact]` section (custom rules may spell the very
/// secrets they hide). Text that is not valid TOML is dropped altogether.
pub fn without_redact_section(overrides: &str) -> String {
    if overrides.trim().is_empty() {
        return String::new();
    }
    match toml::from_str::<toml::Table>(overrides) {
        Ok(mut t) => {
            t.remove("redact");
            toml::to_string(&t).unwrap_or_default()
        }
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_text_sources_are_copied() {
        assert_eq!(excluded_reason_of("text", FileKind::Text), None);
        assert_eq!(excluded_reason_of("json", FileKind::Compressed), None);
        assert!(excluded_reason_of("registry", FileKind::Binary).unwrap().contains("binary"));
        assert!(excluded_reason_of("csv", FileKind::Text).unwrap().contains("csv"));
    }

    #[test]
    fn copy_paths_are_safe_relative_and_unique() {
        let mut taken = HashSet::new();
        assert_eq!(copy_path("web01/app.log", &mut taken), "web01/app.log");
        assert_eq!(copy_path("web01/APP.log", &mut taken), "web01/APP (2).log");
        assert_eq!(copy_path("../../etc/<email>:x", &mut taken), "etc/_email__x");
        assert_eq!(copy_path("", &mut taken), "source.log");
        assert_eq!(copy_path("..", &mut taken), "source (2).log");
    }

    #[test]
    fn the_copys_profile_loses_the_redact_section_only() {
        let t = without_redact_section(
            "name = 'x'\n[redact]\ndetectors = ['email']\n[[redact.rule]]\nname='s'\nregex='hunter2'\n[live]\nwindow='1m'\n",
        );
        assert!(!t.contains("hunter2") && !t.contains("redact") && t.contains("live") && t.contains("name"));
        assert_eq!(without_redact_section("  "), "");
        assert_eq!(without_redact_section("not [ toml"), "");
    }
}
