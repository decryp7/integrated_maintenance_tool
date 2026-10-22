//! IMT core: turns log and data files into events and answers the questions
//! the UI asks about them (paging, timeline, facets, charts).
//!
//! Pipeline: [`importers`] parse files into [`model::Record`]s according to a
//! [`profile::Profile`]; the [`workspace::Workspace`] merges them, runs the
//! profile's extractors and marker rules and serves queries.

pub mod analysis;
pub mod api;
pub mod archive;
pub mod bundle;
pub mod chart;
pub mod condition;
pub mod config;
pub mod dashboard;
pub mod events;
pub mod exceptions;
pub mod export;
pub mod file_target;
pub mod guard;
pub mod home;
pub mod icons;
pub mod importers;
pub mod ingest;
pub mod instance;
pub mod overwrite;
pub mod package;
pub mod parser;
pub mod patterns;
pub mod pipeline;
pub mod plugin;
pub mod ports;
pub mod profile;
pub mod profile_change;
pub mod project;
pub mod ql;
pub mod query;
pub mod redact;
pub mod regex_assist;
pub mod related;
pub mod report;
pub mod route;
pub mod row;
pub mod script;
pub mod search_all;
pub mod settings;
pub mod skew;
pub mod spans;
pub mod store;
pub mod survey;
pub mod svg;
pub mod template;
pub mod tiles;
pub mod timestamp_issue;
pub mod words;
pub mod workspace;

// The values plugins build live in the plugin contract (ADR 0017); the engine uses the same definitions.
pub use imt_plugin_api::{model, process, time, user_regex};

/// The event batch of ADR 0014 §3 ([`imt_batch`]).
pub use imt_batch as batch;
pub use plugin::{Plugin, PluginRegistry, API_VERSION};
pub use profile::{Profile, ProfileError, DEFAULT_PROFILE_TOML};
pub use store::LogStore;
pub use workspace::Workspace;
