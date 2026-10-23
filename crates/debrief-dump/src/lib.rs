//! Debrief's dump engine (ADR 0023): crash/memory dumps and UMDH heap logs as documents, and every analysis over
//! them. Pure: the dumps plugin (`debrief-plugin-dumps`) parses the untrusted files in its sandboxed process and sends
//! a [`model::Document`] ([`wire`]); this crate checks it, keeps it per window ([`store`]) and answers the views'
//! queries ([`api`]): overview and exception card, threads and stacks, modules, problem-first findings, UMDH
//! growth by stack, module and function, the stack tree, timeline markers and the AI summary.
//!
//! - [`model`]: the document model;
//! - [`exceptions`]: exception codes with names and meanings;
//! - [`modules`]: which modules and functions are system, runtime, allocator or wait code;
//! - [`symbols`]: the [`symbols::SymbolProvider`] port, by PDB identity (implemented by the plugin's PDB reader
//!   over the symbol folders and the cache the host fills from symbol servers), and each module's symbol status;
//! - [`views`], [`findings`], [`umdh`], [`explain`]: the analyses;
//! - [`import`]: the importer port the plugin's importers are reached through.

pub mod api;
pub mod error;
pub mod exceptions;
pub mod explain;
pub mod findings;
pub mod import;
pub mod limits;
pub mod model;
pub mod modules;
pub mod store;
pub mod symbols;
pub mod umdh;
pub mod views;
pub mod wire;

#[doc(hidden)]
pub mod testing;

pub use error::{DumpError, ErrorCode, Result};
pub use import::{DumpImporter, ImportOptions, Produces, ReadAt, IMPORTERS_POINT};
pub use limits::Limits;
pub use model::Document;
pub use store::DumpDocs;
