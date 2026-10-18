//! A record split into typed columns: the fixed ones, the promoted fields ([`Column`]) and the
//! rest of the fields as JSON. A value that does not fit its column's type (a `ProcessId` of
//! `"n/a"`) stays in the JSON, so nothing is lost by promoting a field.

use crate::port::{Column, ColumnKind};
use imt_core::model::Record;
use serde_json::{Map, Value};

/// A promoted value.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell<'a> {
    Null,
    Text(&'a str),
    Integer(i64),
    Float(f64),
}

fn typed(kind: ColumnKind, v: &Value) -> Option<Cell<'_>> {
    match (kind, v) {
        (ColumnKind::Text, Value::String(s)) => Some(Cell::Text(s)),
        (ColumnKind::Integer, Value::Number(n)) => n.as_i64().map(Cell::Integer),
        (ColumnKind::Float, Value::Number(n)) => n.as_f64().map(Cell::Float),
        _ => None,
    }
}

/// The promoted cells of `r`, in [`Column::ALL`] order.
pub fn promoted(r: &Record) -> [Cell<'_>; Column::ALL.len()] {
    Column::ALL.map(|c| r.fields.get(c.field()).and_then(|v| typed(c.kind(), v)).unwrap_or(Cell::Null))
}

/// The fields that did not go to a promoted column, as a JSON object (empty string when none).
pub fn rest_json(r: &Record) -> String {
    let mut rest = Map::new();
    for (k, v) in &r.fields {
        let promoted = Column::ALL.iter().any(|c| c.field() == k && typed(c.kind(), v).is_some());
        if !promoted {
            rest.insert(k.clone(), v.clone());
        }
    }
    if rest.is_empty() {
        String::new()
    } else {
        Value::Object(rest).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn promotes_typed_values_and_keeps_the_rest() {
        let mut r = Record::new(1);
        r.fields.insert("MachineName".into(), json!("WEB01"));
        r.fields.insert("ProcessId".into(), json!("n/a"));
        r.fields.insert("Elapsed".into(), json!(12.5));
        r.fields.insert("StatusCode".into(), json!(200));
        r.fields.insert("OrderId".into(), json!(7));
        let p = promoted(&r);
        assert_eq!(p[0], Cell::Text("WEB01"));
        assert_eq!(p[1], Cell::Null);
        assert_eq!(p[2], Cell::Null);
        assert_eq!(p[3], Cell::Integer(200));
        assert_eq!(p[4], Cell::Float(12.5));
        assert_eq!(rest_json(&r), r#"{"ProcessId":"n/a","OrderId":7}"#);
        assert_eq!(rest_json(&Record::new(2)), "");
    }
}
