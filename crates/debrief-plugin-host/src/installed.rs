//! Third-party plugins installed from `.debriefplugin` packages (ADR 0027): where they live, installing, updating,
//! removing, enabling and granting, and their processes (contained, verified by hash before every start).
//!
//! ```text
//! <plugins>/<id>/install.json                  what was installed and what the user granted (InstallRecord)
//! <plugins>/<id>/v-<version>-<digest8>/bin/<program>
//! <plugins>/<id>/v-<version>-<digest8>/manifest.toml, contract.json (the program's own manifest)
//! <plugins>/<id>/data/                         the plugin's private folder, kept across updates
//! ```
//!
//! An install or update writes a new version folder next to the old one and then switches `install.json` (written
//! atomically); a running process keeps its program until Debrief restarts, and [`PluginsDir::cleanup`] at start removes
//! folders nothing refers to. Every value read back from disk is checked; a damaged record makes its plugin
//! unavailable with a reason, never a crash.

use crate::process::{sha256_of, PluginProcess, PluginSpec};
use debrief_ipc::Json;
use debrief_plugin_api::manifest::{Manifest, Permission, PermissionRequest};
use debrief_plugin_api::protocol::PluginRequest;
use debrief_plugin_api::sandbox::{container_name, PluginHome};
use debrief_plugin_api::PROTOCOL;
use debrief_plugin_package::keys::to_hex;
use debrief_plugin_package::manifest::{host_target, valid_id, PackageManifest};
use debrief_plugin_package::{Package, PackageError, SignatureStatus, Trust, TrustSettings};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// The record file in a plugin's folder.
pub const RECORD_FILE: &str = "install.json";
/// The format of [`InstallRecord`].
pub const RECORD_FORMAT: u32 = 1;
/// How long the program may take to describe itself when installed.
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
/// Largest record or contract file read back.
const MAX_RECORD_BYTES: u64 = 4 << 20;

/// What was installed, and what the user decided about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallRecord {
    /// [`RECORD_FORMAT`].
    pub format: u32,
    /// The plugin's id.
    pub id: String,
    /// Its version.
    pub version: String,
    /// Its name.
    pub name: String,
    /// Its publisher.
    pub publisher: String,
    /// Its description.
    pub description: String,
    /// The signature status when it was installed (`trusted`, `unknownKey`, `unsigned`).
    pub signature: String,
    /// The trusted key's name, or the unknown key's id.
    pub signer: Option<String>,
    /// The warning the user accepted (developer mode), if any.
    pub warning: Option<String>,
    /// The package digest the user approved.
    pub digest: String,
    /// The version folder, relative to the plugin's folder (`v-1.2.0-0a1b2c3d`).
    pub folder: String,
    /// The program's file name in `<folder>/bin`.
    pub program: String,
    /// The program's SHA-256, hex (checked before every start).
    pub sha256: String,
    /// The permissions it asks for, with reasons.
    pub requested: Vec<PermissionRequest>,
    /// The permissions the user granted (a subset of `requested`).
    pub granted: Vec<Permission>,
    /// Whether it runs (from the next start).
    pub enabled: bool,
    /// When it was installed, milliseconds since the Unix epoch.
    pub installed_at_ms: u64,
}

/// What the user sees before installing a package.
#[derive(Debug, Clone)]
pub struct Inspection {
    /// The package's manifest.
    pub manifest: PackageManifest,
    /// Its digest (pass it back to [`PluginsDir::install`]: the user approves exactly this content).
    pub digest: String,
    /// Who signed it.
    pub signature: SignatureStatus,
    /// Whether it may be installed now, and how far it is trusted; the error says why not.
    pub trust: Result<Trust, PackageError>,
    /// Whether this Debrief can run it (protocol version, a program for this computer).
    pub runnable: Result<(), PackageError>,
    /// The installed version of the same plugin, when there is one.
    pub installed: Option<InstallRecord>,
    /// Permissions it asks for that the installed version did not.
    pub added_permissions: Vec<Permission>,
    /// Permissions the installed version asked for that it no longer does.
    pub removed_permissions: Vec<Permission>,
}

impl Inspection {
    /// Whether [`PluginsDir::install`] would accept it (with the same trust settings).
    pub fn installable(&self) -> Result<(), PackageError> {
        self.trust.as_ref().map_err(Clone::clone)?;
        self.runnable.clone()
    }
}

/// An installed plugin with what it needs to run, or why it cannot.
pub struct Loaded {
    /// Its record.
    pub record: InstallRecord,
    /// Its process, or why it is unavailable.
    pub process: Result<Arc<PluginProcess>, String>,
}

fn io(e: std::io::Error) -> PackageError {
    PackageError::Io(e.to_string())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// Whether `rel` is one plain path component (no separators, no `..`).
fn plain(rel: &str) -> bool {
    let p = Path::new(rel);
    !rel.is_empty() && p.components().count() == 1 && matches!(p.components().next(), Some(Component::Normal(_)))
}

/// Writes `bytes` to `path` through a temporary file and a rename (never a half-written file).
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

fn read_bounded(path: &Path) -> std::io::Result<Vec<u8>> {
    let len = std::fs::metadata(path)?.len();
    if len > MAX_RECORD_BYTES {
        return Err(std::io::Error::other(format!("{} is too large", path.display())));
    }
    std::fs::read(path)
}

/// The folder of installed third-party plugins.
#[derive(Debug, Clone)]
pub struct PluginsDir {
    root: PathBuf,
}

impl PluginsDir {
    /// The plugins below `root` (`<app data>/plugins`).
    pub fn new(root: impl Into<PathBuf>) -> PluginsDir {
        PluginsDir { root: root.into() }
    }

    /// The folder.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn plugin_dir(&self, id: &str) -> Result<PathBuf, PackageError> {
        if !valid_id(id) {
            return Err(PackageError::InvalidManifest(format!("\"{id}\" is not a plugin id")));
        }
        Ok(self.root.join(id))
    }

    /// The folders of an installed plugin, as its sandbox sees them.
    pub fn home(&self, r: &InstallRecord) -> Result<PluginHome, PackageError> {
        let dir = self.plugin_dir(&r.id)?;
        if !plain(&r.folder) || !plain(&r.program) {
            return Err(PackageError::Tampered(format!(
                "the record of {} names paths outside its folder",
                r.id
            )));
        }
        Ok(PluginHome {
            container: container_name(&r.id),
            program_dir: dir.join(&r.folder).join("bin"),
            data_dir: dir.join("data"),
        })
    }

    /// The record of `id`, when it is installed (a damaged record is an error with the reason).
    pub fn record(&self, id: &str) -> Result<Option<InstallRecord>, PackageError> {
        let path = self.plugin_dir(id)?.join(RECORD_FILE);
        let bytes = match read_bounded(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io(e)),
        };
        let r: InstallRecord =
            serde_json::from_slice(&bytes).map_err(|e| PackageError::Io(format!("the record of {id} is damaged ({e}); reinstall it")))?;
        if r.format != RECORD_FORMAT || r.id != id {
            return Err(PackageError::Io(format!(
                "the record of {id} is not one this Debrief reads; reinstall it"
            )));
        }
        Ok(Some(r))
    }

    /// Every installed plugin, by id; damaged records are listed as errors.
    pub fn list(&self) -> Vec<Result<InstallRecord, (String, PackageError)>> {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut ids: Vec<String> = entries
            .flatten()
            .filter(|e| e.path().join(RECORD_FILE).is_file())
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .filter(|id| valid_id(id))
            .collect();
        ids.sort();
        ids.into_iter()
            .filter_map(|id| match self.record(&id) {
                Ok(Some(r)) => Some(Ok(r)),
                Ok(None) => None,
                Err(e) => Some(Err((id, e))),
            })
            .collect()
    }

    fn save(&self, r: &InstallRecord) -> Result<(), PackageError> {
        let dir = self.plugin_dir(&r.id)?;
        std::fs::create_dir_all(&dir).map_err(io)?;
        let json = serde_json::to_vec_pretty(r).map_err(|e| PackageError::Io(e.to_string()))?;
        write_atomic(&dir.join(RECORD_FILE), &json).map_err(io)
    }

    /// Reads and checks the package at `path` and says what installing it would mean.
    pub fn inspect(&self, path: &Path, trust: &TrustSettings) -> Result<Inspection, PackageError> {
        let pkg = Package::open(path)?;
        let m = &pkg.manifest;
        let installed = self.record(&m.id).ok().flatten();
        let asked: Vec<Permission> = m.permissions.iter().map(|p| p.permission).collect();
        let before: Vec<Permission> = installed
            .as_ref()
            .map(|r| r.requested.iter().map(|p| p.permission).collect())
            .unwrap_or_default();
        let runnable = if !PROTOCOL.accepts(m.protocol) {
            Err(PackageError::Incompatible(
                PROTOCOL.mismatch(m.protocol, &format!("plugin \"{}\"", m.id)).unwrap_or_default(),
            ))
        } else if pkg.program(host_target()).is_none() {
            Err(PackageError::WrongTarget {
                target: host_target().into(),
                supported: m.targets.join(", "),
            })
        } else {
            Ok(())
        };
        Ok(Inspection {
            digest: pkg.digest(),
            signature: trust.status(&pkg.signature),
            trust: trust.decide(&pkg.signature),
            runnable,
            added_permissions: asked.iter().copied().filter(|p| !before.contains(p)).collect(),
            removed_permissions: before
                .iter()
                .copied()
                .filter(|p| installed.is_some() && !asked.contains(p))
                .collect(),
            installed,
            manifest: pkg.manifest,
        })
    }

    /// Installs (or updates) the package at `path`, which must still have `digest` (what the user reviewed),
    /// granting the `approved` permissions it asks for. The program is started once, contained, to check that it is
    /// what the package says. Takes effect at the next start of Debrief.
    pub fn install(
        &self,
        path: &Path,
        trust: &TrustSettings,
        digest: &str,
        approved: &[Permission],
    ) -> Result<InstallRecord, PackageError> {
        let inspection = self.inspect(path, trust)?;
        if inspection.digest != digest {
            return Err(PackageError::Tampered(
                "the package changed after you reviewed it; review it again".into(),
            ));
        }
        let trust_level = inspection.installable().and(inspection.trust.clone())?;
        let pkg = Package::open(path)?;
        if pkg.digest() != digest {
            return Err(PackageError::Tampered(
                "the package changed after you reviewed it; review it again".into(),
            ));
        }
        let m = &pkg.manifest;
        let dir = self.plugin_dir(&m.id)?;
        let folder = format!("v-{}-{}", m.version, &digest[..8]);
        let version_dir = dir.join(&folder);
        let staging = dir.join(format!(".staging-{}", now_ms()));
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(staging.join("bin")).map_err(io)?;
        std::fs::create_dir_all(dir.join("data")).map_err(io)?;
        let result = (|| -> Result<InstallRecord, PackageError> {
            let program = m.binary_file(host_target());
            let sha = pkg.extract(path, host_target(), &staging.join("bin").join(&program))?;
            std::fs::write(staging.join("manifest.toml"), m.to_toml()).map_err(io)?;
            let home = PluginHome {
                container: container_name(&m.id),
                program_dir: staging.join("bin"),
                data_dir: dir.join("data"),
            };
            let contract = probe(&staging.join("bin").join(&program), m, &home, sha)?;
            check_contract(&contract, m)?;
            let contract_json = serde_json::to_vec_pretty(&contract).map_err(|e| PackageError::Io(e.to_string()))?;
            std::fs::write(staging.join("contract.json"), contract_json).map_err(io)?;
            if version_dir.exists() {
                std::fs::remove_dir_all(&version_dir).map_err(io)?;
            }
            std::fs::rename(&staging, &version_dir).map_err(io)?;
            let previous = inspection.installed.clone();
            let (signature, signer) = match &inspection.signature {
                SignatureStatus::Trusted(name) => ("trusted", Some(name.clone())),
                SignatureStatus::UnknownKey(id) => ("unknownKey", Some(id.clone())),
                SignatureStatus::Unsigned => ("unsigned", None),
            };
            let record = InstallRecord {
                format: RECORD_FORMAT,
                id: m.id.clone(),
                version: m.version.to_string(),
                name: m.name.clone(),
                publisher: m.publisher.clone(),
                description: m.description.clone(),
                signature: signature.into(),
                signer,
                warning: match trust_level {
                    Trust::Developer { warning } => Some(warning),
                    Trust::Trusted { .. } => None,
                },
                digest: digest.into(),
                folder,
                program,
                sha256: to_hex(&sha),
                requested: m.permissions.clone(),
                granted: approved.iter().copied().filter(|p| m.asks_for(*p)).collect(),
                enabled: previous.map(|p| p.enabled).unwrap_or(true),
                installed_at_ms: now_ms(),
            };
            self.save(&record)?;
            Ok(record)
        })();
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&staging);
        }
        result
    }

    /// Removes plugin `id` (its files go at the next start, when its program is no longer running). Returns whether
    /// it was installed.
    pub fn remove(&self, id: &str) -> Result<bool, PackageError> {
        let path = self.plugin_dir(id)?.join(RECORD_FILE);
        match std::fs::remove_file(&path) {
            Ok(()) => {
                // Gone now if nothing holds it; else at the next cleanup.
                let _ = std::fs::remove_dir_all(self.plugin_dir(id)?);
                Ok(true)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(io(e)),
        }
    }

    /// Turns plugin `id` on or off (from the next start).
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<InstallRecord, PackageError> {
        let mut r = self.record(id)?.ok_or_else(|| PackageError::MissingFile(format!("plugin {id}")))?;
        r.enabled = enabled;
        self.save(&r)?;
        Ok(r)
    }

    /// Sets the permissions plugin `id` is granted (only ones it asks for count).
    pub fn set_granted(&self, id: &str, granted: &[Permission]) -> Result<InstallRecord, PackageError> {
        let mut r = self.record(id)?.ok_or_else(|| PackageError::MissingFile(format!("plugin {id}")))?;
        r.granted = granted
            .iter()
            .copied()
            .filter(|p| r.requested.iter().any(|q| q.permission == *p))
            .collect();
        r.granted.dedup();
        self.save(&r)?;
        Ok(r)
    }

    /// Removes what nothing refers to: plugins without a record (and their containers), old version folders,
    /// unfinished installs. Run at start, before any plugin process.
    pub fn cleanup(&self) {
        let Ok(entries) = std::fs::read_dir(&self.root) else { return };
        for e in entries.flatten() {
            let path = e.path();
            let Some(id) = e.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if !path.is_dir() {
                continue;
            }
            match self.record(&id) {
                Ok(Some(r)) => {
                    let Ok(inner) = std::fs::read_dir(&path) else { continue };
                    for v in inner.flatten() {
                        let name = v.file_name().to_string_lossy().into_owned();
                        if v.path().is_dir() && name != r.folder && name != "data" {
                            let _ = std::fs::remove_dir_all(v.path());
                        }
                    }
                }
                Ok(None) if valid_id(&id) && std::fs::remove_dir_all(&path).is_ok() => delete_container(&container_name(&id)),
                _ => {}
            }
        }
    }

    /// The enabled plugins with their processes (started on first use); a plugin that cannot run is listed with
    /// the reason.
    pub fn load(&self) -> Vec<Loaded> {
        self.list()
            .into_iter()
            .filter_map(Result::ok)
            .filter(|r| r.enabled)
            .map(|record| {
                let process = self.process(&record).map_err(|e| e.to_string());
                Loaded { record, process }
            })
            .collect()
    }

    /// The process of an installed plugin.
    pub fn process(&self, r: &InstallRecord) -> Result<Arc<PluginProcess>, PackageError> {
        let home = self.home(r)?;
        let folder = self.plugin_dir(&r.id)?.join(&r.folder);
        let bytes = read_bounded(&folder.join("contract.json")).map_err(io)?;
        let contract: Manifest = serde_json::from_slice(&bytes).map_err(|e| PackageError::Io(format!("contract.json: {e}")))?;
        contract.check().map_err(PackageError::Incompatible)?;
        if *contract.id != r.id {
            return Err(PackageError::Tampered(format!(
                "the installed program of {} describes another plugin",
                r.id
            )));
        }
        let sha = debrief_plugin_package::keys::from_hex::<32>(&r.sha256)
            .ok_or_else(|| PackageError::Tampered(format!("the record of {} is damaged", r.id)))?;
        let exe = home.program_dir.join(&r.program);
        let spec = PluginSpec::third_party(exe, &r.id, home, sha, r.granted.clone());
        Ok(PluginProcess::new(contract, spec))
    }
}

/// Starts the program once, contained and without grants, and asks it to describe itself.
fn probe(exe: &Path, m: &PackageManifest, home: &PluginHome, sha: [u8; 32]) -> Result<Manifest, PackageError> {
    let placeholder = Manifest {
        id: m.id.as_str().into(),
        name: m.name.as_str().into(),
        version: m.version.to_string().into(),
        protocol: m.protocol,
        permissions: Vec::new(),
        memory_limit: None,
        importers: Vec::new(),
        config_parsers: Vec::new(),
        marker_kinds: Vec::new(),
        annotators: Vec::new(),
        transforms: Vec::new(),
        settings: None,
        tools: Vec::new(),
        profile_importers: Vec::new(),
        document_importers: Vec::new(),
    };
    let mut spec = PluginSpec::third_party(exe, &m.id, home.clone(), sha, Vec::new());
    spec.log = None;
    spec.supervisor.log = None;
    let p = PluginProcess::new(placeholder, spec);
    let reply: Result<Json, _> = p.call(&PluginRequest::Manifest, Some(PROBE_TIMEOUT), false);
    p.shutdown();
    let json = reply.map_err(|e| PackageError::Incompatible(format!("the plugin's program does not start in Debrief's sandbox: {e}")))?;
    serde_json::from_value(json.0).map_err(|e| PackageError::Incompatible(format!("the plugin's program describes itself wrongly: {e}")))
}

/// Checks that the program is what its package says.
fn check_contract(c: &Manifest, m: &PackageManifest) -> Result<(), PackageError> {
    c.check().map_err(PackageError::Incompatible)?;
    let bad = |what: String| PackageError::InvalidManifest(format!("the program and its package disagree: {what}"));
    if *c.id != m.id {
        return Err(bad(format!("the program is \"{}\", the package \"{}\"", c.id, m.id)));
    }
    if *c.version != m.version.to_string() {
        return Err(bad(format!("the program is version {}, the package {}", c.version, m.version)));
    }
    let mut a: Vec<Permission> = c.permissions.iter().map(|p| p.permission).collect();
    let mut b: Vec<Permission> = m.permissions.iter().map(|p| p.permission).collect();
    a.sort_by_key(|p| format!("{p:?}"));
    b.sort_by_key(|p| format!("{p:?}"));
    if a != b {
        return Err(bad(format!("the program asks for {a:?}, the package declares {b:?}")));
    }
    if c.has_elevated_ops() {
        return Err(PackageError::InvalidManifest(
            "operations that need administrator rights are only available to Debrief's own plugins".into(),
        ));
    }
    Ok(())
}

/// Deletes a plugin's container profile (Windows); nothing elsewhere.
fn delete_container(name: &str) {
    #[cfg(windows)]
    {
        let _ = debrief_ipc::contain::Container::delete(name);
    }
    #[cfg(not(windows))]
    let _ = name;
}

/// The SHA-256 of an installed plugin's program now, hex (for diagnostics).
pub fn program_sha256(path: &Path) -> Option<String> {
    sha256_of(path).ok().map(|h| to_hex(&h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_name_only_plain_paths() {
        assert!(plain("v-1.0.0-abcd1234") && plain("kv.exe"));
        assert!(!plain("../x") && !plain("a/b") && !plain("") && !plain(".."));
        let d = PluginsDir::new("/plugins");
        let mut r = InstallRecord {
            format: RECORD_FORMAT,
            id: "com.example.x".into(),
            version: "1.0.0".into(),
            name: "X".into(),
            publisher: "P".into(),
            description: String::new(),
            signature: "unsigned".into(),
            signer: None,
            warning: None,
            digest: "0".repeat(64),
            folder: "v-1.0.0-00000000".into(),
            program: "x.exe".into(),
            sha256: "0".repeat(64),
            requested: Vec::new(),
            granted: Vec::new(),
            enabled: true,
            installed_at_ms: 0,
        };
        let home = d.home(&r).unwrap();
        assert!(home.program_dir.ends_with(Path::new("com.example.x/v-1.0.0-00000000/bin")));
        assert!(home.data_dir.ends_with(Path::new("com.example.x/data")));
        r.folder = "../../evil".into();
        assert_eq!(d.home(&r).unwrap_err().code(), "tampered");
        r.id = "../evil".into();
        assert!(d.home(&r).is_err());
    }

    #[test]
    fn damaged_or_missing_records_degrade() {
        let t = tempfile::tempdir().unwrap();
        let d = PluginsDir::new(t.path());
        assert!(d.list().is_empty());
        assert_eq!(d.record("com.example.none").unwrap(), None);
        std::fs::create_dir_all(t.path().join("com.example.bad")).unwrap();
        std::fs::write(t.path().join("com.example.bad").join(RECORD_FILE), b"{not json").unwrap();
        let list = d.list();
        assert!(matches!(&list[..], [Err((id, e))] if id == "com.example.bad" && e.to_string().contains("damaged")));
        assert!(d.load().is_empty());
        assert!(!d.remove("com.example.none").unwrap());
        assert!(d.remove("com.example.bad").unwrap());
        assert!(d.set_enabled("com.example.none", true).is_err());
        // An unfinished install folder without a record is removed at cleanup.
        std::fs::create_dir_all(t.path().join("com.example.left").join("v-1")).unwrap();
        d.cleanup();
        assert!(!t.path().join("com.example.left").exists());
    }
}
