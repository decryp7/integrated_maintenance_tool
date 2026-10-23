/**
 * Wire contract of the configuration diff (camelCase JSON). Mirrors `debrief_core::config::dto`,
 * `debrief_core::config::snapshot` and `app/src-tauri/src/config_diff.rs`; change both together.
 *
 * Commands:
 * - `config_diff_formats()` → {@link DiffFormats}
 * - `diff_configs({ leftPath, rightPath, options: DiffOptions })` → {@link DiffPage}
 * - `diff_snapshots({ leftDir, rightDir, compare: CompareOptions })` → {@link SnapshotDiff}
 * - `export_config_diff({ leftPath, rightPath, options, format: 'csv' | 'html', path })` → {@link DiffExported}
 */

export type ChangeKind = 'added' | 'removed' | 'changed' | 'unchanged';
export const CHANGE_KINDS: readonly ChangeKind[] = ['added', 'removed', 'changed', 'unchanged'];

export type ValueKind = 'section' | 'null' | 'bool' | 'integer' | 'float' | 'string' | 'binary' | 'list';

export interface ValueDto {
  kind: ValueKind;
  text: string;
  /** The text was cut (very long value). */
  clipped: boolean;
}

/** How two trees are compared. */
export interface CompareOptions {
  /** Globs over display paths (`*InstallDate*`) or regexes written `re:<pattern>`; case-insensitive. */
  ignore: string[];
  /** Compare paths without regard to case even when the format does not ask for it. */
  caseInsensitive: boolean;
}

export type ExpandMode = 'all' | 'changedOnly';

/** What part of a diff to show. */
export interface DiffView {
  kinds: ChangeKind[];
  search: string;
  expand: ExpandMode;
  /** Display paths whose expanded state is flipped from the default. */
  toggled: string[];
  offset: number;
  limit: number;
}

export interface DiffOptions {
  compare: CompareOptions;
  view: DiffView;
}

export interface DiffCounts {
  added: number;
  removed: number;
  changed: number;
  unchanged: number;
  ignored: number;
}

export interface SideInfo {
  path: string;
  /** Parser id: json, xml, ini, toml, yaml, registry, msinfo, or a plugin's. */
  format: string;
  nodes: number;
  truncated: boolean;
}

/** One row of the tree diff, in tree order. */
export interface DiffRow {
  /** Display path (`servers[0]/host`), unique within the diff. */
  path: string;
  name: string;
  depth: number;
  kind: ChangeKind;
  old: ValueDto | null;
  new: ValueDto | null;
  hasChanges: boolean;
  expandable: boolean;
  expanded: boolean;
  /** Shown only as the ancestor of a matching row. */
  context: boolean;
}

export interface DiffPage {
  left: SideInfo;
  right: SideInfo;
  caseInsensitive: boolean;
  counts: DiffCounts;
  /** Rows shown after filters and folding, across all pages. */
  total: number;
  offset: number;
  rows: DiffRow[];
  truncated: boolean;
}

export interface FormatInfo {
  id: string;
  name: string;
  extensions: string[];
}

export interface DiffFormats {
  formats: FormatInfo[];
  defaultIgnore: string[];
}

export type FileStatus = 'added' | 'removed' | 'changed' | 'unchanged' | 'unreadable';

export interface SnapshotFile {
  relPath: string;
  leftPath: string | null;
  rightPath: string | null;
  status: FileStatus;
  format: string | null;
  counts: DiffCounts | null;
  error: string | null;
}

export interface SnapshotDiff {
  left: string;
  right: string;
  files: SnapshotFile[];
  truncated: boolean;
}

export type DiffExportFormat = 'csv' | 'html';

export interface DiffExported {
  path: string;
  rows: number;
}
