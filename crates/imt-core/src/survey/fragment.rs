//! Turning [`Fragment`]s into profile TOML: a snippet per proposal (for review) and the merge of a selection
//! into the user's profile. The merge works on the TOML tables, so sections this engine version does not
//! know are kept, and a list the user never wrote (e.g. `[[marker]]`) starts from the built-in defaults so
//! adding one entry does not silently drop the others.

use super::dto::Fragment;
use crate::profile::DEFAULT_PROFILE_TOML;
use anyhow::{anyhow, bail, Context, Result};
use toml::{Table, Value};

fn to_value<T: serde::Serialize>(v: &T) -> Result<Value> {
    let mut v = Value::try_from(v).context("cannot write the change as TOML")?;
    prune(&mut v);
    Ok(v)
}

/// Drops keys that only repeat defaults (empty lists and tables, `disabled = false`, `lifecycle = false`).
fn prune(v: &mut Value) {
    match v {
        Value::Table(t) => {
            t.retain(|k, v| {
                prune(v);
                match v {
                    Value::Array(a) => !a.is_empty(),
                    Value::Table(t) => !t.is_empty(),
                    Value::Boolean(false) => k != "disabled" && k != "lifecycle",
                    _ => true,
                }
            });
        }
        Value::Array(a) => a.iter_mut().for_each(prune),
        _ => {}
    }
}

/// (list key, entry, identity key) of a list fragment.
fn list_entry(f: &Fragment) -> Result<Option<(&'static str, Value, Option<String>)>> {
    Ok(match f {
        Fragment::Extractor { extractor } => Some(("extract", to_value(extractor)?, Some(extractor.name.clone()))),
        Fragment::Marker { marker } => Some(("marker", to_value(marker)?, Some(marker.kind.clone()))),
        Fragment::Span { span } => Some(("span", to_value(span)?, Some(span.kind.clone()))),
        Fragment::Pipeline { step } => Some(("pipeline", to_value(step)?, None)),
        Fragment::Chart { chart } => Some(("chart", to_value(chart)?, chart.id.clone())),
        Fragment::Dashboard { dashboard } => Some(("dashboard", to_value(dashboard)?, Some(dashboard.id.clone()))),
        _ => None,
    })
}

fn identity_key(list: &str) -> &'static str {
    match list {
        "extract" => "name",
        "marker" | "span" => "kind",
        _ => "id",
    }
}

/// The fragments as a TOML snippet (what applying them adds).
pub fn fragment_toml(fragments: &[Fragment]) -> Result<String> {
    let mut table = Table::new();
    let mut comments = String::new();
    for f in fragments {
        match f {
            Fragment::ParseRegex { .. } => {}
            Fragment::InstanceKey { key } => {
                section(&mut table, "instance")?.insert("key".into(), to_value(key)?);
            }
            Fragment::SavedQuery { name, ql } => comments.push_str(&format!("# Saved query \"{name}\": {ql}\n")),
            other => {
                if let Some((list, v, _)) = list_entry(other)? {
                    push(&mut table, list, v)?;
                }
            }
        }
    }
    let body = if table.is_empty() {
        String::new()
    } else {
        toml::to_string(&table)?
    };
    let mut parsers = String::new();
    for p in parser_drafts(fragments)? {
        parsers.push_str("# A new parser for the library (Settings ▸ Parsers):\n");
        parsers.push_str(&p);
    }
    Ok(format!("{comments}{body}{parsers}"))
}

/// The new parsers of the `ParseRegex` fragments (their TOML): one per file name, with its regexes in
/// order (ADR 0026: formats are parsers, not profile sections).
pub fn parser_drafts(fragments: &[Fragment]) -> Result<Vec<String>> {
    let mut by_file: Vec<(String, Vec<&str>)> = Vec::new();
    for f in fragments {
        if let Fragment::ParseRegex { regex, file_name } = f {
            match by_file.iter_mut().find(|(n, _)| n == file_name) {
                Some((_, list)) => list.push(regex),
                None => by_file.push((file_name.clone(), vec![regex])),
            }
        }
    }
    let mut out = Vec::new();
    for (file_name, regexes) in by_file {
        let mut parser: Option<crate::parser::Parser> = None;
        // The first regex of the list is tried first: add them last to first.
        for regex in regexes.iter().rev() {
            let req = crate::parser::draft::DraftRequest {
                file_name: file_name.clone(),
                regex: Some(regex.to_string()),
                ..Default::default()
            };
            let text = crate::parser::draft::compose(&req, parser.as_ref()).map_err(|e| anyhow!("the new parser is not valid: {e}"))?;
            let mut p = crate::parser::Parser::from_toml(&text).map_err(|e| anyhow!("the new parser is not valid: {e}"))?;
            p.version = crate::parser::ParserVersion::new(1, 0, 0);
            parser = Some(p);
        }
        if let Some(p) = parser {
            out.push(p.canonical_toml());
        }
    }
    Ok(out)
}

fn section<'a>(t: &'a mut Table, key: &str) -> Result<&'a mut Table> {
    t.entry(key.to_string())
        .or_insert_with(|| Value::Table(Table::new()))
        .as_table_mut()
        .ok_or_else(|| anyhow!("[{key}] is not a table in the profile"))
}

fn push(t: &mut Table, key: &str, v: Value) -> Result<()> {
    t.entry(key.to_string())
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| anyhow!("`{key}` is not a list in the profile"))?
        .push(v);
    Ok(())
}

/// The user's list `key`, or the built-in default list when the profile does not have one, so adding one
/// entry keeps the defaults (as the markers manager and the Runs editor do).
fn list<'a>(base: &'a mut Table, defaults: &Table, key: &str) -> Result<&'a mut Vec<Value>> {
    if !base.contains_key(key) {
        let seed = defaults.get(key).cloned();
        base.insert(key.to_string(), seed.unwrap_or_else(|| Value::Array(Vec::new())));
    }
    base.get_mut(key)
        .and_then(Value::as_array_mut)
        .ok_or_else(|| anyhow!("`{key}` is not a list in the profile"))
}

fn has_identity(list: &[Value], key: &str, id: &str) -> bool {
    list.iter().any(|v| v.get(key).and_then(Value::as_str) == Some(id))
}

/// Merges `fragments` into `base_toml`; returns (base, merged) both written the same way (comments are not
/// kept, as in the profile editor), so their diff shows only the change.
pub fn compose_toml(base_toml: &str, fragments: &[Fragment]) -> Result<(String, String)> {
    let base: Table = toml::from_str(base_toml).map_err(|e| anyhow!("the current profile is not valid TOML: {}", e.message()))?;
    let defaults: Table = toml::from_str(DEFAULT_PROFILE_TOML).context("the built-in profile")?;
    let before = toml::to_string(&base)?;
    let mut merged = base;
    let mut new_markers: Vec<Value> = Vec::new();
    for f in fragments {
        match f {
            // Formats are parsers ([`parser_drafts`]): the profile does not change.
            Fragment::ParseRegex { .. } => {}
            Fragment::InstanceKey { key } => {
                if key.is_empty() || key.len() > 8 || key.iter().any(|k| k.trim().is_empty()) {
                    bail!("an instance key needs 1 to 8 field names");
                }
                section(&mut merged, "instance")?.insert("key".into(), to_value(key)?);
            }
            Fragment::SavedQuery { .. } => {}
            other => {
                let Some((name, value, id)) = list_entry(other)? else { continue };
                let existing = list(&mut merged, &defaults, name)?;
                if let (Some(id), "span") = (&id, name) {
                    // Like the Runs editor: a rule of the same kind is replaced in place, else added last.
                    match existing.iter().position(|v| v.get("kind").and_then(Value::as_str) == Some(id)) {
                        Some(i) => existing[i] = value,
                        None => existing.push(value),
                    }
                    continue;
                }
                if let Some(id) = &id {
                    let key = identity_key(name);
                    if has_identity(existing, key, id) || (name == "marker" && has_identity(&new_markers, key, id)) {
                        bail!("the profile already has a {name} named \"{id}\"; deselect one of them");
                    }
                }
                if name == "marker" {
                    // New, specific rules go first: the first matching rule is an event's primary marker.
                    new_markers.push(value);
                } else {
                    existing.push(value);
                }
            }
        }
    }
    if !new_markers.is_empty() {
        let markers = list(&mut merged, &defaults, "marker")?;
        let rest = std::mem::take(markers);
        *markers = new_markers.into_iter().chain(rest).collect();
    }
    let after = toml::to_string(&merged)?;
    Ok((before, after))
}
