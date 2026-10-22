//! The plugin's settings (*Settings ▸ Native profiling*): the default symbol path and whether PDBs of Windows
//! modules may be downloaded from the Microsoft symbol server (off by default, ADR 0022 §4). The capture dialog
//! starts from these values (`defaults`) and sends them with each capture.

use anyhow::{anyhow, Result};
use debrief_plugin_api::settings::{Configurable, FieldKind, SectionOwner, SettingField, SettingsSection, ValidValues};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// The section id.
pub const SECTION: &str = "native-profiling";

/// Current values, shared by the settings object and the tools.
#[derive(Default)]
pub struct Knobs {
    symbol_path: Mutex<String>,
    symbol_server: AtomicBool,
}

impl Knobs {
    pub fn symbol_path(&self) -> String {
        self.symbol_path.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn symbol_server(&self) -> bool {
        self.symbol_server.load(Ordering::Relaxed)
    }
}

pub struct NativeSettings(pub Arc<Knobs>);

impl Configurable for NativeSettings {
    fn settings_section(&self) -> SettingsSection {
        SettingsSection::new(SECTION, "Native profiling", SectionOwner::Core)
            .describe("Where native (C/C++) captures find symbols (PDB files).")
            .field(
                SettingField::new(
                    "symbolPath",
                    "Symbol path",
                    FieldKind::Text {
                        max_len: crate::request::MAX_SYMBOL_PATH,
                        pattern: None,
                    },
                    "",
                )
                .describe("Folders with PDB files, separated by semicolons (C:\\symbols;D:\\build\\pdb). The folder of each module is searched too."),
            )
            .field(
                SettingField::new("symbolServer", "Download symbols from Microsoft", FieldKind::Bool, false).describe(
                    "Download missing PDBs of Windows modules from the Microsoft symbol server (msdl.microsoft.com) into Debrief's symbol cache. Off: no symbol is ever downloaded.",
                ),
            )
    }

    fn apply_settings(&self, values: &ValidValues) -> Result<()> {
        let path = values.text("symbolPath").ok_or_else(|| anyhow!("missing setting symbolPath"))?;
        crate::request::SymbolOptions::parse(path, false).map_err(|e| anyhow!(e))?;
        let server = values.bool("symbolServer").ok_or_else(|| anyhow!("missing setting symbolServer"))?;
        *self.0.symbol_path.lock().unwrap_or_else(|p| p.into_inner()) = path.to_string();
        self.0.symbol_server.store(server, Ordering::Relaxed);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use debrief_plugin_api::settings::validate;
    use serde_json::json;

    #[test]
    fn schema_is_valid_values_apply_and_bad_paths_are_refused() {
        let knobs = Arc::new(Knobs::default());
        let s = NativeSettings(knobs.clone());
        let section = s.settings_section();
        assert_eq!(section.check(), Ok(()));
        assert!(
            !knobs.symbol_server() && knobs.symbol_path().is_empty(),
            "downloads are off by default"
        );
        s.apply_settings(&validate(&section, &json!({ "symbolPath": "C:\\syms", "symbolServer": true })).unwrap())
            .unwrap();
        assert_eq!(knobs.symbol_path(), "C:\\syms");
        assert!(knobs.symbol_server());
        let bad = validate(&section, &json!({ "symbolPath": "relative", "symbolServer": false })).unwrap();
        assert!(s.apply_settings(&bad).is_err());
        assert_eq!(knobs.symbol_path(), "C:\\syms", "unchanged after a refused value");
        let other = validate(&SettingsSection::new("x", "X", SectionOwner::Core), &json!({})).unwrap();
        assert!(s.apply_settings(&other).is_err());
    }
}
