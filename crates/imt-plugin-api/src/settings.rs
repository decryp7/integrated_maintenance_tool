//! Typed settings: the schema a settings screen is generated from, the validation of untrusted values
//! against it, and [`Configurable`], through which plugins (and the app's own modules) expose knobs.
//!
//! A [`SettingsSection`] lists [`SettingField`]s, each with a [`FieldKind`] (bool, ranged int/float, text,
//! enum, path, duration, size, string list) and a default. [`validate`] turns a JSON object from the UI or a
//! settings file into [`ValidValues`] (parse, don't validate): unknown keys are dropped, missing keys take
//! their default, and every type and range problem is reported per field with a message the user can act on.
//! A plugin's section crosses the process boundary in its manifest; the host validates values before it sends
//! them and the plugin validates them again.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashSet};
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SectionOwner {
    /// The engine itself.
    Core,
    /// The desktop shell (diagnostics, workspace, plugin management).
    App,
    /// An engine plugin, by plugin id.
    Plugin {
        /// Plugin id.
        id: String,
    },
}

/// One choice of an [`FieldKind::Enum`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnumOption {
    /// The stored value.
    pub value: String,
    /// Text shown to the user.
    pub label: String,
}

impl EnumOption {
    /// Creates an option with the stored `value` and its `label`.
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        EnumOption {
            value: value.into(),
            label: label.into(),
        }
    }
}

/// The type of a field and its limits. Durations are stored as milliseconds and sizes as bytes, so values
/// are plain JSON integers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum FieldKind {
    /// On or off.
    Bool,
    /// Whole number in `min..=max`, reachable from `min` in `step`s.
    Int {
        /// Smallest accepted value.
        min: i64,
        /// Largest accepted value.
        max: i64,
        /// Granularity of the control; at least 1.
        step: i64,
        /// Unit label shown beside the value (e.g. `"lines"`).
        unit: Option<String>,
    },
    /// Finite number in `min..=max`; `step` is a hint for the UI only.
    Float {
        /// Smallest accepted value.
        min: f64,
        /// Largest accepted value.
        max: f64,
        /// UI step hint; greater than 0.
        step: f64,
        /// Unit label shown beside the value.
        unit: Option<String>,
    },
    /// Up to `max_len` characters, fully matching `pattern` when given.
    Text {
        /// Longest accepted text, in characters (1 to the text limit).
        max_len: usize,
        /// Regular expression the whole text must match.
        pattern: Option<String>,
    },
    /// One of a fixed set of choices.
    Enum {
        /// The choices; at least one, with unique values.
        options: Vec<EnumOption>,
    },
    /// An absolute path (or empty for "not set"); `dir` picks a folder dialog.
    Path {
        /// Whether to pick a folder rather than a file.
        dir: bool,
    },
    /// A time span, stored as milliseconds.
    Duration {
        /// Shortest accepted span, in milliseconds.
        min_ms: u64,
        /// Longest accepted span, in milliseconds.
        max_ms: u64,
    },
    /// A size, stored as bytes.
    Size {
        /// Smallest accepted size, in bytes.
        min_bytes: u64,
        /// Largest accepted size, in bytes.
        max_bytes: u64,
    },
    /// A list of text items.
    StringList {
        /// Most items accepted.
        max_items: usize,
    },
}

/// One knob.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingField {
    /// Stable key within the section (`[A-Za-z0-9._-]`).
    pub key: String,
    /// Name shown to the user.
    pub label: String,
    /// Help text shown under the control.
    pub description: String,
    /// The field's type and limits.
    pub kind: FieldKind,
    /// The value used until the user changes it; must pass the field's own validation.
    pub default: Value,
    /// A change only takes effect after IMT restarts.
    pub restart_required: bool,
    /// Hidden unless the user shows advanced settings.
    pub advanced: bool,
}

impl SettingField {
    /// Creates a field with a `key`, `label`, type and default; other properties take their defaults.
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

    /// Sets the help text.
    pub fn describe(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Marks the field as taking effect only after IMT restarts.
    pub fn needs_restart(mut self) -> Self {
        self.restart_required = true;
        self
    }

    /// Hides the field unless the user shows advanced settings.
    pub fn as_advanced(mut self) -> Self {
        self.advanced = true;
        self
    }
}

/// A group of fields shown together (one entry of the settings navigation).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSection {
    /// Stable id; also the key in `settings.json`. Plugin sections use the plugin id.
    pub id: String,
    /// Title shown in the settings navigation.
    pub title: String,
    /// Help text shown under the title.
    pub description: String,
    /// Who provides the section.
    pub owner: SectionOwner,
    /// The fields, in display order.
    pub fields: Vec<SettingField>,
}

impl SettingsSection {
    /// Creates an empty section.
    pub fn new(id: impl Into<String>, title: impl Into<String>, owner: SectionOwner) -> Self {
        SettingsSection {
            id: id.into(),
            title: title.into(),
            description: String::new(),
            owner,
            fields: Vec::new(),
        }
    }

    /// Sets the description.
    pub fn describe(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Appends a field.
    pub fn field(mut self, field: SettingField) -> Self {
        self.fields.push(field);
        self
    }

    /// The field with `key`, if any.
    pub fn find(&self, key: &str) -> Option<&SettingField> {
        self.fields.iter().find(|f| f.key == key)
    }

    /// Every field at its default.
    pub fn defaults(&self) -> ValidValues {
        ValidValues(self.fields.iter().map(|f| (f.key.clone(), f.default.clone())).collect())
    }

    /// Checks the schema itself: ids and keys, consistent limits, compilable patterns and defaults that pass
    /// their own validation.
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

/// Values that passed [`validate`]: every field of the section is present with a value of its kind and within
/// its limits.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ValidValues(BTreeMap<String, Value>);

impl ValidValues {
    /// The raw value of `key`.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }

    /// A boolean value.
    pub fn bool(&self, key: &str) -> Option<bool> {
        self.get(key)?.as_bool()
    }

    /// An integer value.
    pub fn int(&self, key: &str) -> Option<i64> {
        self.get(key)?.as_i64()
    }

    /// A floating-point value.
    pub fn float(&self, key: &str) -> Option<f64> {
        self.get(key)?.as_f64()
    }

    /// Text, enum and path values.
    pub fn text(&self, key: &str) -> Option<&str> {
        self.get(key)?.as_str()
    }

    /// A duration value (stored as milliseconds).
    pub fn duration(&self, key: &str) -> Option<Duration> {
        self.get(key)?.as_u64().map(Duration::from_millis)
    }

    /// Size values, in bytes.
    pub fn bytes(&self, key: &str) -> Option<u64> {
        self.get(key)?.as_u64()
    }

    /// A list-of-text value.
    pub fn strings(&self, key: &str) -> Option<Vec<String>> {
        self.get(key)?.as_array()?.iter().map(|v| v.as_str().map(String::from)).collect()
    }

    /// Every key and value, ordered by key.
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
    /// The field the problem is about; empty for the whole object.
    pub key: String,
    /// What is wrong, to show the user.
    pub message: String,
}

/// Validates `values` (a JSON object) against `section`. Unknown keys are dropped, missing or `null` keys take
/// their default; any invalid field rejects the whole set with one error per field.
pub fn validate(section: &SettingsSection, values: &Value) -> Result<ValidValues, Vec<FieldError>> {
    let (valid, errors) = sanitize(section, values);
    if errors.is_empty() {
        Ok(valid)
    } else {
        Err(errors)
    }
}

/// Like [`validate`], but replaces each invalid field by its default and returns the problems alongside (for
/// loading stored values).
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

/// Something with user-facing settings: a plugin or an app module.
pub trait Configurable: Send + Sync {
    /// The schema. Should be cheap and deterministic.
    fn settings_section(&self) -> SettingsSection;
    /// Puts validated values into effect. Called at startup with the stored values and after every change.
    /// Must not panic; return an error instead.
    fn apply_settings(&self, values: &ValidValues) -> Result<()>;
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
    fn sections_cross_the_boundary_as_json() {
        let s = SettingsSection::new("p", "P", SectionOwner::Plugin { id: "p".into() })
            .describe("d")
            .field(int_field().describe("x").needs_restart().as_advanced())
            .field(SettingField::new(
                "e",
                "E",
                FieldKind::Enum {
                    options: vec![EnumOption::new("a", "A")],
                },
                "a",
            ));
        s.check().unwrap();
        let back: SettingsSection = serde_json::from_value(serde_json::to_value(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        assert_eq!(ValidValues::default().iter().count(), 0);
    }
}
