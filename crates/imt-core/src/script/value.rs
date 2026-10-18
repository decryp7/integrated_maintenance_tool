//! Conversion between record field values (JSON) and script values (Rhai `Dynamic`).

use crate::model::Fields;
use rhai::{Array, Dynamic, Map};
use serde_json::{Number, Value};

/// Deepest nesting of arrays and maps converted (the depth `serde_json` parses at most);
/// deeper values become null instead of recursing further.
pub(crate) const MAX_DEPTH: usize = 128;

/// A JSON value as a script value: null -> `()`, integers -> `i64`, other numbers -> `f64`,
/// arrays -> arrays, objects -> maps.
pub(crate) fn to_dynamic(v: &Value, depth: usize) -> Dynamic {
    match v {
        Value::Null => Dynamic::UNIT,
        Value::Bool(b) => Dynamic::from_bool(*b),
        Value::Number(n) => match n.as_i64() {
            Some(i) => Dynamic::from_int(i),
            // Integers beyond i64 (and every fraction) are floats in scripts.
            None => n.as_f64().map_or(Dynamic::UNIT, Dynamic::from_float),
        },
        Value::String(s) => Dynamic::from(s.clone()),
        _ if depth >= MAX_DEPTH => Dynamic::UNIT,
        Value::Array(a) => Dynamic::from_array(a.iter().map(|x| to_dynamic(x, depth + 1)).collect()),
        Value::Object(o) => Dynamic::from_map(o.iter().map(|(k, x)| (k.as_str().into(), to_dynamic(x, depth + 1))).collect()),
    }
}

/// A record's fields as the script's `fields` map.
pub(crate) fn fields_to_map(fields: &Fields) -> Map {
    fields.iter().map(|(k, v)| (k.as_str().into(), to_dynamic(v, 0))).collect()
}

/// A script value as JSON: `()` -> null, non-finite floats -> null, characters -> text,
/// and anything else without a JSON form (function pointers, ranges, blobs) -> its text.
pub(crate) fn to_json(d: Dynamic, depth: usize) -> Value {
    let d = d.flatten();
    if d.is_unit() {
        return Value::Null;
    }
    if let Ok(b) = d.as_bool() {
        return Value::Bool(b);
    }
    if let Ok(i) = d.as_int() {
        return Value::from(i);
    }
    if let Ok(f) = d.as_float() {
        return Number::from_f64(f).map_or(Value::Null, Value::Number);
    }
    if let Ok(c) = d.as_char() {
        return Value::String(c.to_string());
    }
    if d.is_string() {
        return Value::String(d.into_string().unwrap_or_default());
    }
    if (d.is_array() || d.is_map()) && depth >= MAX_DEPTH {
        return Value::Null;
    }
    if d.is_array() {
        let a: Array = d.cast();
        return Value::Array(a.into_iter().map(|x| to_json(x, depth + 1)).collect());
    }
    if d.is_map() {
        let m: Map = d.cast();
        return Value::Object(m.into_iter().map(|(k, x)| (k.to_string(), to_json(x, depth + 1))).collect());
    }
    Value::String(d.to_string())
}

/// The script's `fields` map back as record fields. Fields the record already had keep their
/// position; new ones follow in name order; fields missing from the map are removed.
pub(crate) fn map_to_fields(old: &Fields, mut map: Map) -> Fields {
    let mut out = Fields::new();
    for k in old.keys() {
        if let Some(v) = map.remove(k.as_str()) {
            out.insert(k.clone(), to_json(v, 0));
        }
    }
    for (k, v) in map {
        out.insert(k.to_string(), to_json(v, 0));
    }
    out
}
