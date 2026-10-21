//! The "Plugin trust" settings section: developer mode and the publisher keys the user trusts. Values live in
//! `settings.json`; every change is parsed into [`TrustSettings`] here (a key that is not valid is refused with
//! the line it is on), and installs read them from [`super::Installed::trust`].

use imt_core::settings::{Configurable, FieldKind, SectionOwner, SettingField, SettingsSection, ValidValues};
use imt_plugin_package::trust::{TrustSettings, FIRST_PARTY_KEYS, MAX_TRUSTED_KEYS};
use std::sync::{Arc, RwLock};

/// The section's id.
pub const TRUST_SECTION: &str = "plugin-trust";

/// The section, writing into the shared trust settings.
pub struct TrustSection {
    pub trust: Arc<RwLock<TrustSettings>>,
}

/// Parses the trusted-key lines (`Name = imt-plugin-public-key ed25519 <hex>`).
pub fn parse_keys(lines: &[String], developer_mode: bool) -> Result<TrustSettings, String> {
    let mut t = TrustSettings {
        developer_mode,
        keys: Vec::new(),
    };
    for (i, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let Some((name, key)) = line.rsplit_once('=') else {
            return Err(format!("trusted key {}: write it as \"Publisher = imt-plugin-public-key ed25519 …\"", i + 1));
        };
        t.add_key(name, key).map_err(|e| format!("trusted key {}: {e}", i + 1))?;
    }
    Ok(t)
}

impl Configurable for TrustSection {
    fn settings_section(&self) -> SettingsSection {
        let first_party = FIRST_PARTY_KEYS.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ");
        SettingsSection::new(TRUST_SECTION, "Plugin trust", SectionOwner::App)
            .describe(format!(
                "Which third-party plugins IMT installs. A package signed with a trusted publisher key installs normally; {first_party} is always trusted. \
                 Unsigned packages and unknown keys install only in developer mode."
            ))
            .field(
                SettingField::new("developerMode", "Developer mode", FieldKind::Bool, false).describe(
                    "Allow installing unsigned plugins and plugins signed with keys you have not trusted, after a warning. \
                     Turn it on only while you develop or test a plugin.",
                ),
            )
            .field(
                SettingField::new(
                    "trustedKeys",
                    "Trusted publisher keys",
                    FieldKind::StringList {
                        max_items: MAX_TRUSTED_KEYS,
                    },
                    serde_json::json!([]),
                )
                .describe("One per line: the publisher's name, \"=\", and the public key they give you (imt-plugin-public-key ed25519 …)."),
            )
    }

    fn apply_settings(&self, values: &ValidValues) -> anyhow::Result<()> {
        let t = parse_keys(
            &values.strings("trustedKeys").unwrap_or_default(),
            values.bool("developerMode").unwrap_or(false),
        )
        .map_err(anyhow::Error::msg)?;
        *crate::sync::write(&self.trust) = t;
        Ok(())
    }
}
