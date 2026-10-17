//! User settings of the Windows plugin: how long `relog` (for `.blg` files)
//! may take before it is stopped. (The `logman` timeout of Data Collector
//! Sets is a setting of the tools plugin.)

use crate::command::DEFAULT_TIMEOUT;
use anyhow::{anyhow, Result};
use imt_core::settings::{Configurable, FieldKind, SectionOwner, SettingField, SettingsSection, ValidValues};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const DEFAULT_MS: u64 = DEFAULT_TIMEOUT.as_millis() as u64;
static RELOG_TIMEOUT_MS: AtomicU64 = AtomicU64::new(DEFAULT_MS);

/// Time allowed for one `relog` conversion.
pub fn relog_timeout() -> Duration {
    Duration::from_millis(RELOG_TIMEOUT_MS.load(Ordering::Relaxed))
}

/// The plugin's [`Configurable`]. The values are process-wide, shared with the
/// importers.
pub struct WindowsSettings;

impl Configurable for WindowsSettings {
    fn settings_section(&self) -> SettingsSection {
        SettingsSection::new("windows", "Windows diagnostics", SectionOwner::Core)
            .describe("External Windows tools used to read Performance Monitor logs.")
            .field(
                SettingField::new(
                    "relogTimeout",
                    "relog timeout",
                    FieldKind::Duration {
                        min_ms: 5_000,
                        max_ms: 3_600_000,
                    },
                    DEFAULT_MS,
                )
                .describe("How long converting one .blg file with relog may take before it is stopped. Raise it for very large logs."),
            )
    }

    fn apply_settings(&self, values: &ValidValues) -> Result<()> {
        let ms = |key: &str| {
            values
                .duration(key)
                .map(|d| d.as_millis() as u64)
                .ok_or_else(|| anyhow!("missing setting {key}"))
        };
        RELOG_TIMEOUT_MS.store(ms("relogTimeout")?, Ordering::Relaxed);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_core::settings::validate;
    use serde_json::json;

    #[test]
    fn schema_is_valid_and_values_apply() {
        let section = WindowsSettings.settings_section();
        assert_eq!(section.check(), Ok(()));
        assert_eq!(section.defaults().duration("relogTimeout"), Some(DEFAULT_TIMEOUT));
        let values = validate(&section, &json!({ "relogTimeout": 600_000 })).unwrap();
        WindowsSettings.apply_settings(&values).unwrap();
        assert_eq!(relog_timeout(), Duration::from_secs(600));
        let too_short = validate(&section, &json!({ "relogTimeout": 10 })).unwrap_err();
        assert_eq!(too_short[0].message, "Must be between 5 s and 1 h.");
        WindowsSettings.apply_settings(&section.defaults()).unwrap();
        assert_eq!(relog_timeout(), DEFAULT_TIMEOUT);
    }

    #[test]
    fn values_of_another_schema_are_refused() {
        let other = validate(&SettingsSection::new("x", "X", SectionOwner::Core), &json!({})).unwrap();
        assert!(WindowsSettings
            .apply_settings(&other)
            .unwrap_err()
            .to_string()
            .contains("relogTimeout"));
    }
}
