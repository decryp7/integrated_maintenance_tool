//! The sandbox of a plugin's process (ADR 0017 §6, ADR 0027): locked down by default (no files, no network, no
//! other programs, low integrity, a memory cap), widened only by the permissions it asked for and was granted.
//!
//! Debrief's own plugins lock themselves down after connecting ([`sandbox_for`]). Third-party plugins are also
//! **contained by the launcher** before they run ([`sandbox_for_third_party`]): an AppContainer on Windows,
//! landlock and seccomp before `exec` on Linux, `sandbox-exec` on macOS. The test harness (`debrief-plugin-test`)
//! uses the same functions, so a plugin is tested in the sandbox it gets in Debrief.

use crate::manifest::{Manifest, Permission, MAX_MEMORY_LIMIT};
use debrief_ipc::sandbox::{Containment, FsAccess, SandboxSpec};
use std::path::PathBuf;

/// Memory of a plugin process that asks for no other limit.
pub const DEFAULT_MEMORY_LIMIT: u64 = 1 << 30;
/// Smallest memory cap a plugin gets, whatever it asks for.
pub const MIN_MEMORY_LIMIT: u64 = 64 << 20;

/// The memory cap of `manifest`'s plugin.
fn memory(manifest: &Manifest) -> u64 {
    manifest
        .memory_limit
        .unwrap_or(DEFAULT_MEMORY_LIMIT)
        .clamp(MIN_MEMORY_LIMIT, MAX_MEMORY_LIMIT)
}

/// The sandbox of one of Debrief's own plugins, `manifest`'s, with the permissions in `granted` (its elevated process
/// when `elevated`). A permission counts only when the manifest asks for it and it was granted.
pub fn sandbox_for(manifest: &Manifest, granted: &[Permission], elevated: bool) -> SandboxSpec {
    let has = |p: Permission| manifest.asks_for(p) && granted.contains(&p);
    let mut s = SandboxSpec::locked();
    s.memory_limit = Some(memory(manifest));
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
    if has(Permission::ReadFolders) && s.fs == FsAccess::Nothing {
        // Reading anything, writing nothing: low integrity on Windows (it reads, it cannot write), a read-only
        // file system elsewhere. The folders are settings that change after the start, so they are not listed.
        s.fs = FsAccess::Paths {
            read: vec![PathBuf::from("/")],
            write: Vec::new(),
        };
    }
    if elevated {
        // An elevated process cannot lower its own integrity below what its operations need (ETW sessions);
        // it keeps no file-system writes of its own by contract (its output streams back to the host).
        s.low_integrity = false;
        s.fs = FsAccess::Unrestricted;
    }
    s
}

/// Where an installed third-party plugin lives, as its sandbox sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginHome {
    /// The name of its container (stable per plugin id; see [`container_name`]).
    pub container: String,
    /// The folder of its program (readable and executable, never writable).
    pub program_dir: PathBuf,
    /// Its private data folder (readable and writable); tools receive it as their data folder.
    pub data_dir: PathBuf,
}

/// The container name of plugin `id`: `debrief.plugin.` and 16 hex digits of a hash of the id (container names are
/// limited to 64 characters).
pub fn container_name(id: &str) -> String {
    // FNV-1a, 64 bits: stable across builds and platforms (the name must not change between versions of Debrief).
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in id.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("debrief.plugin.{h:016x}")
}

/// The sandbox of a third-party plugin: the strictest one ([`SandboxSpec::locked`]: no files but its own folders,
/// no network, no other programs, low integrity, a memory cap), enforced by the launcher before the program runs,
/// and widened only by `network` and `runPrograms` when asked for and granted. Third-party plugins never get
/// `openedFilePaths`, `readFolders` or `writeFolders`: they read the files the user opens through Debrief.
pub fn sandbox_for_third_party(manifest: &Manifest, granted: &[Permission], home: &PluginHome) -> SandboxSpec {
    let has = |p: Permission| manifest.asks_for(p) && granted.contains(&p);
    let mut s = SandboxSpec::locked();
    s.memory_limit = Some(memory(manifest));
    s.network = has(Permission::Network);
    // Programs it starts run in the same container and job.
    s.children = has(Permission::RunPrograms);
    s.fs = FsAccess::Paths {
        read: vec![home.program_dir.clone()],
        write: vec![home.data_dir.clone()],
    };
    s.containment = Containment::Launcher {
        name: home.container.clone(),
        read: vec![home.program_dir.clone()],
        write: vec![home.data_dir.clone()],
    };
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::PermissionRequest;

    fn manifest(perms: &[Permission]) -> Manifest {
        Manifest {
            id: "p".into(),
            name: "P".into(),
            version: "1.0.0".into(),
            protocol: crate::PROTOCOL,
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
            document_importers: Vec::new(),
        }
    }

    #[test]
    fn plugins_are_locked_down_unless_granted() {
        let parser = sandbox_for(&manifest(&[]), &[], false);
        assert!(parser.low_integrity && !parser.network && !parser.children && parser.fs == FsAccess::Nothing);
        assert_eq!(parser.memory_limit, Some(DEFAULT_MEMORY_LIMIT));
        assert!(!parser.contained());
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
        let reads = sandbox_for(&manifest(&[Permission::ReadFolders]), &[Permission::ReadFolders], false);
        assert!(reads.low_integrity && matches!(&reads.fs, FsAccess::Paths { read, write } if read.len() == 1 && write.is_empty()));
        big.memory_limit = Some(1);
        assert_eq!(sandbox_for(&big, &[], false).memory_limit, Some(MIN_MEMORY_LIMIT));
    }

    #[test]
    fn third_party_plugins_are_contained_and_widened_only_by_network_and_programs() {
        let home = PluginHome {
            container: container_name("com.example.x"),
            program_dir: "/p/bin".into(),
            data_dir: "/p/data".into(),
        };
        let all = [
            Permission::Network,
            Permission::RunPrograms,
            Permission::OpenedFilePaths,
            Permission::WriteFolders,
        ];
        let strict = sandbox_for_third_party(&manifest(&all), &[], &home);
        assert!(strict.contained() && strict.low_integrity && !strict.network && !strict.children);
        assert_eq!(
            strict.fs,
            FsAccess::Paths {
                read: vec!["/p/bin".into()],
                write: vec!["/p/data".into()]
            }
        );
        let granted = sandbox_for_third_party(&manifest(&all), &all, &home);
        assert!(granted.network && granted.children && granted.contained() && granted.low_integrity);
        assert!(
            matches!(granted.fs, FsAccess::Paths { .. }),
            "never the user's files, whatever was granted"
        );
        // Revoking narrows it again.
        let revoked = sandbox_for_third_party(&manifest(&all), &[Permission::RunPrograms], &home);
        assert!(!revoked.network && revoked.children);
    }

    #[test]
    fn container_names_are_stable_and_valid() {
        let a = container_name("com.example.x");
        assert_eq!(a, container_name("com.example.x"));
        assert_ne!(a, container_name("com.example.y"));
        assert!(debrief_ipc::sandbox::Containment::valid_name(&a) && a.len() == 31, "{a}");
        assert_eq!(container_name(""), "debrief.plugin.cbf29ce484222325");
    }
}
