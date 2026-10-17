//! YAML (`.yaml`, `.yml`), read with `yaml-rust2`'s event API.
//!
//! Why `yaml-rust2`: `serde_yaml` is deprecated and `serde_yml` is marked
//! unmaintained (RustSec); `yaml-rust2` is maintained, MIT/Apache-2.0, pure
//! Rust and YAML 1.2 compliant. Its *event* API lets us bound what its tree
//! loader would not: nesting is tracked on our own stack (no recursion), and
//! aliases are expanded by us under the node limit, so a "billion laughs"
//! document stops at [`Limits::max_nodes`] instead of exhausting memory.
//!
//! Plain scalars are typed with the YAML 1.2 core schema (null, booleans,
//! integers, floats); quoted and block scalars stay text. A merge key (`<<`)
//! is kept as an ordinary key. Documents after the first sit under `--- N`.

use super::by_extension;
use crate::config::{ConfigError, ConfigNode, ConfigParser, ConfigTree, ConfigValue, Limits, PathSeg, TreeBuilder};
use std::collections::HashMap;
use yaml_rust2::parser::{Event, Parser};
use yaml_rust2::scanner::TScalarStyle;

pub struct YamlParser;

impl ConfigParser for YamlParser {
    fn id(&self) -> &str {
        "yaml"
    }

    fn name(&self) -> &str {
        "YAML"
    }

    fn extensions(&self) -> Vec<String> {
        vec!["yaml".into(), "yml".into()]
    }

    fn sniff(&self, file_name: &str, head: &str) -> u8 {
        let ext = by_extension(file_name, &["yaml", "yml"]);
        if ext > 0 {
            ext
        } else if head.trim_start_matches('\u{feff}').starts_with("---") {
            40
        } else {
            0
        }
    }

    fn parse(&self, content: &str, limits: &Limits) -> Result<ConfigTree, ConfigError> {
        let mut state = State {
            b: TreeBuilder::new(self.id(), limits),
            stack: Vec::new(),
            anchors: HashMap::new(),
            budget: limits.max_nodes,
            documents: 0,
        };
        let mut parser = Parser::new_from_str(content.trim_start_matches('\u{feff}'));
        loop {
            let (event, _) = parser.next_token().map_err(|e| ConfigError::syntax("YAML", e))?;
            match event {
                Event::StreamEnd => break,
                Event::DocumentStart => state.documents += 1,
                Event::Scalar(text, style, anchor, _) => match state.place()? {
                    Place::Key(frame) => state.set_key(frame, text),
                    Place::At(path) => {
                        let value = typed(&text, style);
                        if anchor > 0 {
                            state.keep(
                                anchor,
                                vec![ConfigNode {
                                    path: Vec::new(),
                                    value: value.clone(),
                                }],
                            );
                        }
                        state.b.push(path, value)?;
                    }
                },
                Event::SequenceStart(anchor, _) => state.open(anchor, false)?,
                Event::MappingStart(anchor, _) => state.open(anchor, true)?,
                Event::SequenceEnd | Event::MappingEnd => state.close(),
                Event::Alias(id) => match state.place()? {
                    Place::Key(frame) => {
                        let key = match state.anchors.get(&id).and_then(Option::as_deref) {
                            Some([node]) if node.path.is_empty() && !node.value.is_section() => node.value.text(),
                            _ => return Err(ConfigError::syntax("YAML", "only a scalar alias can be a key")),
                        };
                        state.set_key(frame, key);
                    }
                    Place::At(path) => state.replay(id, path)?,
                },
                _ => {}
            }
            if state.b.is_full() {
                state.b.mark_truncated();
                break;
            }
        }
        Ok(state.b.finish())
    }
}

/// An open mapping or sequence.
struct Frame {
    path: Vec<PathSeg>,
    /// Mapping: the key read, waiting for its value. Sequence: unused.
    key: Option<String>,
    mapping: bool,
    next: usize,
    anchor: usize,
    /// Number of nodes before this container's own node.
    start: usize,
}

enum Place {
    /// The next node is the key of the mapping at this stack index.
    Key(usize),
    /// The next node is a value at this path.
    At(Vec<PathSeg>),
}

struct State {
    b: TreeBuilder,
    stack: Vec<Frame>,
    /// Nodes of each anchored node, with paths relative to it.
    /// `None`: the anchor was too large to keep (its aliases are dropped).
    anchors: HashMap<usize, Option<Vec<ConfigNode>>>,
    /// Anchored nodes that may still be kept (all anchors share the node limit).
    budget: usize,
    documents: usize,
}

impl State {
    /// Where the next node goes (consuming a mapping's pending key).
    fn place(&mut self) -> Result<Place, ConfigError> {
        let documents = self.documents;
        let Some((index, frame)) = self.stack.iter_mut().enumerate().next_back() else {
            return Ok(Place::At(if documents > 1 {
                vec![PathSeg::Key(format!("--- {documents}"))]
            } else {
                Vec::new()
            }));
        };
        let mut path = frame.path.clone();
        if frame.mapping {
            match frame.key.take() {
                None => return Ok(Place::Key(index)),
                Some(k) => path.push(PathSeg::Key(k)),
            }
        } else {
            path.push(PathSeg::Index(frame.next));
            frame.next += 1;
        }
        Ok(Place::At(path))
    }

    fn set_key(&mut self, frame: usize, key: String) {
        self.stack[frame].key = Some(key);
    }

    fn open(&mut self, anchor: usize, mapping: bool) -> Result<(), ConfigError> {
        let path = match self.place()? {
            Place::Key(_) => return Err(ConfigError::syntax("YAML", "a mapping or sequence as a key is not supported")),
            Place::At(path) => path,
        };
        self.b.check_depth(self.stack.len() + 1)?;
        let start = self.b.nodes().len();
        if !path.is_empty() {
            self.b.push(path.clone(), ConfigValue::Section)?;
        }
        self.stack.push(Frame {
            path,
            key: None,
            mapping,
            next: 0,
            anchor,
            start,
        });
        Ok(())
    }

    fn close(&mut self) {
        let Some(frame) = self.stack.pop() else { return };
        if frame.anchor > 0 {
            let nodes = self.b.nodes()[frame.start..]
                .iter()
                .map(|n| ConfigNode {
                    path: n.path[frame.path.len().min(n.path.len())..].to_vec(),
                    value: n.value.clone(),
                })
                .collect();
            self.keep(frame.anchor, nodes);
        }
    }

    /// Remembers an anchor's nodes while they fit the budget.
    fn keep(&mut self, anchor: usize, nodes: Vec<ConfigNode>) {
        if nodes.len() <= self.budget {
            self.budget -= nodes.len();
            self.anchors.insert(anchor, Some(nodes));
        } else {
            self.anchors.insert(anchor, None);
        }
    }

    /// Copies an anchored node to `path` (bounded by the node limit).
    fn replay(&mut self, id: usize, path: Vec<PathSeg>) -> Result<(), ConfigError> {
        // The parser rejects unknown anchors; a dropped one leaves the tree incomplete.
        let Some(Some(nodes)) = self.anchors.get(&id) else {
            self.b.mark_truncated();
            return Ok(());
        };
        for n in nodes {
            if self.b.is_full() {
                break;
            }
            let mut p = path.clone();
            p.extend(n.path.iter().cloned());
            self.b.push(p, n.value.clone())?;
        }
        Ok(())
    }
}

/// A scalar's value under the YAML 1.2 core schema.
fn typed(text: &str, style: TScalarStyle) -> ConfigValue {
    if style != TScalarStyle::Plain {
        return ConfigValue::String(text.to_string());
    }
    match text {
        "" | "~" | "null" | "Null" | "NULL" => return ConfigValue::Null,
        "true" | "True" | "TRUE" => return ConfigValue::Bool(true),
        "false" | "False" | "FALSE" => return ConfigValue::Bool(false),
        ".inf" | ".Inf" | ".INF" | "+.inf" | "+.Inf" | "+.INF" => return ConfigValue::Float(f64::INFINITY),
        "-.inf" | "-.Inf" | "-.INF" => return ConfigValue::Float(f64::NEG_INFINITY),
        ".nan" | ".NaN" | ".NAN" => return ConfigValue::Float(f64::NAN),
        _ => {}
    }
    if let Some(hex) = text.strip_prefix("0x") {
        if let Ok(i) = i64::from_str_radix(hex, 16) {
            return ConfigValue::Integer(i);
        }
    }
    if let Some(oct) = text.strip_prefix("0o") {
        if let Ok(i) = i64::from_str_radix(oct, 8) {
            return ConfigValue::Integer(i);
        }
    }
    let numeric = text
        .bytes()
        .all(|c| c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.' | b'e' | b'E'));
    if numeric && text.bytes().any(|c| c.is_ascii_digit()) {
        if let Ok(i) = text.parse::<i64>() {
            return ConfigValue::Integer(i);
        }
        if let Ok(f) = text.parse::<f64>() {
            return ConfigValue::Float(f);
        }
    }
    ConfigValue::String(text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tree::display_path;

    fn shown(s: &str) -> Vec<String> {
        let t = YamlParser.parse(s, &Limits::default()).unwrap();
        t.nodes
            .iter()
            .map(|n| format!("{}={}", display_path(&n.path), n.value.text()))
            .collect()
    }

    #[test]
    fn reads_mappings_sequences_and_typed_scalars() {
        assert_eq!(
            shown("\u{feff}a: 1\nb: [x, 'y', 2.5, true, ~, 0x1f, 0o17, .inf, -.inf, 1e3, 12abc, \"3\"]\nc:\n  d: null\n"),
            [
                "a=1",
                "b=",
                "b[0]=x",
                "b[1]=y",
                "b[2]=2.5",
                "b[3]=true",
                "b[4]=null",
                "b[5]=31",
                "b[6]=15",
                "b[7]=inf",
                "b[8]=-inf",
                "b[9]=1000",
                "b[10]=12abc",
                "b[11]=3",
                "c=",
                "c/d=null",
            ]
        );
        assert_eq!(shown("False"), ["=false"]);
        assert_eq!(
            shown("- .nan\n- 0xZZ\n- 0o9\n- '-'\n- -\n- 1-2\n"),
            ["[0]=NaN", "[1]=0xZZ", "[2]=0o9", "[3]=-", "[4]=", "[4][0]=null", "[5]=1-2"]
        );
    }

    #[test]
    fn expands_aliases_and_documents() {
        assert_eq!(
            shown("base: &b {x: 1, y: [2]}\nname: &n web\ncopy: *b\n*n : 3\nother: *n\n---\nz: 1\n"),
            [
                "base=",
                "base/x=1",
                "base/y=",
                "base/y[0]=2",
                "name=web",
                "copy=",
                "copy/x=1",
                "copy/y=",
                "copy/y[0]=2",
                "web=3",
                "other=web",
                "--- 2=",
                "--- 2/z=1",
            ]
        );
    }

    #[test]
    fn bounds_billion_laughs_and_nesting() {
        let mut doc = String::from("a: &a [x, x, x, x, x, x, x, x, x, x]\n");
        for i in 1..10 {
            let prev = (b'a' + i - 1) as char;
            let cur = (b'a' + i) as char;
            doc.push_str(&format!(
                "{cur}: &{cur} [*{prev}, *{prev}, *{prev}, *{prev}, *{prev}, *{prev}, *{prev}, *{prev}, *{prev}, *{prev}]\n"
            ));
        }
        let limits = Limits {
            max_nodes: 10_000,
            ..Limits::default()
        };
        let t = YamlParser.parse(&doc, &limits).unwrap();
        assert!(t.truncated);
        assert_eq!(t.nodes.len(), 10_000);
        // Anchors beyond the budget are dropped; their aliases mark the tree truncated.
        let tiny = Limits {
            max_nodes: 3,
            ..Limits::default()
        };
        let t = YamlParser
            .parse(
                "a: &a [1, 2, 3, 4]
",
                &tiny,
            )
            .unwrap();
        assert!(t.truncated);
        let t = YamlParser
            .parse(
                "a: &a [1, 2]
b: *a
",
                &tiny,
            )
            .unwrap();
        assert!(t.truncated);
        // A nested anchor uses the budget first; the outer one no longer fits and its alias is dropped.
        let six = Limits {
            max_nodes: 6,
            ..Limits::default()
        };
        let t = YamlParser.parse("a: &a [&b [1, 2]]\nc: *a\n", &six).unwrap();
        assert!(t.truncated);
        assert_eq!(t.nodes.len(), 4);
        // Block nesting is bounded by our stack; flow nesting by the scanner.
        let deep: String = (0..200).map(|i| format!("{}a:\n", "  ".repeat(i))).collect();
        assert_eq!(YamlParser.parse(&deep, &Limits::default()), Err(ConfigError::TooDeep { max: 128 }));
        assert!(matches!(
            YamlParser.parse(&"[".repeat(1000), &Limits::default()),
            Err(ConfigError::Syntax { .. })
        ));
    }

    #[test]
    fn rejects_malformed_input() {
        let l = Limits::default();
        assert!(matches!(YamlParser.parse("a: [1, 2", &l), Err(ConfigError::Syntax { .. })));
        assert!(YamlParser.parse("? [a]\n: 1\n", &l).unwrap_err().to_string().contains("as a key"));
        assert!(YamlParser.parse("a: *nope\n", &l).is_err());
        assert!(YamlParser
            .parse("x: &m {a: 1}\n*m : 2\n", &l)
            .unwrap_err()
            .to_string()
            .contains("scalar alias"));
        assert_eq!(YamlParser.sniff("a.yml", ""), 80);
        assert_eq!(YamlParser.sniff("a", "---\n"), 40);
        assert_eq!(YamlParser.sniff("a", "x"), 0);
        assert_eq!(
            (YamlParser.id(), YamlParser.name(), YamlParser.extensions().len()),
            ("yaml", "YAML", 2)
        );
    }
}
