//! `[[span]]` rules: parsing, validation (with profile lines) and resolving
//! their marker kinds against the effective marker set.

use crate::model::MarkerMask;
use crate::profile::{Marker, ProfileError};
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

/// Most `[[span]]` rules in one profile.
pub const MAX_SPAN_RULES: usize = 16;
/// Most marker kinds one `start` or `stop` may list.
pub const MAX_TRIGGER_KINDS: usize = 16;
/// Longest span kind (an id).
pub const MAX_KIND_LEN: usize = 64;
/// Longest span label.
pub const MAX_LABEL_LEN: usize = 200;

/// A marker kind, or several, as TOML writes them: `"start"` or `["a", "b"]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    pub fn list(&self) -> Vec<String> {
        match self {
            OneOrMany::One(s) => vec![s.clone()],
            OneOrMany::Many(v) => v.clone(),
        }
    }
}

/// `[[span]]`: runs between a start and a stop marker, paired per lane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpanCfg {
    /// Id of the span kind (letters, digits, `-`, `_`, `.`).
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Marker kind(s) that start a run.
    pub start: OneOrMany,
    /// Marker kind(s) that stop a run.
    pub stop: OneOrMany,
    /// `"instance"` (each timeline lane) or `"field:<Name>"` (each value of a field within a lane).
    #[serde(default = "default_per")]
    pub per: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Runs longer than this are flagged (`"12h"`, `"90m"`, `"2d"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_duration: Option<String>,
    /// A start while running ends the previous run as "no stop seen" (else the start is ignored).
    #[serde(default = "yes")]
    pub restart_on_start: bool,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn default_per() -> String {
    "instance".into()
}

fn yes() -> bool {
    true
}

/// What runs are paired within.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Per {
    /// Each timeline lane (the instance key).
    Instance,
    /// Each value of this field within a lane (e.g. `ProcessId` in a shared file).
    Field(String),
}

impl Per {
    /// Parses `"instance"` or `"field:<Name>"`.
    pub fn parse(text: &str) -> Result<Per> {
        let t = text.trim();
        if t.eq_ignore_ascii_case("instance") || t.eq_ignore_ascii_case("lane") {
            return Ok(Per::Instance);
        }
        match t.strip_prefix("field:").map(str::trim) {
            Some(name) if !name.is_empty() && name.len() <= 200 => Ok(Per::Field(name.to_string())),
            _ => Err(anyhow!(
                "per \"{text}\" must be \"instance\" or \"field:<Name>\" (e.g. \"field:ProcessId\")"
            )),
        }
    }
}

/// A validated `[[span]]` rule; `start_mask`/`stop_mask` are set by [`resolve`].
#[derive(Debug, Clone)]
pub struct SpanRule {
    pub kind: String,
    pub label: String,
    pub start: Vec<String>,
    pub stop: Vec<String>,
    pub per: Per,
    pub color: Option<String>,
    pub max_duration_ms: Option<i64>,
    pub restart_on_start: bool,
    pub enabled: bool,
    pub start_mask: MarkerMask,
    pub stop_mask: MarkerMask,
    /// Marker kinds the rule names that are not defined (or switched off): such a rule matches nothing for them.
    pub missing: Vec<String>,
}

/// Which key of a `[[span]]` an error is about (for its profile line).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpanField {
    Kind,
    Label,
    Start,
    Stop,
    Per,
    Color,
    MaxDuration,
    General,
}

impl SpanField {
    /// The profile key errors about this field point at (`None` for problems of the whole rule).
    pub fn key(self) -> Option<&'static str> {
        Some(match self {
            SpanField::Kind => "kind",
            SpanField::Label => "label",
            SpanField::Start => "start",
            SpanField::Stop => "stop",
            SpanField::Per => "per",
            SpanField::Color => "color",
            SpanField::MaxDuration => "max_duration",
            SpanField::General => return None,
        })
    }
}

/// One problem with a span rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpanIssue {
    pub field: SpanField,
    pub message: String,
}

fn issue(field: SpanField, message: String) -> SpanIssue {
    SpanIssue { field, message }
}

pub(crate) fn valid_kind(kind: &str) -> bool {
    !kind.is_empty() && kind.len() <= MAX_KIND_LEN && kind.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn is_hex_color(c: &str) -> bool {
    c.strip_prefix('#')
        .is_some_and(|h| matches!(h.len(), 3 | 6) && h.chars().all(|ch| ch.is_ascii_hexdigit()))
}

/// Checks the trigger list of `start` or `stop`.
pub(crate) fn check_kinds(list: &[String], field: SpanField, out: &mut Vec<SpanIssue>) {
    let name = field.key().unwrap_or("");
    if list.is_empty() || list.iter().all(|k| k.trim().is_empty()) {
        out.push(issue(field, format!("{name} needs at least one marker kind")));
    } else if list.len() > MAX_TRIGGER_KINDS {
        out.push(issue(
            field,
            format!("{name} lists {} marker kinds; at most {MAX_TRIGGER_KINDS}", list.len()),
        ));
    } else if list.iter().any(|k| k.trim().is_empty() || k.len() > 200) {
        out.push(issue(field, format!("{name} has an empty or overlong marker kind")));
    }
}

/// Every problem of one rule (empty = valid). Marker kinds are checked later, by [`resolve`].
pub fn check(cfg: &SpanCfg) -> Vec<SpanIssue> {
    let mut out = Vec::new();
    if !valid_kind(&cfg.kind) {
        out.push(issue(
            SpanField::Kind,
            format!(
                "kind \"{}\" must be 1–{MAX_KIND_LEN} letters, digits, '-', '_' or '.' (e.g. \"run\")",
                cfg.kind
            ),
        ));
    }
    if cfg.label.as_deref().is_some_and(|l| l.chars().count() > MAX_LABEL_LEN) {
        out.push(issue(SpanField::Label, format!("label is longer than {MAX_LABEL_LEN} characters")));
    }
    check_kinds(&cfg.start.list(), SpanField::Start, &mut out);
    check_kinds(&cfg.stop.list(), SpanField::Stop, &mut out);
    if let Err(e) = Per::parse(&cfg.per) {
        out.push(issue(SpanField::Per, e.to_string()));
    }
    if let Some(c) = cfg.color.as_deref().filter(|c| !is_hex_color(c)) {
        out.push(issue(SpanField::Color, format!("color \"{c}\" must be #RGB or #RRGGBB")));
    }
    if let Some(d) = &cfg.max_duration {
        if let Err(e) = crate::workspace::duration_ms(d) {
            out.push(issue(SpanField::MaxDuration, format!("max_duration: {e}")));
        }
    }
    out
}

/// Validates and compiles every rule; the first problem fails with `span "<kind>": ...`.
pub fn compile(cfgs: &[SpanCfg]) -> Result<Vec<SpanRule>> {
    if cfgs.len() > MAX_SPAN_RULES {
        return Err(anyhow!("at most {MAX_SPAN_RULES} [[span]] rules are supported"));
    }
    let mut rules: Vec<SpanRule> = Vec::new();
    for cfg in cfgs {
        if let Some(first) = check(cfg).into_iter().next() {
            return Err(anyhow!("span \"{}\": {}", cfg.kind, first.message));
        }
        if rules.iter().any(|r| r.kind == cfg.kind) {
            return Err(anyhow!("span \"{}\" is defined twice", cfg.kind));
        }
        rules.push(SpanRule {
            kind: cfg.kind.clone(),
            label: cfg
                .label
                .clone()
                .filter(|l| !l.trim().is_empty())
                .unwrap_or_else(|| cfg.kind.clone()),
            start: trimmed(&cfg.start),
            stop: trimmed(&cfg.stop),
            per: Per::parse(&cfg.per)?,
            color: cfg.color.clone(),
            max_duration_ms: cfg.max_duration.as_deref().map(crate::workspace::duration_ms).transpose()?,
            restart_on_start: cfg.restart_on_start,
            enabled: cfg.enabled,
            start_mask: 0,
            stop_mask: 0,
            missing: Vec::new(),
        });
    }
    Ok(rules)
}

fn trimmed(list: &OneOrMany) -> Vec<String> {
    list.list().iter().map(|k| k.trim().to_string()).collect()
}

/// Bits of the marker kinds in `kinds`, and the kinds that are not defined.
pub(crate) fn mask_of(kinds: &[String], markers: &[Marker]) -> (MarkerMask, Vec<String>) {
    let mut mask: MarkerMask = 0;
    let mut missing = Vec::new();
    for k in kinds {
        match markers.iter().position(|m| m.kind == *k) {
            Some(i) => mask |= 1 << i,
            None => missing.push(k.clone()),
        }
    }
    (mask, missing)
}

/// Sets each rule's marker bits from the effective marker set (call again whenever it changes).
pub fn resolve(rules: &mut [SpanRule], markers: &[Marker]) {
    for r in rules {
        let (start, mut missing) = mask_of(&r.start, markers);
        let (stop, more) = mask_of(&r.stop, markers);
        missing.extend(more);
        missing.dedup();
        (r.start_mask, r.stop_mask, r.missing) = (start, stop, missing);
    }
}

/// Checks the `[[span]]` entries of a user profile before it is merged, so a
/// problem points at the line of the offending rule (or key).
pub fn check_toml(user: &toml::Table, text: &str) -> std::result::Result<(), ProfileError> {
    let Some(value) = user.get("span") else { return Ok(()) };
    let at = |index: usize, key: Option<&str>, message: String| ProfileError {
        message,
        line: rule_line(text, index, key),
    };
    let Some(list) = value.as_array() else {
        return Err(ProfileError {
            message: "span must be a list of [[span]] tables".into(),
            line: key_line(text, "span"),
        });
    };
    if list.len() > MAX_SPAN_RULES {
        return Err(at(
            MAX_SPAN_RULES,
            None,
            format!("at most {MAX_SPAN_RULES} [[span]] rules are supported"),
        ));
    }
    let mut seen: Vec<String> = Vec::new();
    for (i, v) in list.iter().enumerate() {
        let cfg: SpanCfg = v
            .clone()
            .try_into()
            .map_err(|e: toml::de::Error| at(i, None, format!("[[span]] #{}: {}", i + 1, e.message())))?;
        if let Some(first) = check(&cfg).into_iter().next() {
            return Err(at(i, first.field.key(), format!("span \"{}\": {}", cfg.kind, first.message)));
        }
        if seen.contains(&cfg.kind) {
            return Err(at(i, Some("kind"), format!("span \"{}\" is defined twice", cfg.kind)));
        }
        seen.push(cfg.kind);
    }
    Ok(())
}

/// 1-based line of the `index`-th `[[span]]` header, or of `key` inside that rule.
fn rule_line(text: &str, index: usize, key: Option<&str>) -> Option<usize> {
    let lines: Vec<&str> = text.lines().collect();
    let header = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.trim_start().starts_with("[[span]]"))
        .nth(index)
        .map(|(i, _)| i)?;
    let Some(key) = key else { return Some(header + 1) };
    let found = lines[header + 1..]
        .iter()
        .take_while(|l| !l.trim_start().starts_with('['))
        .position(|l| is_key_line(l, key));
    Some(found.map_or(header, |p| header + 1 + p) + 1)
}

fn is_key_line(line: &str, key: &str) -> bool {
    line.trim_start()
        .strip_prefix(key)
        .is_some_and(|rest| rest.trim_start().starts_with('='))
}

fn key_line(text: &str, key: &str) -> Option<usize> {
    text.lines().position(|l| is_key_line(l, key)).map(|i| i + 1)
}
