//! Configuration-like data (registry exports, JSON/XML/INI/TOML/YAML files,
//! msinfo reports) as a [`ConfigTree`] of typed values addressed by path, and
//! a path-based [`diff`] of two trees.
//!
//! Built-in parsers read JSON, XML, INI, TOML and YAML. Plugins add formats by
//! contributing a [`ConfigParser`] to the [`PARSERS_POINT`] extension point
//! (the registry plugin adds `.reg`, the Windows plugin `.nfo`);
//! [`ConfigParsers`] picks the best parser for a file and calls contributed
//! ones under their plugin's guard.
//!
//! Every parser enforces [`Limits`]: input size, nesting depth and node count.
//! A tree that reached the node limit is kept, flagged `truncated`.

pub mod diff;
pub mod dto;
pub mod export;
mod formats;
mod parsers;
pub mod snapshot;
mod tree;

pub use diff::{diff, Diff, DiffEntry, IgnoreSet, DEFAULT_IGNORE};
pub use dto::{
    ChangeKind, CompareOptions, DiffCounts, DiffFormats, DiffOptions, DiffPage, DiffRow, DiffView, ExpandMode, FormatInfo, SideInfo,
    ValueDto, ValueKind,
};
pub use formats::{IniParser, JsonParser, TomlParser, XmlParser, YamlParser};
pub use parsers::{read_config, ConfigParsers};
pub use tree::{ConfigNode, ConfigTree, ConfigValue, PathSeg, TreeBuilder};

/// Extension point id for [`ConfigParser`] contributions.
pub const PARSERS_POINT: &str = "config.parsers";

/// Bounds every parser and the diff respect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
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

/// A configuration format: recognises files and turns their text into a tree.
/// Implementations never panic (contributed ones are guarded anyway) and
/// respect `limits` (use [`TreeBuilder`]).
pub trait ConfigParser: Send + Sync {
    /// Stable id, e.g. `json`.
    fn id(&self) -> &str;
    /// Name shown to users, e.g. `JSON`.
    fn name(&self) -> &str;
    /// File extensions (lower case, no dot) this parser is meant for.
    fn extensions(&self) -> Vec<String>;
    /// Confidence 0–100 that the file is in this format, from its name and the
    /// start of its text. 0 = not this format.
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
