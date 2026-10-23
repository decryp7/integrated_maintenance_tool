//! Typed settings: the schema a settings screen is generated from, the
//! validation of untrusted values against it, and the [`Configurable`]
//! extension point through which plugins and app modules expose knobs.
//!
//! A [`SettingsSection`] lists [`SettingField`]s, each with a [`FieldKind`]
//! (bool, ranged int/float, text, enum, path, duration, size, string list) and
//! a default. [`validate`] turns a JSON object from the UI or a settings file
//! into [`ValidValues`] (parse, don't validate): unknown keys are dropped,
//! missing keys take their default, and every type and range problem is
//! reported per field with a message the user can act on.
//!
//! Plugins opt in with [`crate::Plugin::configurable`]. The registry wraps the
//! returned object in a [`GuardedConfigurable`]: the section is described once
//! at install (so a disabled or quarantined plugin still shows it, read-only)
//! and `apply_settings` runs under crash containment like every other plugin
//! contribution.

use crate::guard::Guard;
use crate::plugin::{PluginInfo, PluginStatus};
use anyhow::{anyhow, Result};
use std::sync::Arc;

// The schema, its validation and `Configurable` are part of the plugin contract (ADR 0017).
pub use debrief_plugin_api::settings::{
    format_bytes, format_ms, sanitize, validate, validate_field, Configurable, EnumOption, FieldError, FieldKind, SectionOwner,
    SettingField, SettingsSection, ValidValues, MAX_FIELDS, MAX_ITEM_LEN, MAX_LIST_ITEMS, MAX_PATH_LEN, MAX_TEXT_LEN,
};

/// A plugin's [`Configurable`] under crash containment. The section is
/// described once, when the plugin is installed, so it can still be shown
/// (read-only) when the plugin is disabled or quarantined without calling
/// plugin code again. Its id and owner are forced to the plugin's.
pub struct GuardedConfigurable {
    inner: Arc<dyn Configurable>,
    guard: Guard,
    section: SettingsSection,
}

impl GuardedConfigurable {
    /// Asks `plugin` for its settings, if it has any. Panics and invalid
    /// schemas are contained: the plugin gets a section without fields that
    /// says why, and a panic counts towards its quarantine.
    pub fn describe(plugin: &dyn crate::Plugin, guard: Guard) -> Option<Arc<GuardedConfigurable>> {
        let described = guard
            .run("configurable", || Ok(plugin.configurable()))
            .and_then(|inner| match inner {
                None => Ok(None),
                Some(inner) => guard.run("settings_section", || {
                    let s = inner.settings_section();
                    s.check().map_err(|e| anyhow!("invalid settings schema: {e}"))?;
                    Ok(Some((inner, s)))
                }),
            });
        let (inner, section) = match described {
            Ok(None) => return None,
            Ok(Some((inner, mut s))) => {
                s.id = plugin.id().to_string();
                s.owner = SectionOwner::Plugin { id: plugin.id().into() };
                if s.title.trim().is_empty() {
                    s.title = plugin.name().into();
                }
                (inner, s)
            }
            // Values validated against an unknown schema mean nothing to the plugin: never apply them.
            Err(e) => (Arc::new(Unavailable) as Arc<dyn Configurable>, unavailable(plugin, &e)),
        };
        Some(Arc::new(GuardedConfigurable { inner, guard, section }))
    }
}

impl Configurable for GuardedConfigurable {
    fn settings_section(&self) -> SettingsSection {
        self.section.clone()
    }

    fn apply_settings(&self, values: &ValidValues) -> Result<()> {
        self.guard.run("apply_settings", || self.inner.apply_settings(values))
    }
}

/// Stands in for a plugin whose settings could not be described.
struct Unavailable;

impl Configurable for Unavailable {
    fn settings_section(&self) -> SettingsSection {
        SettingsSection::new("unavailable", "Unavailable", SectionOwner::Core)
    }
    fn apply_settings(&self, _values: &ValidValues) -> Result<()> {
        Ok(())
    }
}

fn unavailable(plugin: &dyn crate::Plugin, error: &anyhow::Error) -> SettingsSection {
    SettingsSection::new(plugin.id(), plugin.name(), SectionOwner::Plugin { id: plugin.id().into() })
        .describe(format!("Settings are unavailable: {error:#}"))
}

/// A plugin's settings with its current status, as listed by
/// [`crate::PluginRegistry::plugin_settings`].
#[derive(Clone)]
pub struct PluginSettings {
    pub plugin: PluginInfo,
    pub configurable: Arc<dyn Configurable>,
}

impl PluginSettings {
    /// Only an active plugin's settings can be changed.
    pub fn read_only(&self) -> bool {
        self.plugin.status != PluginStatus::Active
    }

    /// Why the section is read-only.
    pub fn note(&self) -> Option<&'static str> {
        match self.plugin.status {
            PluginStatus::Active => None,
            PluginStatus::Disabled => Some("This plugin is disabled. Enable it under Plugins and restart to change its settings."),
            PluginStatus::Quarantined => {
                Some("This plugin was stopped after repeated crashes. Re-enable it in the Plugins panel to change its settings.")
            }
            PluginStatus::Inactive => {
                Some("This plugin is inactive (see the Plugins panel for why). Its settings can be changed once it runs.")
            }
        }
    }

    /// Applies values to an active plugin; refuses for a read-only one.
    pub fn apply(&self, values: &ValidValues) -> Result<()> {
        match self.note() {
            Some(note) => Err(anyhow!("{note}")),
            None => self.configurable.apply_settings(values),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_stand_in_is_inert() {
        assert_eq!(Unavailable.settings_section().id, "unavailable");
        assert!(Unavailable.apply_settings(&ValidValues::default()).is_ok());
    }
}
