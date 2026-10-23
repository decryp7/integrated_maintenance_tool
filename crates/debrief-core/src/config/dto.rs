//! Wire types of the config diff (mirrored in
//! `app/src/app/core/config-diff.types.ts`, camelCase JSON).

use super::diff::DEFAULT_IGNORE;
use super::ConfigValue;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

/// Longest value text sent to the UI; longer values are clipped.
pub const MAX_VALUE_TEXT: usize = 16 * 1024;
/// Most rows in one page.
pub const MAX_PAGE: usize = 5000;
/// Most ignore patterns per request.
pub const MAX_IGNORE_PATTERNS: usize = 200;
/// Longest search text.
pub const MAX_SEARCH: usize = 1000;
/// Most expanded/collapsed overrides per request.
pub const MAX_TOGGLED: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeKind {
    Added,
    Removed,
    Changed,
    Unchanged,
}

impl ChangeKind {
    pub const ALL: [ChangeKind; 4] = [ChangeKind::Added, ChangeKind::Removed, ChangeKind::Changed, ChangeKind::Unchanged];

    pub fn as_str(self) -> &'static str {
        match self {
            ChangeKind::Added => "added",
            ChangeKind::Removed => "removed",
            ChangeKind::Changed => "changed",
            ChangeKind::Unchanged => "unchanged",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ValueKind {
    Section,
    Null,
    Bool,
    Integer,
    Float,
    String,
    Binary,
    List,
}

/// A value as shown: its type and text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueDto {
    pub kind: ValueKind,
    pub text: String,
    /// The text was cut at [`MAX_VALUE_TEXT`].
    pub clipped: bool,
}

impl From<&ConfigValue> for ValueDto {
    fn from(v: &ConfigValue) -> Self {
        let kind = match v {
            ConfigValue::Section => ValueKind::Section,
            ConfigValue::Null => ValueKind::Null,
            ConfigValue::Bool(_) => ValueKind::Bool,
            ConfigValue::Integer(_) => ValueKind::Integer,
            ConfigValue::Float(_) => ValueKind::Float,
            ConfigValue::String(_) => ValueKind::String,
            ConfigValue::Binary(_) => ValueKind::Binary,
            ConfigValue::List(_) => ValueKind::List,
        };
        let mut text = v.text();
        let clipped = text.len() > MAX_VALUE_TEXT;
        if clipped {
            let mut end = MAX_VALUE_TEXT;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
            text.push('…');
        }
        ValueDto { kind, text, clipped }
    }
}

/// How two trees are compared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CompareOptions {
    /// Paths to leave out (with their subtrees): globs over the display path
    /// (`*InstallDate*`, `HKEY_*/Software/*/Cache`), or regexes as `re:<pattern>`.
    /// Both ignore case.
    pub ignore: Vec<String>,
    /// Compare paths without regard to case even when neither format asks for it.
    pub case_insensitive: bool,
}

impl Default for CompareOptions {
    fn default() -> Self {
        CompareOptions {
            ignore: DEFAULT_IGNORE.iter().map(|s| s.to_string()).collect(),
            case_insensitive: false,
        }
    }
}

/// Which sections are expanded by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExpandMode {
    #[default]
    All,
    /// Only sections with changes inside.
    ChangedOnly,
}

/// What part of a diff to show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DiffView {
    /// Kinds of change to show (their ancestors are shown as context).
    pub kinds: Vec<ChangeKind>,
    /// Case-insensitive text in the path or either value.
    pub search: String,
    pub expand: ExpandMode,
    /// Display paths of sections whose expanded state is flipped from the default.
    pub toggled: Vec<String>,
    pub offset: usize,
    pub limit: usize,
}

impl Default for DiffView {
    fn default() -> Self {
        DiffView {
            kinds: ChangeKind::ALL.to_vec(),
            search: String::new(),
            expand: ExpandMode::All,
            toggled: Vec::new(),
            offset: 0,
            limit: 500,
        }
    }
}

impl DiffView {
    /// Fails fast on oversized requests.
    pub fn validate(&self) -> Result<()> {
        if self.search.chars().count() > MAX_SEARCH {
            return Err(anyhow!("the search text is longer than {MAX_SEARCH} characters"));
        }
        if self.toggled.len() > MAX_TOGGLED {
            return Err(anyhow!("too many expanded or collapsed sections (limit {MAX_TOGGLED})"));
        }
        Ok(())
    }
}

/// `diff_configs` options: how to compare and what to show.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DiffOptions {
    pub compare: CompareOptions,
    pub view: DiffView,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffCounts {
    pub added: usize,
    pub removed: usize,
    pub changed: usize,
    pub unchanged: usize,
    /// Nodes left out by ignore patterns (both sides).
    pub ignored: usize,
}

impl DiffCounts {
    /// Added, removed and changed nodes.
    pub fn differences(&self) -> usize {
        self.added + self.removed + self.changed
    }
}

/// One file of a diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SideInfo {
    pub path: String,
    /// Parser id (`json`, `registry`...).
    pub format: String,
    pub nodes: usize,
    /// The file had more nodes than the limit; the rest is not compared.
    pub truncated: bool,
}

/// One row of the tree diff, in tree order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffRow {
    /// Display path (`servers[0]/host`), unique within the diff.
    pub path: String,
    /// Last path segment.
    pub name: String,
    /// Nesting level (0 = top).
    pub depth: usize,
    pub kind: ChangeKind,
    pub old: Option<ValueDto>,
    pub new: Option<ValueDto>,
    /// Something below this node changed.
    pub has_changes: bool,
    /// Shown children exist.
    pub expandable: bool,
    pub expanded: bool,
    /// Shown only as an ancestor of a matching row.
    pub context: bool,
}

/// One page of a diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffPage {
    pub left: SideInfo,
    pub right: SideInfo,
    pub case_insensitive: bool,
    pub counts: DiffCounts,
    /// Rows shown (after filters and collapsing), across all pages.
    pub total: usize,
    pub offset: usize,
    pub rows: Vec<DiffRow>,
    /// A file reached the node limit.
    pub truncated: bool,
}

/// A configuration format Debrief can read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormatInfo {
    pub id: String,
    pub name: String,
    pub extensions: Vec<String>,
}

/// Formats and default options, for the diff view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffFormats {
    pub formats: Vec<FormatInfo>,
    pub default_ignore: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_validation() {
        let o: DiffOptions = serde_json::from_str("{}").unwrap();
        assert_eq!(o.compare.ignore.len(), DEFAULT_IGNORE.len());
        assert_eq!(o.view.kinds.len(), 4);
        assert_eq!(o.view.limit, 500);
        let o: DiffOptions =
            serde_json::from_str(r#"{"compare":{"ignore":[],"caseInsensitive":true},"view":{"kinds":["added"],"expand":"changedOnly"}}"#)
                .unwrap();
        assert!(o.compare.case_insensitive && o.compare.ignore.is_empty());
        assert_eq!((o.view.kinds, o.view.expand), (vec![ChangeKind::Added], ExpandMode::ChangedOnly));
        let mut v = DiffView::default();
        assert!(v.validate().is_ok());
        v.search = "x".repeat(MAX_SEARCH + 1);
        assert!(v.validate().is_err());
        v.search.clear();
        v.toggled = vec![String::new(); MAX_TOGGLED + 1];
        assert!(v.validate().is_err());
        assert_eq!(
            ChangeKind::ALL.map(ChangeKind::as_str),
            ["added", "removed", "changed", "unchanged"]
        );
        let c = DiffCounts {
            added: 1,
            removed: 2,
            changed: 3,
            unchanged: 4,
            ignored: 5,
        };
        assert_eq!(c.differences(), 6);
    }

    #[test]
    fn values_are_typed_and_clipped() {
        let kinds = [
            (ConfigValue::Section, ValueKind::Section),
            (ConfigValue::Null, ValueKind::Null),
            (ConfigValue::Bool(true), ValueKind::Bool),
            (ConfigValue::Integer(1), ValueKind::Integer),
            (ConfigValue::Float(1.0), ValueKind::Float),
            (ConfigValue::String("s".into()), ValueKind::String),
            (ConfigValue::Binary(vec![1]), ValueKind::Binary),
            (ConfigValue::List(vec![]), ValueKind::List),
        ];
        for (v, k) in kinds {
            assert_eq!(ValueDto::from(&v).kind, k);
        }
        let long = ConfigValue::String(format!("a{}", "é".repeat(MAX_VALUE_TEXT)));
        let dto = ValueDto::from(&long);
        assert!(dto.clipped && dto.text.ends_with('…'));
        assert!(dto.text.len() <= MAX_VALUE_TEXT + '…'.len_utf8());
        assert!(!ValueDto::from(&ConfigValue::Null).clipped);
    }
}
