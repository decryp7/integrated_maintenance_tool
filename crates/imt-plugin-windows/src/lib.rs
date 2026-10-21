//! IMT plugin for Windows diagnostics.
//!
//! * **Event logs** (`.evtx`, binary): one record per event with the time from
//!   `TimeCreated`, the level from `System/Level` and the fields `Channel`,
//!   `Provider`, `EventId`, `RecordId`, `Computer`, `ProcessId`, `ThreadId`,
//!   `Task`, `Keywords`, `UserId` plus every named `EventData`/`UserData` value
//!   under its own name. The rendered message needs the provider's message DLL
//!   and is not available offline, so the message is
//!   `Provider EventId: k1=v1, k2=v2`. Registered as a [`BinaryImporter`]
//!   ([`EvtxImporter`]); [`read_evtx`] and [`is_evtx`] are the pure functions
//!   behind it. Corrupt records or chunks are skipped and counted.
//! * **System information** (`.nfo`, msinfo32 XML, often UTF-16): one record per
//!   `Data` row with `Category` (full path, `A/B/C`), `Item`, `Value` and any
//!   other column by name; no timestamp.
//!
//! * **Performance Monitor** (`.csv`/`.tsv` PDH logs, `.blg` on Windows via `relog`):
//!   [`PerfmonCsvImporter`] and [`BlgImporter`] produce one record per row and counter
//!   (see [`perfmon`]). Creating and controlling Data Collector Sets is a tool of the
//!   tools plugin (`imt-plugin-tools`), which can be disabled on its own.
//!
//! * **Process Monitor** (Sysinternals ProcMon "Save as CSV"): [`ProcmonCsvImporter`]
//!   produces one record per operation with `ProcessName`, `ProcessId`, `Operation`,
//!   `Path`, `Result`, `Detail` and every optional column; the level follows `Result`
//!   (see [`procmon`]).
//!
//! * **Minidumps** (`.dmp`, `MDMP` signature): [`MinidumpImporter`] summarises a crash
//!   dump as a few records stamped with the dump time: system, process, the exception (an
//!   Error record with a readable code name and `module+offset`), the crashing thread's
//!   frames (instruction pointer plus a symbol-less stack scan) and the loaded modules
//!   (see [`minidump_import`]).
//!
//! * **Live event logs** ([`eventlog`]): follows channels (backfill with `EvtQuery`, then
//!   `EvtSubscribe` after a persisted bookmark) and maps each rendered event to the same
//!   record as the `.evtx` importer, with the `EvtFormatMessage` text as the message. The
//!   "Event Log follower" tool of the tools plugin drives it and writes rolling CLEF files.
//!
//! * **Markers and annotators**: [`markers::kinds`] marks crashes, hangs,
//!   unexpected shutdowns, boots, disk errors, updates and logon failures in
//!   event logs, and denied operations in ProcMon exports; [`annotators::RebootAnnotator`] and
//!   [`annotators::ServiceAnnotator`] add downtime and service uptime spans.
//!
//! CBS/DISM text logs (`2026-10-14 10:00:00, Info   CBS   message`) need no
//! importer: the built-in text importer reads them with a profile regex.
//!
//! It runs in its own process (`imt-plugins --plugin net.decryptology.windows`, ADR 0017); converting a `.blg`
//! runs `relog` on the file, so the plugin asks for the opened files' paths and to run programs.

pub mod annotators;
mod blg;
pub mod command;
pub mod eventlog;
mod evtx_import;
pub mod markers;
pub mod minidump_import;
mod nfo;
pub mod perfmon;
pub mod procmon;
pub mod settings;

pub use blg::BlgImporter;
pub use evtx_import::{is_evtx, read_evtx, to_record, EvtxImporter};
pub use minidump_import::{is_minidump, read_minidump, MinidumpImporter};
pub use nfo::{NfoConfigParser, NfoImporter};
pub use perfmon::PerfmonCsvImporter;
pub use procmon::ProcmonCsvImporter;

use anyhow::Result;
use imt_plugin_api::manifest::{Permission, PermissionRequest};
use imt_plugin_api::{Contributions, Plugin};
use std::sync::Arc;

#[derive(Default)]
pub struct WindowsPlugin;

impl Plugin for WindowsPlugin {
    fn id(&self) -> &'static str {
        "net.decryptology.windows"
    }

    fn name(&self) -> &'static str {
        "Windows diagnostics"
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn register(&self, c: &mut Contributions) -> Result<()> {
        c.add_binary_importer(EvtxImporter)
            .add_importer(NfoImporter)
            .add_config_parser(NfoConfigParser)
            .add_importer(PerfmonCsvImporter)
            .add_binary_importer(BlgImporter::default())
            .add_importer(ProcmonCsvImporter)
            .add_binary_importer(MinidumpImporter)
            .add_marker_kinds(markers::kinds())
            .add_annotator(annotators::RebootAnnotator)
            .add_annotator(annotators::ServiceAnnotator)
            .set_settings(Arc::new(settings::WindowsSettings));
        Ok(())
    }

    /// `.blg` files are converted by `relog`, which reads the file by its path and writes a temporary CSV.
    fn permissions(&self) -> Vec<PermissionRequest> {
        vec![
            PermissionRequest {
                permission: Permission::OpenedFilePaths,
                reason: "BLG files are converted by relog.exe, which reads the file by its path".into(),
            },
            PermissionRequest {
                permission: Permission::RunPrograms,
                reason: "BLG files are converted by relog.exe".into(),
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_registers_its_importers() {
        let mut c = Contributions::default();
        WindowsPlugin.register(&mut c).unwrap();
        let info = c.manifest(&WindowsPlugin);
        info.check().unwrap();
        assert_eq!(&*info.id, "net.decryptology.windows");
        assert_eq!(info.contributions().len(), 6 + 1 + markers::kinds().len() + 2 + 1);
        assert!(info.contributions().contains(&"config-parser:msinfo".to_string()));
        assert_eq!(info.contributions().last().unwrap(), "settings");
        assert!(info.contributions().contains(&"marker:app-crash".to_string()));
        assert!(info.contributions().contains(&"annotator:windows.reboots".to_string()));
        let section = info.settings.unwrap();
        assert_eq!((section.id.as_str(), section.fields.len()), ("net.decryptology.windows", 1));
        assert!(info.permissions.iter().any(|p| p.permission == Permission::RunPrograms));
        assert_eq!(WindowsPlugin.name(), "Windows diagnostics");
        assert_eq!(WindowsPlugin.version(), env!("CARGO_PKG_VERSION"));
    }
}
