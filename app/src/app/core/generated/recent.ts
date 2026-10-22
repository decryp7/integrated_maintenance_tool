// Generated from the Rust wire types by `cargo test -p imt-app --lib generated_types` (ts-rs; ADR 0016).
// Do not edit: change the Rust type and run the test with IMT_WRITE_TYPES=1.

export type RecentFile = { path: string, 
/**
 * When it was last opened, in milliseconds since the Unix epoch.
 */
openedAt: number, };

export type RecentProject = { path: string, name: string, 
/**
 * When it was last opened or saved, in milliseconds since the Unix epoch.
 */
openedAt: number, };

export type RecentLists = { files: Array<RecentFile>, projects: Array<RecentProject>, };

export type RecentKind = "files" | "projects";
