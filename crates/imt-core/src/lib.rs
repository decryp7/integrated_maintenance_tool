//! IMT core: turns log and data files into events and answers the questions
//! the UI asks about them (paging, timeline, facets, charts).
//!
//! Pipeline: [`importers`] parse files into [`model::Record`]s according to a
//! [`profile::Profile`]; the [`workspace::Workspace`] merges them, runs the
//! profile's extractors and marker rules and serves queries.

pub mod analysis;
pub mod api;
pub mod archive;
pub mod chart;
pub mod condition;
pub mod exceptions;
pub mod export;
pub mod importers;
pub mod model;
pub mod patterns;
pub mod pipeline;
pub mod plugin;
pub mod profile;
pub mod project;
pub mod ql;
pub mod query;
pub mod store;
pub mod template;
pub mod time;
pub mod user_regex;
pub mod workspace;

pub use plugin::{Plugin, PluginRegistry, API_VERSION};
pub use profile::{Profile, ProfileError, DEFAULT_PROFILE_TOML};
pub use store::LogStore;
pub use workspace::Workspace;
