//! The commands of third-party plugins (one line each in the command table). Every argument is checked here: the
//! package path must be one the user chose (dialog or drop), ids are plugin ids, the digest is 64 hex digits.

use super::dto::*;
use super::Installed;
use crate::paths::WindowGrants;
use crate::{AppState, CmdError, CmdResult, State};
use imt_ipc::supervisor::ChildState;
use imt_plugin_api::manifest::Permission;
use imt_plugin_host::installed::{InstallRecord, PluginsDir};
use imt_plugin_host::PluginProcess;
use imt_plugin_package::manifest::valid_id;
use imt_plugin_package::Trust;
use std::sync::Arc;

/// The file extension of packages.
pub const PACKAGE_EXTENSION: &str = "imtplugin";

fn dir(installed: &Installed) -> CmdResult<PluginsDir> {
    installed
        .dir
        .clone()
        .ok_or_else(|| CmdError::unavailable("IMT has no data folder to install plugins into"))
}

fn check_id(id: &str) -> CmdResult<()> {
    if valid_id(id) {
        Ok(())
    } else {
        Err(CmdError::invalid(format!("\"{id}\" is not a plugin id")).field("id"))
    }
}

fn check_digest(d: &str) -> CmdResult<()> {
    if d.len() == 64 && d.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(CmdError::invalid("not a package digest").field("digest"))
    }
}

/// At most one of each permission.
fn permissions(list: &[PermissionDto]) -> CmdResult<Vec<Permission>> {
    if list.len() > 8 {
        return Err(CmdError::invalid("too many permissions").field("granted"));
    }
    let mut out: Vec<Permission> = Vec::new();
    for p in list {
        let p = Permission::from(*p);
        if !out.contains(&p) {
            out.push(p);
        }
    }
    Ok(out)
}

/// An installed plugin as the UI shows it, from its record now and what was loaded at start.
pub fn installed_dto(r: &InstallRecord, loaded: Option<&(InstallRecord, Result<Arc<PluginProcess>, String>)>, safe_mode: bool) -> InstalledPluginDto {
    let (state, problem) = if !r.enabled {
        (InstalledState::Disabled, None)
    } else if safe_mode {
        (InstalledState::Unavailable, Some("IMT runs in safe mode: no plugins are loaded".to_string()))
    } else {
        match loaded {
            None => (InstalledState::PendingRestart, None),
            Some((then, _)) if then.digest != r.digest || !then.enabled => (InstalledState::PendingRestart, None),
            Some((_, Err(e))) => (InstalledState::Unavailable, Some(e.clone())),
            Some((_, Ok(p))) => {
                let s = p.status();
                match s.state {
                    ChildState::Running => (InstalledState::Running, None),
                    ChildState::Stopped => (InstalledState::Ready, s.reason),
                    ChildState::Quarantined => (InstalledState::Quarantined, s.reason),
                }
            }
        }
    };
    InstalledPluginDto {
        id: r.id.clone(),
        name: r.name.clone(),
        version: r.version.clone(),
        publisher: r.publisher.clone(),
        description: r.description.clone(),
        signature: SignatureDto::of_record(&r.signature, r.signer.as_ref()),
        warning: r.warning.clone(),
        requested: r.requested.iter().map(PermissionRequestDto::from).collect(),
        granted: r.granted.iter().copied().map(PermissionDto::from).collect(),
        enabled: r.enabled,
        state,
        problem,
        installed_at_ms: r.installed_at_ms,
    }
}

/// Every installed plugin.
pub fn list(installed: &Installed) -> InstalledPluginsDto {
    let mut plugins = Vec::new();
    let mut damaged = Vec::new();
    if let Some(d) = &installed.dir {
        for r in d.list() {
            match r {
                Ok(r) => plugins.push(installed_dto(&r, installed.loaded.get(&r.id), installed.safe_mode)),
                Err((id, e)) => damaged.push(DamagedPluginDto { id, message: e.to_string() }),
            }
        }
    }
    InstalledPluginsDto {
        plugins,
        damaged,
        developer_mode: installed.trust().developer_mode,
        safe_mode: installed.safe_mode,
    }
}

/// What the user reviews before installing the package at `path`.
pub fn review(installed: &Installed, path: &std::path::Path) -> CmdResult<PackageReviewDto> {
    let trust = installed.trust();
    let i = dir(installed)?.inspect(path, &trust).map_err(cmd_error)?;
    let m = &i.manifest;
    let refusal = i.installable().err().map(|e| RefusalDto::from(&e));
    Ok(PackageReviewDto {
        id: m.id.clone(),
        name: m.name.clone(),
        version: m.version.to_string(),
        publisher: m.publisher.clone(),
        description: m.description.clone(),
        protocol: m.protocol.to_string(),
        targets: m.targets.clone(),
        digest: i.digest.clone(),
        signature: SignatureDto::from(&i.signature),
        permissions: m.permissions.iter().map(PermissionRequestDto::from).collect(),
        warning: match &i.trust {
            Ok(Trust::Developer { warning }) => Some(warning.clone()),
            _ => None,
        },
        refusal,
        developer_mode: trust.developer_mode,
        update: i.installed.as_ref().map(|r| UpdateDto {
            from_version: r.version.clone(),
            added: i.added_permissions.iter().copied().map(PermissionDto::from).collect(),
            removed: i.removed_permissions.iter().copied().map(PermissionDto::from).collect(),
        }),
    })
}

/// Runs blocking work (package reading, hashing, starting the program once) off the command runtime's threads.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> CmdResult<T> + Send + 'static) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| CmdError::new(crate::ErrorCode::Internal, format!("the work stopped: {e}")))?
}

/// Every installed third-party plugin and its state in this run.
#[tauri::command]
pub async fn plugin_packages_list(state: State<'_, AppState>) -> CmdResult<InstalledPluginsDto> {
    Ok(list(&state.installed))
}

/// Reads and checks a package the user chose and says what installing it means (signature, permissions, update).
#[tauri::command]
pub async fn plugin_package_inspect(state: State<'_, AppState>, grants: WindowGrants, path: String) -> CmdResult<PackageReviewDto> {
    let file = grants.read_file(&path, &[PACKAGE_EXTENSION])?.path;
    let installed = state.installed.clone();
    blocking(move || review(&installed, &file)).await
}

/// Installs (or updates) a package the user chose and reviewed (`digest`), granting `approved`. Takes effect at the
/// next start.
#[tauri::command]
pub async fn plugin_package_install(
    state: State<'_, AppState>,
    grants: WindowGrants,
    op: crate::activity::OpId,
    path: String,
    digest: String,
    approved: Vec<PermissionDto>,
) -> CmdResult<InstalledPluginDto> {
    let file = grants.read_file(&path, &[PACKAGE_EXTENSION])?.path;
    check_digest(&digest)?;
    let approved = permissions(&approved)?;
    let installed = state.installed.clone();
    let record = blocking(move || {
        let d = dir(&installed)?;
        d.install(&file, &installed.trust(), &digest, &approved).map_err(cmd_error)
    })
    .await?;
    log::info!(Plugin = record.id.as_str(), Version = record.version.as_str(), Signature = record.signature.as_str(); "Plugin installed");
    crate::activity::domain_event("plugin.installed", op.as_deref(), &[("Plugin", &record.id)]);
    Ok(installed_dto(&record, state.installed.loaded.get(&record.id), state.installed.safe_mode))
}

/// Removes an installed plugin (its process stops; its files go at the next start). Returns whether it was
/// installed.
#[tauri::command]
pub async fn plugin_package_remove(state: State<'_, AppState>, op: crate::activity::OpId, id: String) -> CmdResult<bool> {
    check_id(&id)?;
    if let Some((_, Ok(p))) = state.installed.loaded.get(&id) {
        p.shutdown();
    }
    let removed = dir(&state.installed)?.remove(&id).map_err(cmd_error)?;
    if removed {
        log::info!(Plugin = id.as_str(); "Plugin removed");
        crate::activity::domain_event("plugin.removed", op.as_deref(), &[("Plugin", &id)]);
    }
    Ok(removed)
}

/// Turns an installed plugin on or off from the next start.
#[tauri::command]
pub async fn plugin_package_set_enabled(
    state: State<'_, AppState>,
    op: crate::activity::OpId,
    id: String,
    enabled: bool,
) -> CmdResult<InstalledPluginDto> {
    check_id(&id)?;
    let r = dir(&state.installed)?.set_enabled(&id, enabled).map_err(cmd_error)?;
    crate::activity::domain_event(if enabled { "plugin.enabled" } else { "plugin.disabled" }, op.as_deref(), &[("Plugin", &id)]);
    Ok(installed_dto(&r, state.installed.loaded.get(&id), state.installed.safe_mode))
}

/// Changes what an installed plugin is granted. A running plugin restarts at once in the new sandbox (revoking
/// takes effect immediately).
#[tauri::command]
pub async fn plugin_package_set_permissions(
    state: State<'_, AppState>,
    op: crate::activity::OpId,
    id: String,
    granted: Vec<PermissionDto>,
) -> CmdResult<InstalledPluginDto> {
    check_id(&id)?;
    let granted = permissions(&granted)?;
    let r = dir(&state.installed)?.set_granted(&id, &granted).map_err(cmd_error)?;
    if let Some((_, Ok(p))) = state.installed.loaded.get(&id) {
        p.set_granted(r.granted.clone());
    }
    log::info!(Plugin = id.as_str(); "Plugin permissions changed");
    crate::activity::domain_event("plugin.permissions", op.as_deref(), &[("Plugin", &id)]);
    Ok(installed_dto(&r, state.installed.loaded.get(&id), state.installed.safe_mode))
}

#[cfg(test)]
pub(super) mod checks {
    pub use super::{check_digest, check_id, permissions};
}
