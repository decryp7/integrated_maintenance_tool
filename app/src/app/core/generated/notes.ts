// Generated from the Rust wire types by `cargo test -p debrief-app --lib generated_types` (ts-rs; ADR 0016).
// Do not edit: change the Rust type and run the test with DEBRIEF_WRITE_TYPES=1.

export type NoteRegion = { x: number, y: number, w: number, h: number, };

export type NoteImageRef = { key: string, name: string, path: string | null, width: number, height: number, };

export type NoteAnchor = { "kind": "event", eventId: number, ts: number | null, source: string, excerpt: string, } | { "kind": "instant", at: number, } | { "kind": "range", from: number, to: number, } | { "kind": "source", path: string, } | { "kind": "marker", name: string, from: number, to: number | null, } | { "kind": "chartPoint", chart: string, series: string | null, at: number, value: number | null, } | { "kind": "project" } | { "kind": "image", image: NoteImageRef, regions: Array<NoteRegion>, };

export type NoteSeverity = "info" | "warning" | "problem";

export type Note = { id: string, anchor: NoteAnchor, text: string, tags: Array<string>, severity: NoteSeverity, author: string, createdMs: number, updatedMs: number, };

export type NoteDraft = { anchor: NoteAnchor, text: string, tags?: Array<string>, severity?: NoteSeverity, };

export type NoteChange = { text?: string, tags?: Array<string>, severity?: NoteSeverity, regions?: Array<NoteRegion>, };

export type NoteRecord = { version: number, notes: Array<Note>, };

export type NoteItem = { resolved: boolean, id: string, anchor: NoteAnchor, text: string, tags: Array<string>, severity: NoteSeverity, author: string, createdMs: number, updatedMs: number, };

export type NotesList = { notes: Array<NoteItem>, maxNotes: number, maxTextBytes: number, maxTags: number, maxRegions: number, };

export type NotesRestore = { count: number, warning: string | null, };

export type NoteImage = { 
/**
 * The content key notes name it by.
 */
key: string, name: string, 
/**
 * The file it was opened from (none when it came from the project's copy).
 */
path: string | null, 
/**
 * The original's size: note regions are in these pixels.
 */
width: number, height: number, 
/**
 * The size of the copy in `data_url`.
 */
viewWidth: number, viewHeight: number, 
/**
 * `data:image/png;base64,...` of the clean copy.
 */
dataUrl: string, };
