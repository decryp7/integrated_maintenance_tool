//! The dumps plugin (ADR 0023): readers for crash/memory dumps and UMDH heap logs, run in a sandboxed plugin
//! process; they produce `debrief-dump`'s document model, which Debrief checks and analyses. Its pure core (no
//! `debrief-plugin-api`; what the fuzz targets build):
//!
//! - [`source`]: a minidump into memory by ranges (full dumps keep only their stack memory);
//! - [`dump_reader`]: a minidump into a [`debrief_dump::model::Dump`] (rust-minidump, minidump-unwind);
//! - [`umdh_reader`]: UMDH snapshots and diff logs into a [`debrief_dump::model::UmdhLog`];
//! - [`import`]: both readers end to end, with symbols;
//! - [`pdb_symbols`]: function names from PDB files in local symbol folders (a [`debrief_dump::symbols::SymbolProvider`]);
//! - [`synth`]: synthetic minidumps for tests, fuzz seeds and samples.
//!
//! With the `plugin` feature: [`plugin`] (the importers, settings and the Create dump tool) and [`capture`]
//! (`MiniDumpWriteDump`, Windows).

pub mod dump_reader;
pub mod import;
pub mod pdb_symbols;
pub mod source;
#[doc(hidden)]
pub mod synth;
pub mod umdh_reader;

#[cfg(feature = "plugin")]
pub mod capture;
#[cfg(feature = "plugin")]
pub mod plugin;

#[cfg(feature = "plugin")]
pub use plugin::{DumpCapturePlugin, DumpsPlugin};
