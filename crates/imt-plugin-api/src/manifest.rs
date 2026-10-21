//! What a plugin is and contributes, as the host learns it on start (it crosses as JSON: it is descriptive,
//! small and sent once).

use crate::markers::PluginMarkerKind;
use crate::settings::SettingsSection;
use crate::tools::ToolInfo;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Version of the plugin contract. A plugin built against another version is refused with a message.
pub const CONTRACT: u32 = 1;
/// Largest memory a plugin may ask for (4 GiB).
pub const MAX_MEMORY_LIMIT: u64 = 4 << 30;

/// A right a plugin needs beyond parsing what the host hands it. Each widens its sandbox (ADR 0017 §6) and is
/// shown in the Plugins settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Permission {
    /// Receive the full paths of files the user opened and read them directly (e.g. to run `relog` on a `.blg`).
    OpenedFilePaths,
    /// Start other programs (`logman`, `wevtutil`, `relog`).
    RunPrograms,
    /// Write files into folders the user chose (collector output).
    WriteFolders,
    /// Open network connections.
    Network,
}

impl Permission {
    /// What the user is asked to allow.
    pub fn describe(self) -> &'static str {
        match self {
            Permission::OpenedFilePaths => "read the files you open directly",
            Permission::RunPrograms => "run programs",
            Permission::WriteFolders => "write into folders you choose",
            Permission::Network => "connect to the network",
        }
    }
}

/// A permission and why the plugin needs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRequest {
    pub permission: Permission,
    pub reason: String,
}

/// An importer a plugin contributes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImporterCap {
    pub id: Arc<str>,
    /// Receives raw bytes (else text IMT decoded).
    pub binary: bool,
    /// Implements live tail.
    pub tail: bool,
    /// A record may span several lines.
    pub multiline: bool,
    /// Fields that identify who wrote an event, widest first (ADR 0020; see
    /// [`crate::importer::TextImporter::identity_fields`]).
    #[serde(default)]
    pub identity_fields: Vec<String>,
}

/// Most identity fields an importer may declare.
pub const MAX_IDENTITY_FIELDS: usize = 8;

/// A configuration format a plugin contributes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigParserCap {
    pub id: Arc<str>,
    pub name: String,
    pub extensions: Vec<String>,
}

/// A profile (or heap snapshot) format a plugin contributes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileImporterCap {
    pub id: Arc<str>,
    /// Produces heap snapshots (else profiles).
    pub heap: bool,
}

/// A plugin's identity and contributions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    /// Reverse-DNS id (`net.decryptology.registry`).
    pub id: Arc<str>,
    pub name: Arc<str>,
    /// Semver.
    pub version: Arc<str>,
    /// [`CONTRACT`] it was built against.
    pub contract: u32,
    pub permissions: Vec<PermissionRequest>,
    /// Memory it needs at most (bytes); the host caps it at [`MAX_MEMORY_LIMIT`]. `None`: the default.
    pub memory_limit: Option<u64>,
    pub importers: Vec<ImporterCap>,
    pub config_parsers: Vec<ConfigParserCap>,
    pub marker_kinds: Vec<PluginMarkerKind>,
    pub annotators: Vec<Arc<str>>,
    pub transforms: Vec<Arc<str>>,
    pub settings: Option<SettingsSection>,
    pub tools: Vec<ToolInfo>,
    pub profile_importers: Vec<ProfileImporterCap>,
}

impl Manifest {
    /// What the plugin contributes, as the Plugins view lists it (`importer:evtx`, `tool:perfmon.sets`, ...).
    pub fn contributions(&self) -> Vec<String> {
        let mut out = Vec::new();
        out.extend(self.importers.iter().map(|i| format!("importer:{}", i.id)));
        out.extend(self.config_parsers.iter().map(|p| format!("config-parser:{}", p.id)));
        out.extend(self.marker_kinds.iter().map(|k| format!("marker:{}", k.kind)));
        out.extend(self.annotators.iter().map(|a| format!("annotator:{a}")));
        out.extend(self.transforms.iter().map(|t| format!("transform:{t}")));
        out.extend(self.tools.iter().map(|t| format!("tool:{}", t.id)));
        out.extend(self.profile_importers.iter().map(|p| format!("profile-importer:{}", p.id)));
        if self.settings.is_some() {
            out.push("settings".into());
        }
        out
    }

    /// Whether the plugin asks for `p`.
    pub fn asks_for(&self, p: Permission) -> bool {
        self.permissions.iter().any(|r| r.permission == p)
    }

    /// Whether any tool operation must run elevated.
    pub fn has_elevated_ops(&self) -> bool {
        self.tools.iter().any(|t| t.ops.iter().any(|o| o.elevated))
    }

    /// Checks what the host relies on: the contract version, ids, unique contributions.
    pub fn check(&self) -> Result<(), String> {
        if self.contract != CONTRACT {
            return Err(format!(
                "plugin \"{}\" was built for plugin contract {}, this IMT speaks {CONTRACT}",
                self.id, self.contract
            ));
        }
        if self.id.is_empty() || self.id.len() > 128 || !self.id.chars().all(|c| c.is_ascii_alphanumeric() || ".-_".contains(c)) {
            return Err(format!("plugin id \"{}\" is not valid", self.id));
        }
        let mut seen = std::collections::HashSet::new();
        for c in self.contributions() {
            if !seen.insert(c.clone()) {
                return Err(format!("plugin \"{}\" contributes \"{c}\" twice", self.id));
            }
        }
        for i in &self.importers {
            let bad = |f: &String| f.is_empty() || f.len() > 64 || f.chars().any(char::is_control);
            if i.identity_fields.len() > MAX_IDENTITY_FIELDS || i.identity_fields.iter().any(bad) {
                return Err(format!(
                    "importer \"{}\": declare at most {MAX_IDENTITY_FIELDS} identity fields of 1–64 characters",
                    i.id
                ));
            }
        }
        for t in &self.tools {
            t.check()?;
        }
        if let Some(s) = &self.settings {
            s.check()?;
        }
        if let Some(k) = self.marker_kinds.iter().find(|k| !k.valid_id()) {
            return Err(format!(
                "marker kind \"{}\": use 1–64 characters from A–Z, a–z, 0–9, '.', '_' and '-'",
                k.kind
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::OpSpec;

    pub(crate) fn sample() -> Manifest {
        Manifest {
            id: "net.example.toy".into(),
            name: "Toy".into(),
            version: "1.0.0".into(),
            contract: CONTRACT,
            permissions: vec![PermissionRequest {
                permission: Permission::RunPrograms,
                reason: "relog".into(),
            }],
            memory_limit: None,
            importers: vec![ImporterCap {
                id: "toy".into(),
                binary: false,
                tail: true,
                multiline: false,
                identity_fields: vec!["Host".into()],
            }],
            config_parsers: Vec::new(),
            marker_kinds: vec![PluginMarkerKind::new("boom", "Boom")],
            annotators: vec!["toy.spans".into()],
            transforms: Vec::new(),
            settings: None,
            tools: vec![ToolInfo::new("toy.tool", "Toy").op(OpSpec::new("go", "Go").elevated())],
            profile_importers: Vec::new(),
        }
    }

    #[test]
    fn manifests_check_and_list_their_contributions() {
        let m = sample();
        m.check().unwrap();
        assert_eq!(
            m.contributions(),
            ["importer:toy", "marker:boom", "annotator:toy.spans", "tool:toy.tool"]
        );
        assert!(m.asks_for(Permission::RunPrograms) && !m.asks_for(Permission::Network));
        assert!(m.has_elevated_ops());
        let back: Manifest = serde_json::from_value(serde_json::to_value(&m).unwrap()).unwrap();
        assert_eq!(back, m);
        let mut old = m.clone();
        old.contract = 0;
        assert!(old.check().unwrap_err().contains("contract 0"));
        let mut identity = m.clone();
        identity.importers[0].identity_fields = vec!["".into()];
        assert!(identity.check().unwrap_err().contains("identity fields"));
        identity.importers[0].identity_fields = vec!["F".into(); MAX_IDENTITY_FIELDS + 1];
        assert!(identity.check().is_err());
        // Older manifests without the field read as none.
        let mut json = serde_json::to_value(&m).unwrap();
        json["importers"][0].as_object_mut().unwrap().remove("identityFields");
        assert!(serde_json::from_value::<Manifest>(json).unwrap().importers[0]
            .identity_fields
            .is_empty());
        let mut twice = m.clone();
        twice.annotators.push("toy.spans".into());
        assert!(twice.check().unwrap_err().contains("twice"));
        let mut bad = m;
        bad.id = "no spaces allowed".into();
        assert!(bad.check().is_err());
        assert_eq!(Permission::Network.describe(), "connect to the network");
    }
}
