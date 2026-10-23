// Generated from the Rust wire types by `cargo test -p debrief-app --lib generated_types` (ts-rs; ADR 0016).
// Do not edit: change the Rust type and run the test with DEBRIEF_WRITE_TYPES=1.

export type PickKind = "openFile" | "openFiles" | "openFolder" | "openFolders" | "save";

export type PickFilter = { name: string, extensions: Array<string>, };

export type PickRequest = { kind: PickKind, title: string, filters?: Array<PickFilter>, 
/**
 * Suggested file name of a save dialog (a name, not a path).
 */
defaultName?: string, };
