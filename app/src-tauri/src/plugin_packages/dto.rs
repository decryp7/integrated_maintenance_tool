//! The wire types of third-party plugins (camelCase JSON, generated into `app/src/app/core/generated/plugins.ts`
//! and re-exported by `api.types.ts`).

use crate::{CmdError, ErrorCode};
use imt_plugin_api::manifest::{Permission, PermissionRequest};
use imt_plugin_package::{PackageError, SignatureStatus};
use serde::{Deserialize, Serialize};

/// A right a plugin asks for (mirrors `imt_plugin_api::manifest::Permission`).
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionDto {
    /// Read the files the user opens directly (IMT's own plugins only).
    OpenedFilePaths,
    /// Start other programs.
    RunPrograms,
    /// Write into folders the user chooses (IMT's own plugins only).
    WriteFolders,
    /// Connect to the network.
    Network,
}

impl From<Permission> for PermissionDto {
    fn from(p: Permission) -> Self {
        match p {
            Permission::OpenedFilePaths => PermissionDto::OpenedFilePaths,
            Permission::RunPrograms => PermissionDto::RunPrograms,
            Permission::WriteFolders => PermissionDto::WriteFolders,
            Permission::Network => PermissionDto::Network,
        }
    }
}

impl From<PermissionDto> for Permission {
    fn from(p: PermissionDto) -> Self {
        match p {
            PermissionDto::OpenedFilePaths => Permission::OpenedFilePaths,
            PermissionDto::RunPrograms => Permission::RunPrograms,
            PermissionDto::WriteFolders => Permission::WriteFolders,
            PermissionDto::Network => Permission::Network,
        }
    }
}

/// A permission, why the plugin wants it, and what the user is asked to allow.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRequestDto {
    pub permission: PermissionDto,
    /// The publisher's reason.
    pub reason: String,
    /// What the user allows, in IMT's words ("connect to the network").
    pub allows: String,
}

impl From<&PermissionRequest> for PermissionRequestDto {
    fn from(r: &PermissionRequest) -> Self {
        PermissionRequestDto {
            permission: r.permission.into(),
            reason: r.reason.clone(),
            allows: r.permission.describe().into(),
        }
    }
}

/// Who signed a package.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SignatureKind {
    /// A trusted publisher key.
    Trusted,
    /// A key the user has not trusted.
    UnknownKey,
    /// Not signed.
    Unsigned,
}

/// A package's signature as the user sees it.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureDto {
    pub kind: SignatureKind,
    /// The trusted key's name, or the unknown key's id.
    pub signer: Option<String>,
}

impl From<&SignatureStatus> for SignatureDto {
    fn from(s: &SignatureStatus) -> Self {
        match s {
            SignatureStatus::Trusted(n) => SignatureDto {
                kind: SignatureKind::Trusted,
                signer: Some(n.clone()),
            },
            SignatureStatus::UnknownKey(k) => SignatureDto {
                kind: SignatureKind::UnknownKey,
                signer: Some(k.clone()),
            },
            SignatureStatus::Unsigned => SignatureDto {
                kind: SignatureKind::Unsigned,
                signer: None,
            },
        }
    }
}

impl SignatureDto {
    /// From an install record's stored status.
    pub fn of_record(signature: &str, signer: Option<&String>) -> SignatureDto {
        SignatureDto {
            kind: match signature {
                "trusted" => SignatureKind::Trusted,
                "unknownKey" => SignatureKind::UnknownKey,
                _ => SignatureKind::Unsigned,
            },
            signer: signer.cloned(),
        }
    }
}

/// What installing over an installed version changes.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDto {
    /// The installed version.
    pub from_version: String,
    /// Permissions the new version asks for that the installed one did not.
    pub added: Vec<PermissionDto>,
    /// Permissions it no longer asks for.
    pub removed: Vec<PermissionDto>,
}

/// Why a package cannot be installed (the same `code` as the error `plugin_package_install` would give).
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefusalDto {
    /// The package reason (`unsigned`, `unknownKey`, `incompatible`, `wrongTarget`, ...).
    pub reason: String,
    pub message: String,
}

impl From<&PackageError> for RefusalDto {
    fn from(e: &PackageError) -> Self {
        RefusalDto {
            reason: e.code().into(),
            message: e.to_string(),
        }
    }
}

/// A package the user is about to install: who made it, what it asks for, and whether it can be installed.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageReviewDto {
    pub id: String,
    pub name: String,
    pub version: String,
    pub publisher: String,
    pub description: String,
    /// The plugin protocol it was built for (`1.0`).
    pub protocol: String,
    pub targets: Vec<String>,
    /// Pass it back to `plugin_package_install`: exactly this content is installed.
    pub digest: String,
    pub signature: SignatureDto,
    /// The permissions to approve (each can be left out).
    pub permissions: Vec<PermissionRequestDto>,
    /// Why it cannot be installed now; `None`: it can.
    pub refusal: Option<RefusalDto>,
    /// Shown before installing (developer mode: unsigned or unknown key).
    pub warning: Option<String>,
    pub developer_mode: bool,
    /// The installed version it replaces.
    pub update: Option<UpdateDto>,
}

/// The state of an installed plugin in this run.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InstalledState {
    /// Loaded; its process starts on first use.
    Ready,
    /// Its process is running.
    Running,
    /// It failed repeatedly; Reset lifts it.
    Quarantined,
    /// Turned off.
    Disabled,
    /// Installed, updated or turned on since IMT started: it takes effect after a restart.
    PendingRestart,
    /// It cannot run (see `problem`).
    Unavailable,
}

/// An installed third-party plugin.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPluginDto {
    pub id: String,
    pub name: String,
    pub version: String,
    pub publisher: String,
    pub description: String,
    pub signature: SignatureDto,
    /// The warning accepted at install (developer mode).
    pub warning: Option<String>,
    /// What it asks for.
    pub requested: Vec<PermissionRequestDto>,
    /// What the user granted.
    pub granted: Vec<PermissionDto>,
    pub enabled: bool,
    pub state: InstalledState,
    /// Why it is unavailable or quarantined.
    pub problem: Option<String>,
    /// Milliseconds since the Unix epoch.
    #[cfg_attr(test, ts(type = "number"))]
    pub installed_at_ms: u64,
}

/// A plugin whose record cannot be read.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DamagedPluginDto {
    pub id: String,
    pub message: String,
}

/// Every installed third-party plugin.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPluginsDto {
    pub plugins: Vec<InstalledPluginDto>,
    pub damaged: Vec<DamagedPluginDto>,
    /// Unsigned packages and unknown keys may be installed (Settings → Plugin trust).
    pub developer_mode: bool,
    /// Third-party plugins are not loaded in this run (safe mode).
    pub safe_mode: bool,
}

/// A package error as a command error: the package reason is in `detail`.
pub fn cmd_error(e: PackageError) -> CmdError {
    let code = match &e {
        PackageError::Io(_) => ErrorCode::Io,
        PackageError::TooLarge { .. } => ErrorCode::TooLarge,
        PackageError::NotAPackage(_)
        | PackageError::InvalidManifest(_)
        | PackageError::UnlistedFile(_)
        | PackageError::MissingFile(_)
        | PackageError::UnsafeEntry(_) => ErrorCode::InvalidManifest,
        PackageError::Tampered(_) => ErrorCode::SignatureInvalid,
        PackageError::Unsigned | PackageError::UnknownKey { .. } => ErrorCode::Untrusted,
        PackageError::Incompatible(_) => ErrorCode::Incompatible,
        PackageError::WrongTarget { .. } => ErrorCode::Unsupported,
        PackageError::InvalidKey(_) => ErrorCode::Invalid,
    };
    let hint = match &e {
        PackageError::Unsigned | PackageError::UnknownKey { .. } => {
            Some("Trust the publisher's key, or turn on developer mode, in Settings → Plugin trust.")
        }
        PackageError::Tampered(_) => Some("Get the package again from its publisher."),
        _ => None,
    };
    let reason = e.code().to_string();
    let mut err = CmdError::new(code, e.to_string()).detail(Some(reason));
    if let Some(h) = hint {
        err = err.hint(h);
    }
    err
}

/// The TypeScript of these types (`plugins.ts`, see `generated_types`).
#[cfg(test)]
pub fn generated(header: &str) -> String {
    use ts_rs::TS;
    let decls = [
        PermissionDto::decl(),
        PermissionRequestDto::decl(),
        SignatureKind::decl(),
        SignatureDto::decl(),
        UpdateDto::decl(),
        RefusalDto::decl(),
        PackageReviewDto::decl(),
        InstalledState::decl(),
        InstalledPluginDto::decl(),
        DamagedPluginDto::decl(),
        InstalledPluginsDto::decl(),
    ];
    format!("{header}{}", decls.iter().map(|d| format!("export {d}\n")).collect::<Vec<_>>().join("\n"))
}
