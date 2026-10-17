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
//! * **Markers and annotators**: [`markers::kinds`] marks crashes, hangs,
//!   unexpected shutdowns, boots, disk errors, updates and logon failures in
//!   event logs; [`annotators::RebootAnnotator`] and
//!   [`annotators::ServiceAnnotator`] add downtime and service uptime spans.
//!
//! CBS/DISM text logs (`2026-10-14 10:00:00, Info   CBS   message`) need no
//! importer: the built-in text importer reads them with a profile regex.
//!
//! ```
//! let mut registry = imt_core::PluginRegistry::new();
//! registry.install(&imt_plugin_windows::WindowsPlugin).unwrap();
//! ```

pub mod annotators;
mod blg;
pub mod command;
mod evtx_import;
pub mod markers;
mod nfo;
pub mod perfmon;
pub mod settings;

pub use blg::BlgImporter;
pub use evtx_import::{is_evtx, read_evtx, EvtxImporter};
pub use nfo::{NfoConfigParser, NfoImporter};
pub use perfmon::PerfmonCsvImporter;

use anyhow::Result;
use imt_core::plugin::Manifest;
use imt_core::settings::Configurable;
use imt_core::{Plugin, PluginRegistry};
use std::sync::Arc;

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

    /// No dependencies; provides no services: PerfMon data reaches IMT as
    /// imported records (`.csv`, `.blg`), not as live counters.
    fn manifest(&self) -> Manifest {
        Manifest::new(self.id(), self.version())
    }

    fn register(&self, registry: &mut PluginRegistry) -> Result<()> {
        registry.add_binary_importer(Box::new(EvtxImporter));
        registry.add_importer(Box::new(NfoImporter));
        registry.contribute::<dyn imt_core::config::ConfigParser>(imt_core::config::PARSERS_POINT, Arc::new(NfoConfigParser));
        registry.add_importer(Box::new(PerfmonCsvImporter));
        registry.add_binary_importer(Box::<BlgImporter>::default());
        registry.add_marker_kinds(markers::kinds())?;
        registry.add_annotator(Arc::new(annotators::RebootAnnotator))?;
        registry.add_annotator(Arc::new(annotators::ServiceAnnotator))?;
        Ok(())
    }

    fn configurable(&self) -> Option<Arc<dyn Configurable>> {
        Some(Arc::new(settings::WindowsSettings))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_core::profile::Profile;
    use std::path::Path;

    #[test]
    fn plugin_registers_its_importers() {
        let mut registry = PluginRegistry::new();
        registry.install(&WindowsPlugin).unwrap();
        let info = &registry.plugins()[0];
        assert_eq!(info.id, "net.decryptology.windows");
        assert_eq!(info.contributes.len(), 4 + 1 + markers::kinds().len() + 2 + 1);
        assert!(info.contributes.contains(&"contribution:config.parsers".to_string()));
        assert_eq!(info.contributes.last().unwrap(), "settings");
        assert!(info.contributes.contains(&"marker:app-crash".to_string()));
        assert!(info.contributes.contains(&"annotator:windows.reboots".to_string()));
        let section = registry.plugin_settings()[0].configurable.settings_section();
        assert_eq!((section.id.as_str(), section.fields.len()), ("net.decryptology.windows", 1));
        assert_eq!(WindowsPlugin.name(), "Windows diagnostics");
        assert_eq!(WindowsPlugin.version(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn workspace_opens_the_fixtures() {
        let mut registry = PluginRegistry::new();
        registry.install(&WindowsPlugin).unwrap();
        let mut ws = imt_core::Workspace::with_registry(Profile::default_profile(), registry).unwrap();
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        ws.open(&[dir]).unwrap();
        let s = ws.summary();
        let mut ids: Vec<&str> = s.sources.iter().map(|x| x.importer.as_str()).collect();
        ids.sort();
        assert_eq!(ids, vec!["evtx", "msinfo", "perfmon-csv"]);
        assert!(ws.records().iter().any(|r| r.fields.contains_key("EventId")));
        assert!(ws.records().iter().any(|r| r.fields.contains_key("Item")));
    }
}
