//! User settings of the tools plugin: how long one `logman` call (import,
//! start, stop, query a Data Collector Set) may take before it is stopped.

use crate::perfmon_control::DEFAULT_TIMEOUT;
use anyhow::{anyhow, Result};
use imt_plugin_api::settings::{Configurable, FieldKind, SectionOwner, SettingField, SettingsSection, ValidValues};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

const DEFAULT_MS: u64 = DEFAULT_TIMEOUT.as_millis() as u64;

/// Current values, shared by the settings object and the tools.
pub(crate) struct Knobs {
    logman_timeout_ms: AtomicU64,
}

impl Default for Knobs {
    fn default() -> Self {
        Knobs {
            logman_timeout_ms: AtomicU64::new(DEFAULT_MS),
        }
    }
}

impl Knobs {
    pub fn logman_timeout(&self) -> Duration {
        Duration::from_millis(self.logman_timeout_ms.load(Ordering::Relaxed))
    }
}

pub(crate) struct ToolsSettings(pub Arc<Knobs>);

impl Configurable for ToolsSettings {
    fn settings_section(&self) -> SettingsSection {
        SettingsSection::new("tools", "System tools", SectionOwner::Core)
            .describe("External Windows tools used to control Performance Monitor Data Collector Sets.")
            .field(
                SettingField::new(
                    "logmanTimeout",
                    "logman timeout",
                    FieldKind::Duration {
                        min_ms: 5_000,
                        max_ms: 600_000,
                    },
                    DEFAULT_MS,
                )
                .describe("How long one logman call (import, start, stop, query a Data Collector Set) may take.")
                .as_advanced(),
            )
    }

    fn apply_settings(&self, values: &ValidValues) -> Result<()> {
        let timeout = values
            .duration("logmanTimeout")
            .ok_or_else(|| anyhow!("missing setting logmanTimeout"))?;
        self.0.logman_timeout_ms.store(timeout.as_millis() as u64, Ordering::Relaxed);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_plugin_api::settings::validate;
    use serde_json::json;

    #[test]
    fn schema_is_valid_and_values_apply() {
        let knobs = Arc::new(Knobs::default());
        let s = ToolsSettings(knobs.clone());
        let section = s.settings_section();
        assert_eq!(section.check(), Ok(()));
        assert_eq!(knobs.logman_timeout(), DEFAULT_TIMEOUT);
        s.apply_settings(&validate(&section, &json!({ "logmanTimeout": 30_000 })).unwrap())
            .unwrap();
        assert_eq!(knobs.logman_timeout(), Duration::from_secs(30));
        let other = validate(&SettingsSection::new("x", "X", SectionOwner::Core), &json!({})).unwrap();
        assert!(s.apply_settings(&other).unwrap_err().to_string().contains("logmanTimeout"));
    }
}
