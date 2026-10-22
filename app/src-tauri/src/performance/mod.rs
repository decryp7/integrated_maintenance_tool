//! Performance settings: Low memory mode (off by default), kept in `performance.json` and read when the store
//! worker starts (`crate::app_state`); the settings section says a change applies after a restart.

use crate::builtin_sections::PERFORMANCE_SECTION;
use crate::settings::ExternalValues;
use imt_core::settings::{Configurable, FieldKind, SectionOwner, SettingField, SettingsSection, ValidValues};
use imt_store_client::MemoryMode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const FILE: &str = "performance.json";
/// The format of `performance.json`.
const VERSION: u32 = 1;

/// What `performance.json` holds (missing or damaged: the defaults).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PerformanceSettings {
    /// Low memory mode: the store worker gives freed memory back at once and uses fewer threads and smaller caches.
    pub low_memory: bool,
}

impl PerformanceSettings {
    /// The store worker's memory mode.
    pub fn memory_mode(self) -> MemoryMode {
        if self.low_memory {
            MemoryMode::Low
        } else {
            MemoryMode::Normal
        }
    }
}

pub fn load(dir: Option<&Path>) -> PerformanceSettings {
    crate::config_store::ConfigStore::new(dir.map(Path::to_path_buf)).load(FILE, VERSION)
}

pub fn save(dir: &Path, s: &PerformanceSettings) -> anyhow::Result<()> {
    Ok(crate::config_store::ConfigStore::new(Some(dir.to_path_buf())).save(FILE, VERSION, s)?)
}

/// The Performance section (Advanced).
pub struct PerformanceSection {
    pub dir: Option<PathBuf>,
}

impl Configurable for PerformanceSection {
    fn settings_section(&self) -> SettingsSection {
        SettingsSection::new(PERFORMANCE_SECTION, "Performance", SectionOwner::App)
            .describe("How IMT uses this computer's memory.")
            .field(
                SettingField::new("lowMemory", "Low memory mode", FieldKind::Bool, false)
                    .describe(
                        "Keeps the database worker's memory low (about 200 MB importing instead of up to 700 MB); \
                         imports take about half as long again.",
                    )
                    .needs_restart(),
            )
    }

    fn apply_settings(&self, values: &ValidValues) -> anyhow::Result<()> {
        let dir = self
            .dir
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("there is no configuration folder"))?;
        let s = PerformanceSettings {
            low_memory: values.bool("lowMemory").unwrap_or(false),
        };
        save(dir, &s)
    }
}

impl ExternalValues for PerformanceSection {
    fn load(&self) -> Value {
        json!({ "lowMemory": load(self.dir.as_deref()).low_memory })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("imt-performance-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn low_memory_mode_is_off_by_default_and_survives_a_damaged_file() {
        assert_eq!(load(None), PerformanceSettings::default());
        assert_eq!(load(None).memory_mode(), MemoryMode::Normal);
        let d = temp("damaged");
        std::fs::write(d.join(FILE), "{ not json").unwrap();
        assert!(!load(Some(&d)).low_memory, "a damaged file falls back to the defaults");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_section_saves_what_the_worker_reads() {
        let d = temp("section");
        let section = PerformanceSection { dir: Some(d.clone()) };
        let s = section.settings_section();
        let field = s.fields.iter().find(|f| f.key == "lowMemory").expect("the field");
        assert!(field.restart_required, "applies when the worker starts");
        let (valid, _) = imt_core::settings::sanitize(&s, &json!({"lowMemory": true}));
        section.apply_settings(&valid).unwrap();
        assert_eq!(load(Some(&d)).memory_mode(), MemoryMode::Low);
        assert_eq!(section.load(), json!({"lowMemory": true}));
        let _ = std::fs::remove_dir_all(&d);
    }
}
