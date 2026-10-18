//! The "Updates" settings section: automatic checks (off by default) and the channel. Its description
//! says exactly what a check sends.

use imt_core::settings::{Configurable, EnumOption, FieldKind, SectionOwner, SettingField, SettingsSection, ValidValues};
use std::sync::{Arc, RwLock};

pub const SECTION: &str = "updates";

/// Release channels. Only stable releases are published.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Channel {
    #[default]
    Stable,
}

impl Channel {
    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
        }
    }
    pub fn parse(s: &str) -> Option<Channel> {
        (s == "stable").then_some(Channel::Stable)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UpdateSettings {
    pub auto_check: bool,
    pub channel: Channel,
}

impl UpdateSettings {
    pub fn from_values(values: &ValidValues) -> UpdateSettings {
        UpdateSettings {
            auto_check: values.bool("autoCheck").unwrap_or(false),
            channel: values.text("channel").and_then(Channel::parse).unwrap_or_default(),
        }
    }
}

/// The values in effect, shared with the automatic check.
#[derive(Clone, Default)]
pub struct SharedSettings(Arc<RwLock<UpdateSettings>>);

impl SharedSettings {
    pub fn get(&self) -> UpdateSettings {
        *self.0.read().unwrap_or_else(|p| p.into_inner())
    }
    pub fn set(&self, s: UpdateSettings) {
        *self.0.write().unwrap_or_else(|p| p.into_inner()) = s;
    }
}

pub struct UpdatesSection {
    pub settings: SharedSettings,
    /// The host checked (`None`: this build has no update key).
    pub endpoint_host: Option<String>,
}

/// What a check sends, for the setting's description.
pub fn privacy_note(host: &str) -> String {
    format!(
        "A check is one HTTPS request to {host} for the latest release's latest.json (version, notes, download links \
         and signatures). It sends no identifier, usage data or file content: the server sees your IP address and the \
         user agent of IMT's updater. A download goes to the same releases, and is installed only when its signature \
         matches the IMT signing key built into this version."
    )
}

impl Configurable for UpdatesSection {
    fn settings_section(&self) -> SettingsSection {
        let description = match &self.endpoint_host {
            Some(host) => format!(
                "{} Help \u{25b8} Check for updates\u{2026} works whether or not automatic checks are on.",
                privacy_note(host)
            ),
            None => "Updates are not available in this build: it was built without an update signing key, so it cannot \
                     verify updates. Download new versions from the releases page."
                .to_string(),
        };
        SettingsSection::new(SECTION, "Updates", SectionOwner::App)
            .describe(description)
            .field(
                SettingField::new("autoCheck", "Check for updates automatically", FieldKind::Bool, false)
                    .describe("Off by default. When on, IMT checks a minute after it starts and then once a day, and tells you when a new version is available. Nothing is downloaded without asking."),
            )
            .field(
                SettingField::new(
                    "channel",
                    "Channel",
                    FieldKind::Enum {
                        options: vec![EnumOption::new(Channel::Stable.as_str(), "Stable")],
                    },
                    Channel::default().as_str(),
                )
                .describe("Stable: published releases only."),
            )
    }

    fn apply_settings(&self, values: &ValidValues) -> anyhow::Result<()> {
        self.settings.set(UpdateSettings::from_values(values));
        Ok(())
    }
}
