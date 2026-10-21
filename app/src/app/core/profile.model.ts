import type { SpanCfgToml } from './api.types';

/**
 * Shape of a parsing profile (TOML), mirroring crates/imt-core/src/default_profile.toml.
 * Keys are snake_case as written in the file. Unknown keys are preserved by the editor.
 */

export type ConditionOp =
  | 'eq'
  | 'ne'
  | 'contains'
  | 'notContains'
  | 'matches'
  | 'exists'
  | 'missing'
  | 'gt'
  | 'lt'
  | 'in';

export const CONDITION_OPS: ConditionOp[] = ['eq', 'ne', 'contains', 'notContains', 'matches', 'exists', 'missing', 'gt', 'lt', 'in'];

export interface Condition {
  field: string;
  op: ConditionOp;
  value?: string;
}

export type ExtractFieldType = 'string' | 'number' | 'integer' | 'bool' | 'bytes' | 'duration';
export const EXTRACT_FIELD_TYPES: ExtractFieldType[] = ['string', 'number', 'integer', 'bool', 'bytes', 'duration'];

export interface ExtractField {
  type: ExtractFieldType;
  unit?: string;
  unit_from?: string;
  to?: string;
  [key: string]: unknown;
}

export interface ExtractRule {
  name: string;
  from?: string;
  regex: string;
  when?: Condition[];
  fields?: Record<string, ExtractField>;
  [key: string]: unknown;
}

export interface MarkerRule {
  kind: string;
  label?: string;
  lifecycle?: boolean;
  color?: string;
  when?: Condition[];
  [key: string]: unknown;
}

/** A `[[marker_override]]` entry: every key but `kind` is optional. */
export interface MarkerOverride {
  kind: string;
  enabled?: boolean;
  label?: string;
  color?: string;
  icon?: string;
  badge?: string;
  shape?: string;
  description?: string;
}

export interface ChartRule {
  id: string;
  title?: string;
  y: string;
  bucket?: string;
  split_by?: string[];
  type?: 'line' | 'bar' | 'stacked';
  unit?: string;
  when?: Condition[];
  [key: string]: unknown;
}

export type PipelineStepKind =
  | 'drop'
  | 'keep'
  | 'mask'
  | 'dedupe'
  | 'remove_fields'
  | 'rename'
  | 'set'
  | 'set_level'
  | 'truncate'
  | 'parse_json'
  | 'parse_kv'
  | 'lookup'
  | 'shift_time'
  | 'calc'
  | 'script'
  | 'plugin';
export const PIPELINE_STEP_KINDS: PipelineStepKind[] = [
  'drop',
  'keep',
  'mask',
  'dedupe',
  'remove_fields',
  'rename',
  'set',
  'set_level',
  'truncate',
  'parse_json',
  'parse_kv',
  'lookup',
  'shift_time',
  'calc',
  'script',
  'plugin',
];

/** One step of the ordered `[[pipeline]]`; only the keys relevant to `step` are used. */
export interface PipelineStep {
  step: PipelineStepKind;
  when?: Condition[];
  field?: string;
  regex?: string;
  replace?: string;
  fields?: string[];
  window?: string;
  from?: string;
  to?: string;
  value?: string;
  level?: string;
  max?: number;
  id?: string;
  /** parse_json / parse_kv: prepended to the produced field names. */
  prefix?: string;
  /** lookup: inline table `{ KEY = { Field = value } }`, or a CSV `file` with the `key` column. */
  table?: Record<string, Record<string, unknown>>;
  file?: string;
  key?: string;
  /** shift_time: "+2.4s", "-150ms", "1m". */
  by?: string;
  /** calc: expression computing `field`, e.g. "ElapsedMs / 1000" (parsed and evaluated by the engine). */
  expr?: string;
  /** script: Rhai source run on every event (compiled, sandboxed and evaluated only by the engine). */
  script?: string;
  /** The engine skips the step (kept in the TOML so it can be re-enabled). */
  disabled?: boolean;
  [key: string]: unknown;
}

/** A `[[span]]` rule: runs between a start and a stop marker (default_profile.toml documents every key). */
export type SpanRuleToml = SpanCfgToml;

export interface ProfileModel {
  name?: string;
  files?: { include?: string[]; exclude?: string[]; [key: string]: unknown };
  instance?: { key?: string[]; candidates?: string[]; [key: string]: unknown };
  extract?: ExtractRule[];
  marker?: MarkerRule[];
  /** Switches off or restyles a plugin or built-in marker kind (default_profile.toml `[[marker_override]]`). */
  marker_override?: MarkerOverride[];
  chart?: ChartRule[];
  pipeline?: PipelineStep[];
  /** Runs between lifecycle markers; absent = the default rule, `[]` = none. */
  span?: SpanRuleToml[];
  [key: string]: unknown;
}

/** Profile regexes are Rust syntax; JS needs (?<name>…) instead of (?P<name>…) and no inline flags. */
export function toJsRegex(pattern: string, extraFlags = ''): RegExp {
  let flags = extraFlags;
  let src = pattern.replace(/\(\?P</g, '(?<');
  const inline = /^\(\?([a-z]+)\)/.exec(src);
  if (inline) {
    src = src.slice(inline[0].length);
    for (const f of inline[1] ?? '') if ('ims'.includes(f) && !flags.includes(f)) flags += f;
  }
  return new RegExp(src, flags);
}
