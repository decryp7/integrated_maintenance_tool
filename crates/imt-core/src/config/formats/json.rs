//! JSON (`.json`, and JSON-shaped text).

use super::{by_extension, check_nesting, head_of};
use crate::config::{ConfigError, ConfigParser, ConfigTree, ConfigValue, Limits, PathSeg, TreeBuilder};
use serde_json::Value;

pub struct JsonParser;

impl ConfigParser for JsonParser {
    fn id(&self) -> &str {
        "json"
    }

    fn name(&self) -> &str {
        "JSON"
    }

    fn extensions(&self) -> Vec<String> {
        vec!["json".into()]
    }

    fn sniff(&self, file_name: &str, head: &str) -> u8 {
        let ext = by_extension(file_name, &["json"]);
        if ext > 0 {
            return ext;
        }
        match head_of(head).chars().next() {
            Some('{') => 60,
            Some('[') => 40,
            _ => 0,
        }
    }

    fn parse(&self, content: &str, limits: &Limits) -> Result<ConfigTree, ConfigError> {
        check_nesting(content, limits.max_depth)?;
        let value: Value = serde_json::from_str(content.trim_start_matches('\u{feff}')).map_err(|e| ConfigError::syntax("JSON", e))?;
        let mut b = TreeBuilder::new(self.id(), limits);
        walk(&mut b, &mut Vec::new(), &value)?;
        Ok(b.finish())
    }
}

/// Adds `v` at `path`: objects and arrays become sections with children.
pub(crate) fn walk(b: &mut TreeBuilder, path: &mut Vec<PathSeg>, v: &Value) -> Result<(), ConfigError> {
    let scalar = match v {
        Value::Object(map) => {
            if !path.is_empty() {
                b.push(path.clone(), ConfigValue::Section)?;
            }
            for (k, child) in map {
                path.push(PathSeg::Key(k.clone()));
                walk(b, path, child)?;
                path.pop();
            }
            return Ok(());
        }
        Value::Array(items) => {
            if !path.is_empty() {
                b.push(path.clone(), ConfigValue::Section)?;
            }
            for (i, child) in items.iter().enumerate() {
                path.push(PathSeg::Index(i));
                walk(b, path, child)?;
                path.pop();
            }
            return Ok(());
        }
        Value::Null => ConfigValue::Null,
        Value::Bool(x) => ConfigValue::Bool(*x),
        Value::Number(n) => match n.as_i64() {
            Some(i) => ConfigValue::Integer(i),
            None => ConfigValue::Float(n.as_f64().unwrap_or(f64::NAN)),
        },
        Value::String(s) => ConfigValue::String(s.clone()),
    };
    b.push(path.clone(), scalar)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::display_path;

    fn parse(s: &str) -> Result<ConfigTree, ConfigError> {
        JsonParser.parse(s, &Limits::default())
    }

    #[test]
    fn reads_typed_values() {
        let t = parse("\u{feff}{\"a\": {\"b\": [1, 2.5, true, null, \"x\"]}, \"big\": 18446744073709551615}").unwrap();
        let shown: Vec<(String, String)> = t.nodes.iter().map(|n| (display_path(&n.path), n.value.text())).collect();
        assert_eq!(
            shown,
            [
                ("a", ""),
                ("a/b", ""),
                ("a/b[0]", "1"),
                ("a/b[1]", "2.5"),
                ("a/b[2]", "true"),
                ("a/b[3]", "null"),
                ("a/b[4]", "x"),
                ("big", "18446744073709552000"),
            ]
            .map(|(a, b)| (a.to_string(), b.to_string()))
        );
        assert_eq!(t.format, "json");
        assert!(!t.case_insensitive);
        assert_eq!(parse("5").unwrap().nodes[0].path, vec![]);
        assert_eq!(parse("[]").unwrap().nodes.len(), 0);
    }

    #[test]
    fn rejects_malformed_and_deep_input() {
        assert!(matches!(parse("{\"a\": "), Err(ConfigError::Syntax { .. })));
        assert!(parse("").is_err());
        assert_eq!(parse(&"[".repeat(500)), Err(ConfigError::TooDeep { max: 128 }));
    }

    #[test]
    fn sniffs() {
        assert_eq!(JsonParser.sniff("a.json", ""), 80);
        assert_eq!(JsonParser.sniff("a.txt", " {"), 60);
        assert_eq!(JsonParser.sniff("a.txt", "["), 40);
        assert_eq!(JsonParser.sniff("a.txt", "x"), 0);
        assert_eq!(
            (JsonParser.id(), JsonParser.name(), JsonParser.extensions()),
            ("json", "JSON", vec!["json".to_string()])
        );
    }
}
