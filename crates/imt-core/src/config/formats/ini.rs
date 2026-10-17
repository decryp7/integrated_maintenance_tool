//! INI files (`.ini`, `.cfg`, `.conf`, `.inf`, `.properties`): `[section]`
//! headers and `key = value` or `key: value` lines. Comments start with `;`
//! or `#`. Values are text (surrounding quotes removed); keys before the first
//! section sit at the top level. As on Windows, names compare without regard
//! to case; a repeated key keeps its last value.

use super::{by_extension, head_of};
use crate::config::{ConfigError, ConfigParser, ConfigTree, ConfigValue, Limits, PathSeg, TreeBuilder};

pub struct IniParser;

const EXTENSIONS: [&str; 5] = ["ini", "cfg", "conf", "inf", "properties"];

impl ConfigParser for IniParser {
    fn id(&self) -> &str {
        "ini"
    }

    fn name(&self) -> &str {
        "INI"
    }

    fn extensions(&self) -> Vec<String> {
        EXTENSIONS.iter().map(|e| e.to_string()).collect()
    }

    fn sniff(&self, file_name: &str, head: &str) -> u8 {
        let ext = by_extension(file_name, &EXTENSIONS);
        if ext > 0 {
            return ext;
        }
        let first = head_of(head).lines().next().unwrap_or("").trim();
        if first.starts_with('[') && first.ends_with(']') && !first.contains(',') {
            30
        } else {
            0
        }
    }

    fn parse(&self, content: &str, limits: &Limits) -> Result<ConfigTree, ConfigError> {
        let mut b = TreeBuilder::new(self.id(), limits).case_insensitive(true);
        let mut section: Option<String> = None;
        for (i, raw) in content.trim_start_matches('\u{feff}').lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix('[') {
                let name = rest
                    .strip_suffix(']')
                    .ok_or_else(|| ConfigError::syntax("INI", format!("line {}: section header without ']'", i + 1)))?;
                let name = name.trim().to_string();
                b.push(vec![PathSeg::Key(name.clone())], ConfigValue::Section)?;
                section = Some(name);
                continue;
            }
            let (key, value) = match line.find(['=', ':']) {
                Some(at) => (line[..at].trim(), Some(unquote(line[at + 1..].trim()))),
                None => (line, None),
            };
            if key.is_empty() {
                return Err(ConfigError::syntax("INI", format!("line {}: a value without a name", i + 1)));
            }
            let mut path: Vec<PathSeg> = section.iter().map(|s| PathSeg::Key(s.clone())).collect();
            path.push(PathSeg::key(key));
            b.push(path, value.map_or(ConfigValue::Null, |v| ConfigValue::String(v.to_string())))?;
        }
        Ok(b.finish())
    }
}

fn unquote(v: &str) -> &str {
    for q in ['"', '\''] {
        if v.len() >= 2 && v.starts_with(q) && v.ends_with(q) {
            return &v[1..v.len() - 1];
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tree::display_path;

    #[test]
    fn reads_sections_and_values() {
        let t = IniParser
            .parse(
                "\u{feff}; top\ntop=1\n[Server]\nHost = \"db01\"\nPort: 5432\n# c\nflag\nquote='x'\n",
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
                "top=1",
                "Server=",
                "Server/Host=db01",
                "Server/Port=5432",
                "Server/flag=null",
                "Server/quote=x"
            ]
        );
        assert!(t.case_insensitive);
    }

    #[test]
    fn rejects_malformed_lines() {
        let e = IniParser.parse("[broken\n", &Limits::default()).unwrap_err();
        assert_eq!(e.to_string(), "INI: line 1: section header without ']'");
        assert!(IniParser.parse("[a]\n = 3", &Limits::default()).is_err());
    }

    #[test]
    fn sniffs() {
        assert_eq!(IniParser.sniff("desktop.INI", ""), 80);
        assert_eq!(IniParser.sniff("x", "[General]\nA=1"), 30);
        assert_eq!(IniParser.sniff("x", "[1, 2]"), 0);
        assert_eq!(IniParser.sniff("x", "{"), 0);
        assert_eq!((IniParser.id(), IniParser.name(), IniParser.extensions().len()), ("ini", "INI", 5));
        assert_eq!(unquote("\""), "\"");
    }
}
