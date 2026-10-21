//! Configuration-like data (registry exports, msinfo reports, JSON/XML/INI/TOML/YAML files) as a
//! [`ConfigTree`] of typed values addressed by path. The engine diffs trees; plugins add formats by
//! contributing a [`ConfigParser`].
//!
//! Every parser enforces [`Limits`]: input size, nesting depth and node count. A tree that reached the node
//! limit is kept, flagged `truncated`.

mod tree;

use serde::{Deserialize, Serialize};
pub use tree::{display_path, ConfigNode, ConfigTree, ConfigValue, PathSeg, TreeBuilder};

/// Bounds every parser and the diff respect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    /// Largest file read, in bytes.
    pub max_bytes: usize,
    /// Deepest path (segments).
    pub max_depth: usize,
    /// Most nodes per tree; more are dropped and the tree is flagged truncated.
    pub max_nodes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_bytes: 64 << 20,
            max_depth: 128,
            max_nodes: 500_000,
        }
    }
}

/// Why a configuration file could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum ConfigError {
    #[error("the file is {bytes} bytes; the limit is {max}")]
    TooLarge { bytes: u64, max: usize },
    #[error("nesting is deeper than {max} levels")]
    TooDeep { max: usize },
    #[error("this is a binary file, not a configuration file")]
    Binary,
    #[error("no configuration parser recognises this file (supported: {supported})")]
    Unrecognised { supported: String },
    #[error("{format}: {message}")]
    Syntax { format: String, message: String },
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    Plugin(String),
}

impl ConfigError {
    /// A syntax error of `format`.
    pub fn syntax(format: &str, message: impl std::fmt::Display) -> ConfigError {
        ConfigError::Syntax {
            format: format.into(),
            message: message.to_string(),
        }
    }
}

/// A configuration format: recognises files and turns their text into a tree. Implementations never panic
/// and respect `limits` (use [`TreeBuilder`]).
pub trait ConfigParser: Send + Sync {
    /// Stable id, e.g. `json`.
    fn id(&self) -> &str;
    /// Name shown to users, e.g. `JSON`.
    fn name(&self) -> &str;
    /// File extensions (lower case, no dot) this parser is meant for.
    fn extensions(&self) -> Vec<String>;
    /// Confidence 0–100 that the file is in this format, from its name and the start of its text. 0 = not
    /// this format.
    fn sniff(&self, file_name: &str, head: &str) -> u8;
    /// Parses the whole text.
    fn parse(&self, content: &str, limits: &Limits) -> Result<ConfigTree, ConfigError>;
}

/// Lower-case extension of a file name ("" when none).
pub fn extension_of(file_name: &str) -> String {
    std::path::Path::new(file_name)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trees_and_errors_cross_the_wire() {
        let mut b = TreeBuilder::new("reg", &Limits::default()).case_insensitive(true);
        b.push(vec![PathSeg::key("HKLM"), PathSeg::Index(2)], ConfigValue::Float(1.5))
            .unwrap();
        b.push(vec![PathSeg::key("b")], ConfigValue::Binary(vec![1, 2])).unwrap();
        let t = b.finish();
        let back: ConfigTree = postcard::from_bytes(&postcard::to_allocvec(&t).unwrap()).unwrap();
        assert_eq!(back, t);
        let e = ConfigError::syntax("reg", "bad line");
        let back: ConfigError = postcard::from_bytes(&postcard::to_allocvec(&e).unwrap()).unwrap();
        assert_eq!(back.to_string(), "reg: bad line");
        assert_eq!(extension_of("a/B.REG"), "reg");
        assert_eq!(extension_of("none"), "");
    }
}
