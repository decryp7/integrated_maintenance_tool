//! TOML (`.toml`).

use super::{by_extension, check_nesting};
use crate::config::{ConfigError, ConfigParser, ConfigTree, ConfigValue, Limits, PathSeg, TreeBuilder};
use toml::Value;

pub struct TomlParser;

impl ConfigParser for TomlParser {
    fn id(&self) -> &str {
        "toml"
    }

    fn name(&self) -> &str {
        "TOML"
    }

    fn extensions(&self) -> Vec<String> {
        vec!["toml".into()]
    }

    fn sniff(&self, file_name: &str, _head: &str) -> u8 {
        by_extension(file_name, &["toml"])
    }

    fn parse(&self, content: &str, limits: &Limits) -> Result<ConfigTree, ConfigError> {
        check_nesting(content, limits.max_depth)?;
        let table: toml::Table =
            toml::from_str(content.trim_start_matches('\u{feff}')).map_err(|e| ConfigError::syntax("TOML", e.message()))?;
        let mut b = TreeBuilder::new(self.id(), limits);
        walk(&mut b, &mut Vec::new(), &Value::Table(table))?;
        Ok(b.finish())
    }
}

fn walk(b: &mut TreeBuilder, path: &mut Vec<PathSeg>, v: &Value) -> Result<(), ConfigError> {
    let scalar = match v {
        Value::Table(map) => {
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
            b.push(path.clone(), ConfigValue::Section)?;
            for (i, child) in items.iter().enumerate() {
                path.push(PathSeg::Index(i));
                walk(b, path, child)?;
                path.pop();
            }
            return Ok(());
        }
        Value::String(s) => ConfigValue::String(s.clone()),
        Value::Integer(i) => ConfigValue::Integer(*i),
        Value::Float(f) => ConfigValue::Float(*f),
        Value::Boolean(x) => ConfigValue::Bool(*x),
        Value::Datetime(d) => ConfigValue::String(d.to_string()),
    };
    b.push(path.clone(), scalar)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tree::display_path;

    #[test]
    fn reads_tables_arrays_and_scalars() {
        let t = TomlParser
            .parse(
                "title = \"x\"\nwhen = 1979-05-27T07:32:00Z\n[db]\nport = 5432\nratio = 0.5\non = true\nhosts = [\"a\", \"b\"]\n[[srv]]\nn = 1\n",
                &Limits::default(),
            )
            .unwrap();
        let shown: Vec<String> = t
            .nodes
            .iter()
            .map(|n| format!("{}={}", display_path(&n.path), n.value.text()))
            .collect();
        assert_eq!(
            shown,
            [
                "db=",
                "db/hosts=",
                "db/hosts[0]=a",
                "db/hosts[1]=b",
                "db/on=true",
                "db/port=5432",
                "db/ratio=0.5",
                "srv=",
                "srv[0]=",
                "srv[0]/n=1",
                "title=x",
                "when=1979-05-27T07:32:00Z",
            ]
        );
    }

    #[test]
    fn rejects_malformed_and_deep_input() {
        assert!(matches!(
            TomlParser.parse("a = ", &Limits::default()),
            Err(ConfigError::Syntax { .. })
        ));
        let deep = format!("a = {}", "[".repeat(300));
        assert_eq!(TomlParser.parse(&deep, &Limits::default()), Err(ConfigError::TooDeep { max: 128 }));
        assert_eq!(TomlParser.sniff("Cargo.TOML", ""), 80);
        assert_eq!(TomlParser.sniff("a.ini", "a = 1"), 0);
        assert_eq!(
            (TomlParser.id(), TomlParser.name(), TomlParser.extensions().len()),
            ("toml", "TOML", 1)
        );
    }
}
