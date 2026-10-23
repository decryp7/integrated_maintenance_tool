//! The configuration tree model: typed values addressed by path.

use super::{ConfigError, Limits};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

/// One step of a path: a named key (object member, section, registry key,
/// XML element) or a position in a list.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PathSeg {
    /// A named key.
    Key(String),
    /// A position in a list (zero-based).
    Index(usize),
}

impl PathSeg {
    /// A key segment.
    pub fn key(s: impl Into<String>) -> PathSeg {
        PathSeg::Key(s.into())
    }

    /// The segment as shown: the key, or `[i]`.
    pub fn label(&self) -> String {
        match self {
            PathSeg::Key(k) => k.clone(),
            PathSeg::Index(i) => format!("[{i}]"),
        }
    }
}

/// Display form of a path: keys joined with `/`, list positions as `[i]`
/// (`servers[0]/host`).
pub fn display_path(path: &[PathSeg]) -> String {
    let mut out = String::new();
    for (i, seg) in path.iter().enumerate() {
        match seg {
            PathSeg::Key(k) => {
                if i > 0 {
                    out.push('/');
                }
                out.push_str(k);
            }
            PathSeg::Index(n) => {
                out.push('[');
                out.push_str(&n.to_string());
                out.push(']');
            }
        }
    }
    out
}

/// A typed value. Containers (objects, sections, keys, elements without
/// text) are [`ConfigValue::Section`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConfigValue {
    /// A container: object, section, registry key, or element without text.
    Section,
    /// An explicit null.
    Null,
    /// A boolean.
    Bool(bool),
    /// A signed integer.
    Integer(i64),
    /// A floating-point number.
    Float(f64),
    /// Text.
    String(String),
    /// Raw bytes.
    Binary(Vec<u8>),
    /// A list of text items.
    List(Vec<String>),
}

impl PartialEq for ConfigValue {
    fn eq(&self, other: &Self) -> bool {
        use ConfigValue::*;
        match (self, other) {
            (Section, Section) | (Null, Null) => true,
            (Bool(a), Bool(b)) => a == b,
            (Integer(a), Integer(b)) => a == b,
            // Total order: NaN equals NaN, so an unchanged NaN is not a change.
            (Float(a), Float(b)) => a.total_cmp(b) == Ordering::Equal,
            (String(a), String(b)) => a == b,
            (Binary(a), Binary(b)) => a == b,
            (List(a), List(b)) => a == b,
            _ => false,
        }
    }
}

impl ConfigValue {
    /// The value as text (empty for a section).
    pub fn text(&self) -> String {
        match self {
            ConfigValue::Section => String::new(),
            ConfigValue::Null => "null".into(),
            ConfigValue::Bool(b) => b.to_string(),
            ConfigValue::Integer(i) => i.to_string(),
            ConfigValue::Float(f) => f.to_string(),
            ConfigValue::String(s) => s.clone(),
            ConfigValue::Binary(b) => b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" "),
            ConfigValue::List(items) => serde_json::to_string(items).unwrap_or_default(),
        }
    }

    /// Whether this is a container.
    pub fn is_section(&self) -> bool {
        matches!(self, ConfigValue::Section)
    }
}

/// One node of a tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigNode {
    /// Where the node sits in the tree.
    pub path: Vec<PathSeg>,
    /// The node's value.
    pub value: ConfigValue,
}

/// A parsed configuration file. Nodes are in document order; a later node
/// with the same path replaces an earlier one. Missing ancestors are implied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigTree {
    /// Id of the parser that read it.
    pub format: String,
    /// Paths compare without regard to case (registry keys and value names).
    pub case_insensitive: bool,
    /// The node limit was reached; later nodes are missing.
    pub truncated: bool,
    /// The nodes, in document order.
    pub nodes: Vec<ConfigNode>,
}

/// Collects nodes while enforcing [`Limits`]: too deep a path fails, nodes
/// beyond the limit are dropped and the tree is flagged truncated.
#[derive(Debug)]
pub struct TreeBuilder {
    tree: ConfigTree,
    limits: Limits,
}

impl TreeBuilder {
    /// Starts a tree for parser `format`, enforcing `limits`.
    pub fn new(format: &str, limits: &Limits) -> TreeBuilder {
        TreeBuilder {
            tree: ConfigTree {
                format: format.into(),
                case_insensitive: false,
                truncated: false,
                nodes: Vec::new(),
            },
            limits: *limits,
        }
    }

    /// Paths of this tree compare without regard to case.
    pub fn case_insensitive(mut self, yes: bool) -> TreeBuilder {
        self.tree.case_insensitive = yes;
        self
    }

    /// Adds a node. Fails when the path is deeper than the limit; ignores it
    /// (flagging the tree truncated) when the tree is full.
    pub fn push(&mut self, path: Vec<PathSeg>, value: ConfigValue) -> Result<(), ConfigError> {
        self.check_depth(path.len())?;
        if self.is_full() {
            self.tree.truncated = true;
            return Ok(());
        }
        self.tree.nodes.push(ConfigNode { path, value });
        Ok(())
    }

    /// Fails when a nesting `depth` exceeds the limit.
    pub fn check_depth(&self, depth: usize) -> Result<(), ConfigError> {
        if depth > self.limits.max_depth {
            return Err(ConfigError::TooDeep {
                max: self.limits.max_depth,
            });
        }
        Ok(())
    }

    /// Whether the node limit is reached (parsers may stop early).
    pub fn is_full(&self) -> bool {
        self.tree.nodes.len() >= self.limits.max_nodes
    }

    /// Flags the tree as truncated (a parser stopped early at the limit).
    pub fn mark_truncated(&mut self) {
        self.tree.truncated = true;
    }

    /// Nodes added so far.
    pub fn nodes(&self) -> &[ConfigNode] {
        &self.tree.nodes
    }

    /// The finished tree.
    pub fn finish(self) -> ConfigTree {
        self.tree
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_values_display() {
        let p = vec![PathSeg::key("servers"), PathSeg::Index(0), PathSeg::key("host")];
        assert_eq!(display_path(&p), "servers[0]/host");
        assert_eq!(display_path(&[PathSeg::Index(2)]), "[2]");
        assert_eq!(PathSeg::Index(3).label(), "[3]");
        assert_eq!(PathSeg::key("a").label(), "a");
        assert_eq!(ConfigValue::Section.text(), "");
        assert_eq!(ConfigValue::Null.text(), "null");
        assert_eq!(ConfigValue::Bool(true).text(), "true");
        assert_eq!(ConfigValue::Integer(-4).text(), "-4");
        assert_eq!(ConfigValue::Float(1.5).text(), "1.5");
        assert_eq!(ConfigValue::String("x".into()).text(), "x");
        assert_eq!(ConfigValue::Binary(vec![0xde, 0xad]).text(), "de ad");
        assert_eq!(ConfigValue::List(vec!["a".into(), "b".into()]).text(), "[\"a\",\"b\"]");
        assert!(ConfigValue::Section.is_section());
        assert!(!ConfigValue::Null.is_section());
    }

    #[test]
    fn value_equality_is_typed() {
        assert_eq!(ConfigValue::Float(f64::NAN), ConfigValue::Float(f64::NAN));
        assert_ne!(ConfigValue::Integer(1), ConfigValue::String("1".into()));
        assert_ne!(ConfigValue::Integer(1), ConfigValue::Float(1.0));
        assert_eq!(ConfigValue::Section, ConfigValue::Section);
        assert_eq!(ConfigValue::Null, ConfigValue::Null);
        assert_eq!(ConfigValue::Bool(false), ConfigValue::Bool(false));
        assert_eq!(ConfigValue::Binary(vec![1]), ConfigValue::Binary(vec![1]));
        assert_eq!(ConfigValue::List(vec![]), ConfigValue::List(vec![]));
    }

    #[test]
    fn builder_enforces_limits() {
        let limits = Limits {
            max_bytes: 10,
            max_depth: 2,
            max_nodes: 1,
        };
        let mut b = TreeBuilder::new("t", &limits).case_insensitive(true);
        assert!(!b.is_full());
        b.push(vec![PathSeg::key("a")], ConfigValue::Null).unwrap();
        assert!(b.is_full());
        b.push(vec![PathSeg::key("b")], ConfigValue::Null).unwrap();
        assert_eq!(
            b.push(vec![PathSeg::Index(0); 3], ConfigValue::Null),
            Err(ConfigError::TooDeep { max: 2 })
        );
        let t = b.finish();
        assert!(t.truncated && t.case_insensitive);
        assert_eq!(t.nodes.len(), 1);
        assert_eq!(t.format, "t");
    }
}
