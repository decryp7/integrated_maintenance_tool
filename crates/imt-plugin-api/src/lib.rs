//! The contract between IMT and its plugins (ADR 0017). A plugin depends on this crate alone:
//!
//! - [`plugin`]: the [`Plugin`] trait and the [`Contributions`] it registers (importers, configuration
//!   parsers, marker kinds, annotators, transforms, tools, settings, profile importers);
//! - the values plugins build: [`model`] (records), [`time`] (timestamps), [`text`] (decoding), [`config`]
//!   (configuration trees), [`settings`], [`tools`], [`markers`], [`annotate`], [`condition`], [`user_regex`];
//! - [`manifest`]: what a plugin is and contributes, sent to IMT on start;
//! - [`protocol`]: the requests IMT sends and the host services a plugin calls back, carried by `imt-ipc`'s
//!   frames;
//! - [`serve`]: a plugin process' `main` (`imt_plugin_api::serve(Arc::new(MyPlugin))`).
//!
//! The engine (`imt-core`) re-exports the shared values, so there is one definition of each.

pub mod annotate;
pub mod clef;
pub mod condition;
pub mod config;
pub mod host_file;
pub mod importer;
pub mod manifest;
pub mod markers;
pub mod model;
pub mod plugin;
pub mod process;
pub mod protocol;
pub mod serve;
pub mod settings;
pub mod text;
pub mod time;
pub mod tools;
pub mod user_regex;

pub use host_file::RangeRead;
pub use importer::{BinaryImporter, ImportCtx, Imported, TextImporter};
pub use manifest::{Manifest, Permission, PermissionRequest, CONTRACT};
pub use model::{Fields, Level, Record};
pub use plugin::{Contributions, DocumentImporter, Plugin, ProfileImporter, Transform, TransformCtx, Verdict};
pub use serve::serve;
