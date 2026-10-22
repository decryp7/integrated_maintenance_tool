//! `manifest.toml`: what a package is (id, name, version, publisher), what it needs (protocol version,
//! permissions) and what it carries (one program per target). Parsed once into a [`PackageManifest`] whose values
//! are all valid; nothing downstream re-checks them.
//!
//! ```toml
//! format = 1
//!
//! [plugin]
//! id = "com.example.kv-lines"
//! name = "Key-value lines"
//! version = "1.2.0"
//! publisher = "Example Ltd"
//! description = "Reads key=value logs."
//! protocol = "1.0"
//! binary = "kv-lines"
//! targets = ["x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu"]
//! key = "3f2a9c0d1e7b4a55"          # the signing key's id; omitted for an unsigned package
//!
//! [[permissions]]
//! permission = "network"
//! reason = "Looks up host names."
//! ```

use crate::PackageError;
use debrief_plugin_api::manifest::{Permission, PermissionRequest};
use debrief_plugin_api::ProtocolVersion;
use serde::{Deserialize, Serialize};

/// The manifest format this crate reads and writes.
pub const FORMAT: u32 = 1;
/// Largest `manifest.toml`.
pub const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
/// Most targets one package may carry.
pub const MAX_TARGETS: usize = 16;
/// Permissions a package may ask for. The others (`openedFilePaths`, `writeFolders`) hand a plugin paths of the
/// user's files, which third-party plugins never get: they receive files through the host (ADR 0027).
pub const PACKAGE_PERMISSIONS: [Permission; 2] = [Permission::Network, Permission::RunPrograms];

/// A valid `manifest.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageManifest {
    /// Reverse-DNS id, unique among plugins (`com.example.kv-lines`).
    pub id: String,
    /// Display name.
    pub name: String,
    /// The plugin's version.
    pub version: semver::Version,
    /// Who publishes it, as shown to the user.
    pub publisher: String,
    /// What it does, as shown to the user before installing.
    pub description: String,
    /// The plugin protocol its program was built for.
    pub protocol: ProtocolVersion,
    /// The program's file name without extension (`kv-lines`; `kv-lines.exe` on Windows targets).
    pub binary: String,
    /// The target triples it carries a program for, in the manifest's order.
    pub targets: Vec<String>,
    /// The id of the key that signs it ([`crate::KeyId`]); `None` for an unsigned package.
    pub key: Option<String>,
    /// The rights it asks for, with the publisher's reasons.
    pub permissions: Vec<PermissionRequest>,
}

/// The TOML shape of `manifest.toml` (parsed into [`PackageManifest`]).
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    format: u32,
    plugin: RawPlugin,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    permissions: Vec<PermissionRequest>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPlugin {
    id: String,
    name: String,
    version: String,
    publisher: String,
    #[serde(default)]
    description: String,
    protocol: String,
    binary: String,
    targets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key: Option<String>,
}

fn bad(m: impl Into<String>) -> PackageError {
    PackageError::InvalidManifest(m.into())
}

/// Whether `s` has 1–`max` characters, none of them control characters.
fn text_ok(s: &str, max: usize) -> bool {
    !s.trim().is_empty() && s.chars().count() <= max && !s.chars().any(char::is_control)
}

/// Whether `id` is a valid plugin id: 3–128 characters from `a–z A–Z 0–9 . - _`, with a dot, not starting or ending
/// with one.
pub fn valid_id(id: &str) -> bool {
    (3..=128).contains(&id.len())
        && id.contains('.')
        && !id.starts_with('.')
        && !id.ends_with('.')
        && id.chars().all(|c| c.is_ascii_alphanumeric() || ".-_".contains(c))
}

/// Whether `t` looks like a target triple (`x86_64-pc-windows-msvc`): 3–64 characters from `a–z 0–9 _ . -`, at
/// least two dashes.
pub fn valid_target(t: &str) -> bool {
    (3..=64).contains(&t.len())
        && t.matches('-').count() >= 2
        && t.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_.-".contains(c))
}

/// Whether `b` is a valid program name: 1–64 characters from `a–z A–Z 0–9 _ -`.
pub fn valid_binary(b: &str) -> bool {
    (1..=64).contains(&b.len()) && b.chars().all(|c| c.is_ascii_alphanumeric() || "_-".contains(c))
}

/// Whether a key id is 16 lowercase hexadecimal digits.
pub fn valid_key_id(k: &str) -> bool {
    k.len() == 16 && k.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

impl PackageManifest {
    /// Parses and validates `manifest.toml`.
    pub fn parse(text: &str) -> Result<PackageManifest, PackageError> {
        if text.len() as u64 > MAX_MANIFEST_BYTES {
            return Err(PackageError::TooLarge {
                what: "manifest.toml".into(),
                limit: MAX_MANIFEST_BYTES,
            });
        }
        let raw: Raw = toml::from_str(text).map_err(|e| bad(e.message().to_string()))?;
        if raw.format != FORMAT {
            return Err(bad(format!(
                "format {} is not supported (this Debrief reads format {FORMAT}); rebuild the package with a current cargo debrief-plugin",
                raw.format
            )));
        }
        let p = raw.plugin;
        if !valid_id(&p.id) {
            return Err(bad(format!(
                "plugin.id \"{}\" is not valid: use 3–128 characters from A–Z, a–z, 0–9, '.', '-', '_' in reverse-DNS style (com.example.name)",
                p.id
            )));
        }
        if !text_ok(&p.name, 80) {
            return Err(bad("plugin.name must have 1–80 characters"));
        }
        if !text_ok(&p.publisher, 120) {
            return Err(bad("plugin.publisher must have 1–120 characters"));
        }
        if p.description.chars().count() > 2000 || p.description.chars().any(|c| c.is_control() && c != '\n') {
            return Err(bad("plugin.description may have at most 2000 characters"));
        }
        let version = semver::Version::parse(&p.version).map_err(|e| bad(format!("plugin.version \"{}\": {e}", p.version)))?;
        let protocol: ProtocolVersion = p.protocol.parse().map_err(|e: String| bad(format!("plugin.protocol: {e}")))?;
        if !valid_binary(&p.binary) {
            return Err(bad(
                "plugin.binary must have 1–64 characters from A–Z, a–z, 0–9, '_' and '-' (no extension)",
            ));
        }
        if p.targets.is_empty() || p.targets.len() > MAX_TARGETS {
            return Err(bad(format!("plugin.targets must list 1–{MAX_TARGETS} target triples")));
        }
        let mut seen = std::collections::HashSet::new();
        for t in &p.targets {
            if !valid_target(t) {
                return Err(bad(format!("plugin.targets: \"{t}\" is not a target triple")));
            }
            if !seen.insert(t.as_str()) {
                return Err(bad(format!("plugin.targets lists \"{t}\" twice")));
            }
        }
        if let Some(k) = &p.key {
            if !valid_key_id(k) {
                return Err(bad(format!("plugin.key \"{k}\" is not a key id (16 hexadecimal digits)")));
            }
        }
        let mut perms = std::collections::HashSet::new();
        for r in &raw.permissions {
            if !PACKAGE_PERMISSIONS.contains(&r.permission) {
                return Err(bad(format!(
                    "permission {:?} is only available to Debrief's own plugins; third-party plugins receive the files the user opens through Debrief",
                    r.permission
                )));
            }
            if !perms.insert(r.permission) {
                return Err(bad(format!("permission {:?} is listed twice", r.permission)));
            }
            if !text_ok(&r.reason, 500) {
                return Err(bad(format!("permission {:?} needs a reason of 1–500 characters", r.permission)));
            }
        }
        Ok(PackageManifest {
            id: p.id,
            name: p.name,
            version,
            publisher: p.publisher,
            description: p.description,
            protocol,
            binary: p.binary,
            targets: p.targets,
            key: p.key,
            permissions: raw.permissions,
        })
    }

    /// The manifest as `manifest.toml` text ([`PackageManifest::parse`] reads it back unchanged).
    pub fn to_toml(&self) -> String {
        let raw = Raw {
            format: FORMAT,
            plugin: RawPlugin {
                id: self.id.clone(),
                name: self.name.clone(),
                version: self.version.to_string(),
                publisher: self.publisher.clone(),
                description: self.description.clone(),
                protocol: self.protocol.to_string(),
                binary: self.binary.clone(),
                targets: self.targets.clone(),
                key: self.key.clone(),
            },
            permissions: self.permissions.clone(),
        };
        // Plain strings, numbers and arrays of tables always serialise.
        toml::to_string(&raw).unwrap_or_default()
    }

    /// The program's path in the package for `target` (`bin/<target>/<binary>[.exe]`).
    pub fn binary_path(&self, target: &str) -> String {
        format!("bin/{target}/{}", self.binary_file(target))
    }

    /// The program's file name for `target` (with `.exe` on Windows targets).
    pub fn binary_file(&self, target: &str) -> String {
        if target.contains("windows") {
            format!("{}.exe", self.binary)
        } else {
            self.binary.clone()
        }
    }

    /// Whether it asks for `p`.
    pub fn asks_for(&self, p: Permission) -> bool {
        self.permissions.iter().any(|r| r.permission == p)
    }
}

/// The target triple of this build of Debrief: the program a package must carry to run here.
pub fn host_target() -> &'static str {
    if cfg!(all(windows, target_arch = "x86_64")) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(all(windows, target_arch = "aarch64")) {
        "aarch64-pc-windows-msvc"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "aarch64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin"
    } else {
        "unknown-unknown-unknown"
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const SAMPLE: &str = r#"format = 1

[plugin]
id = "com.example.kv-lines"
name = "Key-value lines"
version = "1.2.0"
publisher = "Example Ltd"
description = "Reads key=value logs."
protocol = "1.0"
binary = "kv-lines"
targets = ["x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu"]

[[permissions]]
permission = "network"
reason = "Looks up host names."
"#;

    #[test]
    fn a_valid_manifest_parses_and_roundtrips() {
        let m = PackageManifest::parse(SAMPLE).unwrap();
        assert_eq!(m.id, "com.example.kv-lines");
        assert_eq!(m.version, semver::Version::new(1, 2, 0));
        assert_eq!(m.protocol, ProtocolVersion::new(1, 0));
        assert!(m.asks_for(Permission::Network) && !m.asks_for(Permission::RunPrograms));
        assert_eq!(m.binary_path("x86_64-pc-windows-msvc"), "bin/x86_64-pc-windows-msvc/kv-lines.exe");
        assert_eq!(m.binary_path("x86_64-unknown-linux-gnu"), "bin/x86_64-unknown-linux-gnu/kv-lines");
        assert_eq!(PackageManifest::parse(&m.to_toml()).unwrap(), m);
        let mut signed = m.clone();
        signed.key = Some("0123456789abcdef".into());
        assert_eq!(PackageManifest::parse(&signed.to_toml()).unwrap(), signed);
        assert!(valid_target(host_target()));
    }

    #[test]
    fn invalid_manifests_say_what_to_fix() {
        let cases = [
            ("format = 1", "format = 2", "format 2"),
            ("\"com.example.kv-lines\"", "\"nodots\"", "plugin.id"),
            ("\"com.example.kv-lines\"", "\"../evil.x\"", "plugin.id"),
            ("name = \"Key-value lines\"", "name = \"\"", "plugin.name"),
            ("version = \"1.2.0\"", "version = \"one\"", "plugin.version"),
            ("protocol = \"1.0\"", "protocol = \"x\"", "plugin.protocol"),
            ("binary = \"kv-lines\"", "binary = \"../kv\"", "plugin.binary"),
            ("binary = \"kv-lines\"", "binary = \"kv.exe\"", "plugin.binary"),
            (
                "targets = [\"x86_64-pc-windows-msvc\", \"x86_64-unknown-linux-gnu\"]",
                "targets = []",
                "plugin.targets",
            ),
            (
                "targets = [\"x86_64-pc-windows-msvc\", \"x86_64-unknown-linux-gnu\"]",
                "targets = [\"x86_64-pc-windows-msvc\", \"x86_64-pc-windows-msvc\"]",
                "twice",
            ),
            (
                "targets = [\"x86_64-pc-windows-msvc\", \"x86_64-unknown-linux-gnu\"]",
                "targets = [\"../x/y\"]",
                "target triple",
            ),
            (
                "publisher = \"Example Ltd\"",
                "publisher = \"Example Ltd\"\nkey = \"XYZ\"",
                "plugin.key",
            ),
            (
                "permission = \"network\"",
                "permission = \"openedFilePaths\"",
                "only available to Debrief's own",
            ),
            (
                "permission = \"network\"",
                "permission = \"writeFolders\"",
                "only available to Debrief's own",
            ),
            ("reason = \"Looks up host names.\"", "reason = \"\"", "reason"),
            (
                "publisher = \"Example Ltd\"",
                "publisher = \"Example Ltd\"\nextra = 1",
                "unknown field",
            ),
        ];
        for (from, to, says) in cases {
            assert!(SAMPLE.contains(from), "{from}");
            let e = PackageManifest::parse(&SAMPLE.replacen(from, to, 1)).unwrap_err();
            assert_eq!(e.code(), "invalidManifest");
            assert!(e.to_string().contains(says), "{to}: {e}");
        }
        let twice = format!("{SAMPLE}\n[[permissions]]\npermission = \"network\"\nreason = \"again\"\n");
        assert!(PackageManifest::parse(&twice).unwrap_err().to_string().contains("twice"));
        let huge = format!("{SAMPLE}#{}", "x".repeat(MAX_MANIFEST_BYTES as usize));
        assert_eq!(PackageManifest::parse(&huge).unwrap_err().code(), "tooLarge");
        assert!(PackageManifest::parse("not toml [").is_err());
    }

    #[test]
    fn name_rules() {
        assert!(valid_id("com.example.a_b-c") && !valid_id("a") && !valid_id(".a.b") && !valid_id("a.b.") && !valid_id("a b.c"));
        assert!(valid_binary("kv_lines-2") && !valid_binary("") && !valid_binary("a/b") && !valid_binary("a.exe"));
        assert!(valid_key_id("0123456789abcdef") && !valid_key_id("0123456789ABCDEF") && !valid_key_id("01"));
        assert!(!valid_target("x86_64") && !valid_target("X86_64-pc-windows-msvc"));
    }
}
