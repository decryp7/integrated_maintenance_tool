//! The sandbox of a plugin's process (ADR 0017 §6): locked down by default (no files, no network, no other
//! programs, low integrity, a memory cap), widened only by the permissions it asked for and was granted.

use imt_ipc::sandbox::{FsAccess, SandboxSpec};
use imt_plugin_api::manifest::{Manifest, Permission, MAX_MEMORY_LIMIT};

/// Memory of a plugin process that asks for no other limit.
pub const DEFAULT_MEMORY_LIMIT: u64 = 1 << 30;

/// The sandbox of `manifest`'s plugin with the permissions in `granted` (its elevated process when `elevated`).
pub fn sandbox_for(manifest: &Manifest, granted: &[Permission], elevated: bool) -> SandboxSpec {
    let has = |p: Permission| manifest.asks_for(p) && granted.contains(&p);
    let mut s = SandboxSpec::locked();
    s.memory_limit = Some(
        manifest
            .memory_limit
            .unwrap_or(DEFAULT_MEMORY_LIMIT)
            .clamp(64 << 20, MAX_MEMORY_LIMIT),
    );
    if has(Permission::OpenedFilePaths) {
        // It reads the files the user opened by their paths (reading is not prevented by low integrity).
        s.fs = FsAccess::Unrestricted;
    }
    if has(Permission::RunPrograms) {
        // Programs it starts inherit its token; at low integrity most system tools fail to write their output.
        s.children = true;
        s.low_integrity = false;
        s.fs = FsAccess::Unrestricted;
    }
    if has(Permission::WriteFolders) {
        s.low_integrity = false;
        s.fs = FsAccess::Unrestricted;
    }
    if has(Permission::Network) {
        s.network = true;
    }
    if elevated {
        // An elevated process cannot lower its own integrity below what its operations need (ETW sessions);
        // it keeps no file-system writes of its own by contract (its output streams back to the host).
        s.low_integrity = false;
        s.fs = FsAccess::Unrestricted;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_plugin_api::manifest::PermissionRequest;

    fn manifest(perms: &[Permission]) -> Manifest {
        Manifest {
            id: "p".into(),
            name: "P".into(),
            version: "1.0.0".into(),
            protocol: imt_plugin_api::PROTOCOL,
            permissions: perms
                .iter()
                .map(|p| PermissionRequest {
                    permission: *p,
                    reason: String::new(),
                })
                .collect(),
            memory_limit: None,
            importers: Vec::new(),
            config_parsers: Vec::new(),
            marker_kinds: Vec::new(),
            annotators: Vec::new(),
            transforms: Vec::new(),
            settings: None,
            tools: Vec::new(),
            profile_importers: Vec::new(),
        }
    }

    #[test]
    fn plugins_are_locked_down_unless_granted() {
        let parser = sandbox_for(&manifest(&[]), &[], false);
        assert!(parser.low_integrity && !parser.network && !parser.children && parser.fs == FsAccess::Nothing);
        assert_eq!(parser.memory_limit, Some(DEFAULT_MEMORY_LIMIT));
        // Asked but not granted: still locked.
        let asked = sandbox_for(&manifest(&[Permission::RunPrograms]), &[], false);
        assert!(!asked.children && asked.low_integrity);
        // Granted but not asked: nothing changes.
        let unasked = sandbox_for(&manifest(&[]), &[Permission::Network], false);
        assert!(!unasked.network);
        let tools = sandbox_for(
            &manifest(&[Permission::RunPrograms, Permission::WriteFolders]),
            &[Permission::RunPrograms, Permission::WriteFolders],
            false,
        );
        assert!(tools.children && !tools.low_integrity && tools.fs == FsAccess::Unrestricted && !tools.network);
        let paths = sandbox_for(&manifest(&[Permission::OpenedFilePaths]), &[Permission::OpenedFilePaths], false);
        assert!(paths.low_integrity && paths.fs == FsAccess::Unrestricted);
        let net = sandbox_for(&manifest(&[Permission::Network]), &[Permission::Network], false);
        assert!(net.network);
        let elevated = sandbox_for(&manifest(&[]), &[], true);
        assert!(!elevated.low_integrity && !elevated.children);
        let mut big = manifest(&[]);
        big.memory_limit = Some(u64::MAX);
        assert_eq!(sandbox_for(&big, &[], false).memory_limit, Some(MAX_MEMORY_LIMIT));
    }
}
