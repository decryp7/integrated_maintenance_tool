/**
 * Wire contract of the Settings screen (camelCase JSON), mirroring the Rust DTOs in
 * `crates/imt-core/src/settings.rs` (schema) and `app/src-tauri/src/settings.rs` (commands).
 * Commands: `settings_sections`, `set_settings`, `reset_settings`, `export_settings`, `import_settings`.
 */

/** A stored value: durations are milliseconds, sizes are bytes. */
export type SettingValue = boolean | number | string | string[];

export interface EnumOption {
  value: string;
  label: string;
}

/** The type of a field and its limits. */
export type FieldKind =
  | { type: 'bool' }
  | { type: 'int'; min: number; max: number; step: number; unit: string | null }
  | { type: 'float'; min: number; max: number; step: number; unit: string | null }
  | { type: 'text'; maxLen: number; pattern: string | null }
  | { type: 'enum'; options: EnumOption[] }
  | { type: 'path'; dir: boolean }
  | { type: 'duration'; minMs: number; maxMs: number }
  | { type: 'size'; minBytes: number; maxBytes: number }
  | { type: 'stringList'; maxItems: number };

export interface SettingField {
  key: string;
  label: string;
  /** Help text shown under the control. */
  description: string;
  kind: FieldKind;
  default: SettingValue;
  /** The change takes effect after IMT restarts. */
  restartRequired: boolean;
  /** Hidden unless "Show advanced" is on. */
  advanced: boolean;
}

export type SectionOwner = { kind: 'core' } | { kind: 'app' } | { kind: 'plugin'; id: string };

/** Where the navigation lists a section. */
export type SectionGroup = 'app' | 'diagnostics' | 'plugins' | 'ai' | 'advanced';

export type SectionStatus = 'active' | 'disabled' | 'quarantined' | 'notInstalled' | 'safeMode';

/** One section with its current values (every field present) and whether it can be changed. */
export interface SettingsSection {
  id: string;
  title: string;
  description: string;
  owner: SectionOwner;
  fields: SettingField[];
  group: SectionGroup;
  values: Record<string, SettingValue>;
  status: SectionStatus;
  readOnly: boolean;
  /** Why the section is read-only, or another hint. */
  note: string | null;
}

export interface SettingsOverview {
  sections: SettingsSection[];
  /** A change made in this run takes effect after a restart. */
  restartPending: boolean;
  safeMode: boolean;
}

/** An engine validation message; `key` is empty for problems with the whole section. */
export interface FieldError {
  key: string;
  message: string;
}

export interface SetResult {
  /** False when validation failed: nothing changed and `errors` says why. */
  applied: boolean;
  restartRequired: boolean;
  errors: FieldError[];
  /** The section's values now. */
  values: Record<string, SettingValue>;
}

export interface SectionErrors {
  section: string;
  errors: FieldError[];
}

export interface ImportResult {
  applied: string[];
  /** Kept for later (not installed, disabled or quarantined). */
  kept: string[];
  rejected: SectionErrors[];
  restartRequired: boolean;
}
