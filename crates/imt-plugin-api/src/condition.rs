//! Field conditions as written in profile rules (`when = [...]`), marker kinds and annotator filters. The
//! engine compiles and evaluates them; plugins only build them.

use serde::{Deserialize, Serialize};

/// One condition: `field` (a field name or `@level`, `@message`, ...), an operator (`eq`, `ne`, `contains`,
/// `not_contains`, `matches`, `exists`, `missing`, `gt`, `lt`, `in`) and a value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConditionCfg {
    /// Field name or pseudo-field (`@level`, `@message`, `@exception`, `@template`, `@source`, `@line`).
    pub field: String,
    /// Operator name; see the type docs for the supported set.
    pub op: String,
    /// Operand; absent for operators that take none (`exists`, `missing`).
    #[serde(default)]
    pub value: Option<CondValue>,
}

/// Lets TOML write `value = 5`, `value = true`, `value = "x"` or `value = ["a", "b"]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CondValue {
    /// A text operand.
    Text(String),
    /// A numeric operand.
    Number(f64),
    /// A boolean operand.
    Bool(bool),
    /// A list of alternatives, used by `in`.
    List(Vec<String>),
}

impl CondValue {
    /// The value as text (`a|b` for a list).
    pub fn text(&self) -> String {
        match self {
            CondValue::Text(s) => s.clone(),
            CondValue::Number(n) => n.to_string(),
            CondValue::Bool(b) => b.to_string(),
            CondValue::List(l) => l.join("|"),
        }
    }
}

impl ConditionCfg {
    /// `field op value` with a text value.
    pub fn new(field: &str, op: &str, value: CondValue) -> ConditionCfg {
        ConditionCfg {
            field: field.into(),
            op: op.into(),
            value: Some(value),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_read_from_plain_json_and_print_as_text() {
        let c: ConditionCfg = serde_json::from_str(r#"{"field":"EventId","op":"in","value":["1","2"]}"#).unwrap();
        assert_eq!(c.value.as_ref().unwrap().text(), "1|2");
        let n: CondValue = serde_json::from_str("5").unwrap();
        assert_eq!(n.text(), "5");
        assert_eq!(CondValue::Bool(true).text(), "true");
        assert_eq!(ConditionCfg::new("a", "eq", CondValue::Text("b".into())).value.unwrap().text(), "b");
    }
}
