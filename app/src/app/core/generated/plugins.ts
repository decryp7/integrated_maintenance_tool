// Generated from the Rust wire types by `cargo test -p debrief-app --lib generated_types` (ts-rs; ADR 0016).
// Do not edit: change the Rust type and run the test with DEBRIEF_WRITE_TYPES=1.

export type PermissionDto = "openedFilePaths" | "runPrograms" | "writeFolders" | "network" | "readFolders";

export type PermissionRequestDto = { permission: PermissionDto, 
/**
 * The publisher's reason.
 */
reason: string, 
/**
 * What the user allows, in Debrief's words ("connect to the network").
 */
allows: string, };

export type SignatureKind = "trusted" | "unknownKey" | "unsigned";

export type SignatureDto = { kind: SignatureKind, 
/**
 * The trusted key's name, or the unknown key's id.
 */
signer: string | null, };

export type UpdateDto = { 
/**
 * The installed version.
 */
fromVersion: string, 
/**
 * Permissions the new version asks for that the installed one did not.
 */
added: Array<PermissionDto>, 
/**
 * Permissions it no longer asks for.
 */
removed: Array<PermissionDto>, };

export type RefusalDto = { 
/**
 * The package reason (`unsigned`, `unknownKey`, `incompatible`, `wrongTarget`, ...).
 */
reason: string, message: string, };

export type PackageReviewDto = { id: string, name: string, version: string, publisher: string, description: string, 
/**
 * The plugin protocol it was built for (`1.0`).
 */
protocol: string, targets: Array<string>, 
/**
 * Pass it back to `plugin_package_install`: exactly this content is installed.
 */
digest: string, signature: SignatureDto, 
/**
 * The permissions to approve (each can be left out).
 */
permissions: Array<PermissionRequestDto>, 
/**
 * Why it cannot be installed now; `None`: it can.
 */
refusal: RefusalDto | null, 
/**
 * Shown before installing (developer mode: unsigned or unknown key).
 */
warning: string | null, developerMode: boolean, 
/**
 * The installed version it replaces.
 */
update: UpdateDto | null, };

export type InstalledState = "ready" | "running" | "quarantined" | "disabled" | "pendingRestart" | "unavailable";

export type InstalledPluginDto = { id: string, name: string, version: string, publisher: string, description: string, signature: SignatureDto, 
/**
 * The warning accepted at install (developer mode).
 */
warning: string | null, 
/**
 * What it asks for.
 */
requested: Array<PermissionRequestDto>, 
/**
 * What the user granted.
 */
granted: Array<PermissionDto>, enabled: boolean, state: InstalledState, 
/**
 * Why it is unavailable or quarantined.
 */
problem: string | null, 
/**
 * Milliseconds since the Unix epoch.
 */
installedAtMs: number, };

export type DamagedPluginDto = { id: string, message: string, };

export type InstalledPluginsDto = { plugins: Array<InstalledPluginDto>, damaged: Array<DamagedPluginDto>, 
/**
 * Unsigned packages and unknown keys may be installed (Settings → Plugin trust).
 */
developerMode: boolean, 
/**
 * Third-party plugins are not loaded in this run (safe mode).
 */
safeMode: boolean, };
