//! Typed settings: the schema a settings screen is generated from, the
//! validation of untrusted values against it, and the [`Configurable`]
//! extension point through which plugins and app modules expose knobs.
//!
//! A [`SettingsSection`] lists [`SettingField`]s, each with a [`FieldKind`]
//! (bool, ranged int/float, text, enum, path, duration, size, string list) and
//! a default. [`validate`] turns a JSON object from the UI or a settings file
//! into [`ValidValues`] (parse, don't validate): unknown keys are dropped,
//! missing keys take their default, and every type and range problem is
//! reported per field with a message the user can act on.
//!
//! Plugins opt in with [`crate::Plugin::configurable`]. The registry wraps the
//! returned object in a [`GuardedConfigurable`]: the section is described once
//! at install (so a disabled or quarantined plugin still shows it, read-only)
//! and `apply_settings` runs under crash containment like every other plugin
//! contribution.

use crate::guard::Guard;
use crate::plugin::{PluginInfo, PluginStatus};
use anyhow::{anyhow, Result};
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

/// Longest text value (and `Text::max_len`) accepted, in characters.
pub const MAX_TEXT_LEN: usize = 64 * 1024;
/// Most items a `StringList` may allow.
pub const MAX_LIST_ITEMS: usize = 1000;
/// Longest single item of a `StringList`, in characters.
pub const MAX_ITEM_LEN: usize = 4096;
/// Longest path accepted, in characters.
pub const MAX_PATH_LEN: usize = 4096;
/// Most fields one section may declare.
pub const MAX_FIELDS: usize = 200;
const MAX_ID_LEN: usize = 128;

/// Who contributes a section; decides where it is listed in the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SectionOwner {
    /// The engine itself.
    Core,
    /// The desktop shell (diagnostics, workspace, plugin management).
    App,
    /// An engine plugin, by plugin id.
    Plugin { id: String },
}

/// One choice of an [`FieldKind::Enum`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EnumOption {
    pub value: String,
    pub label: String,
}

impl EnumOption {
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        EnumOption {
            value: value.into(),
            label: label.into(),
        }
    }
}

/// The type of a field and its limits. Durations are stored as milliseconds
/// and sizes as bytes, so values are plain JSON integers.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum FieldKind {
    Bool,
    /// Whole number in `min..=max`, reachable from `min` in `step`s.
    Int {
        min: i64,
        max: i64,
        step: i64,
        unit: Option<String>,
    },
    /// Finite number in `min..=max`; `step` is a hint for the UI only.
    Float {
        min: f64,
        max: f64,
        step: f64,
        unit: Option<String>,
    },
    /// Up to `max_len` characters, fully matching `pattern` when given.
    Text {
        max_len: usize,
        pattern: Option<String>,
    },
    Enum {
        options: Vec<EnumOption>,
    },
    /// An absolute path (or empty for "not set"); `dir` picks a folder dialog.
    Path {
        dir: bool,
    },
    Duration {
        min_ms: u64,
        max_ms: u64,
    },
    Size {
        min_bytes: u64,
        max_bytes: u64,
    },
    StringList {
        max_items: usize,
    },
}

/// One knob.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingField {
    /// Stable key within the section (`[A-Za-z0-9._-]`).
    pub key: String,
    pub label: String,
    /// Help text shown under the control.
    pub description: String,
    pub kind: FieldKind,
    pub default: Value,
    /// A change only takes effect after IMT restarts.
    pub restart_required: bool,
    /// Hidden unless the user shows advanced settings.
    pub advanced: bool,
}

impl SettingField {
    pub fn new(key: impl Into<String>, label: impl Into<String>, kind: FieldKind, default: impl Into<Value>) -> Self {
        SettingField {
            key: key.into(),
            label: label.into(),
            description: String::new(),
            kind,
            default: default.into(),
            restart_required: false,
            advanced: false,
        }
    }

    pub fn describe(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn needs_restart(mut self) -> Self {
        self.restart_required = true;
        self
    }

    pub fn as_advanced(mut self) -> Self {
        self.advanced = true;
        self
    }
}

/// A group of fields shown together (one entry of the settings navigation).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSection {
    /// Stable id; also the key in `settings.json`. Plugin sections use the plugin id.
    pub id: String,
    pub title: String,
    pub description: String,
    pub owner: SectionOwner,
    pub fields: Vec<SettingField>,
}

impl SettingsSection {
    pub fn new(id: impl Into<String>, title: impl Into<String>, owner: SectionOwner) -> Self {
        SettingsSection {
            id: id.into(),
            title: title.into(),
            description: String::new(),
            owner,
            fields: Vec::new(),
        }
    }

    pub fn describe(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn field(mut self, field: SettingField) -> Self {
        self.fields.push(field);
        self
    }

    pub fn find(&self, key: &str) -> Option<&SettingField> {
        self.fields.iter().find(|f| f.key == key)
    }

    /// Every field at its default.
    pub fn defaults(&self) -> ValidValues {
        ValidValues(self.fields.iter().map(|f| (f.key.clone(), f.default.clone())).collect())
    }

    /// Checks the schema itself: ids and keys, consistent limits, compilable
    /// patterns and defaults that pass their own validation.
    pub fn check(&self) -> Result<(), String> {
        check_id("section id", &self.id)?;
        if self.fields.len() > MAX_FIELDS {
            return Err(format!(
                "section \"{}\" has {} fields; the limit is {MAX_FIELDS}",
                self.id,
                self.fields.len()
            ));
        }
        let mut seen = HashSet::new();
        for f in &self.fields {
            check_id("field key", &f.key)?;
            if !seen.insert(f.key.as_str()) {
                return Err(format!("field \"{}\" is declared twice", f.key));
            }
            check_kind(&f.kind).map_err(|e| format!("field \"{}\": {e}", f.key))?;
            validate_field(f, &f.default).map_err(|e| format!("field \"{}\": default {}: {e}", f.key, f.default))?;
        }
        Ok(())
    }
}

fn check_id(what: &str, id: &str) -> Result<(), String> {
    let ok = !id.is_empty() && id.len() <= MAX_ID_LEN && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if ok {
        Ok(())
    } else {
        Err(format!(
            "{what} \"{id}\" must be 1-{MAX_ID_LEN} characters of A-Z, a-z, 0-9, '.', '_' or '-'"
        ))
    }
}

fn check_kind(kind: &FieldKind) -> Result<(), String> {
    match kind {
        FieldKind::Int { min, max, step, .. } if min > max || *step < 1 => Err("needs min <= max and step >= 1".into()),
        FieldKind::Float { min, max, step, .. }
            if !(min.is_finite() && max.is_finite() && step.is_finite()) || min > max || *step <= 0.0 =>
        {
            Err("needs finite min <= max and step > 0".into())
        }
        FieldKind::Text { max_len, .. } if *max_len == 0 || *max_len > MAX_TEXT_LEN => Err(format!("max_len must be 1-{MAX_TEXT_LEN}")),
        FieldKind::Text { pattern: Some(p), .. } => anchored(p).map(|_| ()).map_err(|e| format!("pattern: {e:#}")),
        FieldKind::Enum { options } => {
            let mut seen = HashSet::new();
            if options.is_empty() || !options.iter().all(|o| seen.insert(o.value.as_str())) {
                Err("needs at least one option and unique values".into())
            } else {
                Ok(())
            }
        }
        FieldKind::Duration { min_ms, max_ms } if min_ms > max_ms => Err("needs min_ms <= max_ms".into()),
        FieldKind::Size { min_bytes, max_bytes } if min_bytes > max_bytes => Err("needs min_bytes <= max_bytes".into()),
        FieldKind::StringList { max_items } if *max_items == 0 || *max_items > MAX_LIST_ITEMS => {
            Err(format!("max_items must be 1-{MAX_LIST_ITEMS}"))
        }
        _ => Ok(()),
    }
}

fn anchored(pattern: &str) -> Result<regex::Regex> {
    crate::user_regex::compile(&format!("^(?:{pattern})$"), false)
}

/// Values that passed [`validate`]: every field of the section is present
/// with a value of its kind and within its limits.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ValidValues(BTreeMap<String, Value>);

impl ValidValues {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }

    pub fn bool(&self, key: &str) -> Option<bool> {
        self.get(key)?.as_bool()
    }

    pub fn int(&self, key: &str) -> Option<i64> {
        self.get(key)?.as_i64()
    }

    pub fn float(&self, key: &str) -> Option<f64> {
        self.get(key)?.as_f64()
    }

    /// Text, enum and path values.
    pub fn text(&self, key: &str) -> Option<&str> {
        self.get(key)?.as_str()
    }

    pub fn duration(&self, key: &str) -> Option<Duration> {
        self.get(key)?.as_u64().map(Duration::from_millis)
    }

    /// Size values, in bytes.
    pub fn bytes(&self, key: &str) -> Option<u64> {
        self.get(key)?.as_u64()
    }

    pub fn strings(&self, key: &str) -> Option<Vec<String>> {
        self.get(key)?.as_array()?.iter().map(|v| v.as_str().map(String::from)).collect()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &Value)> {
        self.0.iter()
    }

    /// The values as a JSON object.
    pub fn to_json(&self) -> Value {
        Value::Object(self.0.iter().map(|(k, v)| (k.clone(), v.clone())).collect::<Map<_, _>>())
    }
}

/// Why a value was rejected; `key` is empty for problems with the whole object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldError {
    pub key: String,
    pub message: String,
}

/// Validates `values` (a JSON object) against `section`. Unknown keys are
/// dropped, missing or `null` keys take their default; any invalid field
/// rejects the whole set with one error per field.
pub fn validate(section: &SettingsSection, values: &Value) -> Result<ValidValues, Vec<FieldError>> {
    let (valid, errors) = sanitize(section, values);
    if errors.is_empty() {
        Ok(valid)
    } else {
        Err(errors)
    }
}

/// Like [`validate`], but replaces each invalid field by its default and
/// returns the problems alongside (for loading stored values).
pub fn sanitize(section: &SettingsSection, values: &Value) -> (ValidValues, Vec<FieldError>) {
    let empty = Map::new();
    let mut errors = Vec::new();
    let object = match values {
        Value::Object(o) => o,
        Value::Null => &empty,
        _ => {
            errors.push(FieldError {
                key: String::new(),
                message: "Settings must be an object of key/value pairs.".into(),
            });
            &empty
        }
    };
    let mut out = BTreeMap::new();
    for field in &section.fields {
        let value = match object.get(&field.key) {
            None | Some(Value::Null) => field.default.clone(),
            Some(v) => validate_field(field, v).unwrap_or_else(|message| {
                errors.push(FieldError {
                    key: field.key.clone(),
                    message,
                });
                field.default.clone()
            }),
        };
        out.insert(field.key.clone(), value);
    }
    (ValidValues(out), errors)
}

/// Checks one value, returning it normalised (e.g. `5.0` becomes `5` for an int).
pub fn validate_field(field: &SettingField, value: &Value) -> Result<Value, String> {
    match &field.kind {
        FieldKind::Bool => value
            .as_bool()
            .map(Value::from)
            .ok_or_else(|| "Must be on or off (true or false).".into()),
        FieldKind::Int { min, max, step, unit } => {
            let n = whole(value).ok_or("Must be a whole number.")?;
            if n < *min || n > *max {
                return Err(format!("Must be between {} and {}.", with_unit(*min, unit), with_unit(*max, unit)));
            }
            if (n - min) % step != 0 {
                return Err(format!("Must be {min} plus a multiple of {step}."));
            }
            Ok(Value::from(n))
        }
        FieldKind::Float { min, max, unit, .. } => {
            let n = value.as_f64().filter(|n| n.is_finite()).ok_or("Must be a number.")?;
            if n < *min || n > *max {
                return Err(format!("Must be between {} and {}.", with_unit(*min, unit), with_unit(*max, unit)));
            }
            Ok(Value::from(n))
        }
        FieldKind::Text { max_len, pattern } => {
            let s = value.as_str().ok_or("Must be text.")?;
            if s.chars().count() > *max_len {
                return Err(format!("Must be at most {max_len} characters."));
            }
            if let Some(p) = pattern {
                let re = anchored(p).map_err(|e| format!("The setting's pattern is invalid: {e:#}"))?;
                if !re.is_match(s) {
                    return Err(format!("Does not have the expected format ({p})."));
                }
            }
            Ok(Value::from(s))
        }
        FieldKind::Enum { options } => {
            let s = value.as_str().filter(|s| options.iter().any(|o| o.value == *s));
            s.map(Value::from).ok_or_else(|| {
                let names: Vec<&str> = options.iter().map(|o| o.value.as_str()).collect();
                format!("Must be one of: {}.", names.join(", "))
            })
        }
        FieldKind::Path { .. } => {
            let s = value.as_str().ok_or("Must be a path.")?;
            if s.chars().count() > MAX_PATH_LEN {
                return Err(format!("Must be at most {MAX_PATH_LEN} characters."));
            }
            if s.contains('\0') {
                return Err("Must not contain NUL characters.".into());
            }
            if !s.is_empty() && !std::path::Path::new(s).is_absolute() {
                return Err("Must be an absolute path (or empty).".into());
            }
            Ok(Value::from(s))
        }
        FieldKind::Duration { min_ms, max_ms } => {
            let n = whole(value)
                .filter(|n| *n >= 0)
                .ok_or("Must be a duration in whole milliseconds.")?
                .unsigned_abs();
            if n < *min_ms || n > *max_ms {
                return Err(format!("Must be between {} and {}.", format_ms(*min_ms), format_ms(*max_ms)));
            }
            Ok(Value::from(n))
        }
        FieldKind::Size { min_bytes, max_bytes } => {
            let n = whole(value)
                .filter(|n| *n >= 0)
                .ok_or("Must be a size in whole bytes.")?
                .unsigned_abs();
            if n < *min_bytes || n > *max_bytes {
                return Err(format!(
                    "Must be between {} and {}.",
                    format_bytes(*min_bytes),
                    format_bytes(*max_bytes)
                ));
            }
            Ok(Value::from(n))
        }
        FieldKind::StringList { max_items } => {
            let items = value.as_array().ok_or("Must be a list of text.")?;
            if items.len() > *max_items {
                return Err(format!("Must have at most {max_items} items."));
            }
            let mut out = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                let s = item.as_str().ok_or_else(|| format!("Item {} must be text.", i + 1))?;
                if s.chars().count() > MAX_ITEM_LEN {
                    return Err(format!("Item {} must be at most {MAX_ITEM_LEN} characters.", i + 1));
                }
                out.push(Value::from(s));
            }
            Ok(Value::Array(out))
        }
    }
}

/// An integer, also from a float without a fractional part (JavaScript numbers).
fn whole(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_f64().filter(|f| f.fract() == 0.0 && f.abs() < 9.0e15).map(|f| f as i64))
}

fn with_unit<T: std::fmt::Display>(n: T, unit: &Option<String>) -> String {
    match unit {
        Some(u) if !u.is_empty() => format!("{n} {u}"),
        _ => n.to_string(),
    }
}

/// `90000` → `"90 s"`, `3600000` → `"1 h"`.
pub fn format_ms(ms: u64) -> String {
    const UNITS: [(u64, &str); 3] = [(3_600_000, "h"), (60_000, "min"), (1000, "s")];
    UNITS
        .iter()
        .find(|(size, _)| ms >= *size && ms.checked_rem(*size) == Some(0))
        .map_or_else(|| format!("{ms} ms"), |(size, name)| format!("{} {name}", ms / size))
}

/// `10485760` → `"10 MB"` (binary units, as the rest of IMT).
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [(u64, &str); 3] = [(1 << 30, "GB"), (1 << 20, "MB"), (1 << 10, "KB")];
    UNITS
        .iter()
        .find(|(size, _)| bytes >= *size && bytes.checked_rem(*size) == Some(0))
        .map_or_else(|| format!("{bytes} bytes"), |(size, name)| format!("{} {name}", bytes / size))
}

/// Something with user-facing settings: a plugin (see
/// [`crate::Plugin::configurable`]) or an app module.
pub trait Configurable: Send + Sync {
    /// The schema. Should be cheap and deterministic.
    fn settings_section(&self) -> SettingsSection;
    /// Puts validated values into effect. Called at startup with the stored
    /// values and after every change. Must not panic; return an error instead.
    fn apply_settings(&self, values: &ValidValues) -> Result<()>;
}

/// A plugin's [`Configurable`] under crash containment. The section is
/// described once, when the plugin is installed, so it can still be shown
/// (read-only) when the plugin is disabled or quarantined without calling
/// plugin code again. Its id and owner are forced to the plugin's.
pub struct GuardedConfigurable {
    inner: Arc<dyn Configurable>,
    guard: Guard,
    section: SettingsSection,
}

impl GuardedConfigurable {
    /// Asks `plugin` for its settings, if it has any. Panics and invalid
    /// schemas are contained: the plugin gets a section without fields that
    /// says why, and a panic counts towards its quarantine.
    pub fn describe(plugin: &dyn crate::Plugin, guard: Guard) -> Option<Arc<GuardedConfigurable>> {
        let described = guard
            .run("configurable", || Ok(plugin.configurable()))
            .and_then(|inner| match inner {
                None => Ok(None),
                Some(inner) => guard.run("settings_section", || {
                    let s = inner.settings_section();
                    s.check().map_err(|e| anyhow!("invalid settings schema: {e}"))?;
                    Ok(Some((inner, s)))
                }),
            });
        let (inner, section) = match described {
            Ok(None) => return None,
            Ok(Some((inner, mut s))) => {
                s.id = plugin.id().to_string();
                s.owner = SectionOwner::Plugin { id: plugin.id().into() };
                if s.title.trim().is_empty() {
                    s.title = plugin.name().into();
                }
                (inner, s)
            }
            // Values validated against an unknown schema mean nothing to the plugin: never apply them.
            Err(e) => (Arc::new(Unavailable) as Arc<dyn Configurable>, unavailable(plugin, &e)),
        };
        Some(Arc::new(GuardedConfigurable { inner, guard, section }))
    }
}

impl Configurable for GuardedConfigurable {
    fn settings_section(&self) -> SettingsSection {
        self.section.clone()
    }

    fn apply_settings(&self, values: &ValidValues) -> Result<()> {
        self.guard.run("apply_settings", || self.inner.apply_settings(values))
    }
}

/// Stands in for a plugin whose settings could not be described.
struct Unavailable;

impl Configurable for Unavailable {
    fn settings_section(&self) -> SettingsSection {
        SettingsSection::new("unavailable", "Unavailable", SectionOwner::Core)
    }
    fn apply_settings(&self, _values: &ValidValues) -> Result<()> {
        Ok(())
    }
}

fn unavailable(plugin: &dyn crate::Plugin, error: &anyhow::Error) -> SettingsSection {
    SettingsSection::new(plugin.id(), plugin.name(), SectionOwner::Plugin { id: plugin.id().into() })
        .describe(format!("Settings are unavailable: {error:#}"))
}

/// A plugin's settings with its current status, as listed by
/// [`crate::PluginRegistry::plugin_settings`].
#[derive(Clone)]
pub struct PluginSettings {
    pub plugin: PluginInfo,
    pub configurable: Arc<dyn Configurable>,
}

impl PluginSettings {
    /// Only an active plugin's settings can be changed.
    pub fn read_only(&self) -> bool {
        self.plugin.status != PluginStatus::Active
    }

    /// Why the section is read-only.
    pub fn note(&self) -> Option<&'static str> {
        match self.plugin.status {
            PluginStatus::Active => None,
            PluginStatus::Disabled => Some("This plugin is disabled. Enable it under Plugins and restart to change its settings."),
            PluginStatus::Quarantined => {
                Some("This plugin was stopped after repeated crashes. Re-enable it in the Plugins panel to change its settings.")
            }
            PluginStatus::Inactive => {
                Some("This plugin is inactive (see the Plugins panel for why). Its settings can be changed once it runs.")
            }
        }
    }

    /// Applies values to an active plugin; refuses for a read-only one.
    pub fn apply(&self, values: &ValidValues) -> Result<()> {
        match self.note() {
            Some(note) => Err(anyhow!("{note}")),
            None => self.configurable.apply_settings(values),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn int_field() -> SettingField {
        SettingField::new(
            "n",
            "N",
            FieldKind::Int {
                min: 0,
                max: 10,
                step: 2,
                unit: Some("s".into()),
            },
            4,
        )
    }

    #[test]
    fn formats_units() {
        assert_eq!(format_ms(0), "0 ms");
        assert_eq!(format_ms(1500), "1500 ms");
        assert_eq!(format_ms(90_000), "90 s");
        assert_eq!(format_ms(120_000), "2 min");
        assert_eq!(format_ms(7_200_000), "2 h");
        assert_eq!(format_bytes(0), "0 bytes");
        assert_eq!(format_bytes(1000), "1000 bytes");
        assert_eq!(format_bytes(2048), "2 KB");
        assert_eq!(format_bytes(10 << 20), "10 MB");
        assert_eq!(format_bytes(3 << 30), "3 GB");
        assert_eq!(with_unit(3, &Some(String::new())), "3");
    }

    #[test]
    fn whole_numbers_accept_integral_floats() {
        assert_eq!(whole(&json!(5)), Some(5));
        assert_eq!(whole(&json!(5.0)), Some(5));
        assert_eq!(whole(&json!(5.5)), None);
        assert_eq!(whole(&json!(1e300)), None);
        assert_eq!(whole(&json!("5")), None);
        assert_eq!(validate_field(&int_field(), &json!(6.0)), Ok(json!(6)));
    }

    #[test]
    fn unavailable_stand_in_is_inert() {
        assert_eq!(Unavailable.settings_section().id, "unavailable");
        assert!(Unavailable.apply_settings(&ValidValues(BTreeMap::new())).is_ok());
    }
}
