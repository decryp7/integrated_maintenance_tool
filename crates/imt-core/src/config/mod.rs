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

pub use diff::{diff, Diff, DiffEntry, IgnoreSet, DEFAULT_IGNORE};
pub use dto::{
    ChangeKind, CompareOptions, DiffCounts, DiffFormats, DiffOptions, DiffPage, DiffRow, DiffView, ExpandMode, FormatInfo, SideInfo,
    ValueDto, ValueKind,
};
pub use formats::{IniParser, JsonParser, TomlParser, XmlParser, YamlParser};
pub use imt_plugin_api::config::{
    display_path, extension_of, ConfigError, ConfigNode, ConfigParser, ConfigTree, ConfigValue, Limits, PathSeg, TreeBuilder,
};
pub use parsers::{read_config, ConfigParsers};

/// Extension point id for [`ConfigParser`] contributions.
pub const PARSERS_POINT: &str = "config.parsers";
