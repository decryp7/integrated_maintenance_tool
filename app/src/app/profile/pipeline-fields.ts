import { PipelineStepKind } from '../core/profile.model';

export type FieldControl = 'text' | 'field' | 'number' | 'list' | 'level' | 'json' | 'plugin';

export interface StepField {
  /** Key in the TOML table of the step. */
  key: string;
  label: string;
  control: FieldControl;
  placeholder?: string;
}

/** Config form of each step kind (the `when` conditions are edited separately for every kind). */
export const STEP_FIELDS: Record<PipelineStepKind, StepField[]> = {
  drop: [],
  keep: [],
  dedupe: [
    { key: 'fields', label: 'Same values in', control: 'list' },
    { key: 'window', label: 'Within', control: 'text', placeholder: '5s' },
  ],
  mask: [
    { key: 'field', label: 'Field', control: 'field' },
    { key: 'regex', label: 'Regex', control: 'text', placeholder: 'password=\\S+' },
    { key: 'replace', label: 'Replace with', control: 'text', placeholder: '***  ($1 = first group)' },
  ],
  remove_fields: [{ key: 'fields', label: 'Fields to remove', control: 'list' }],
  truncate: [
    { key: 'field', label: 'Field', control: 'field' },
    { key: 'max', label: 'Max characters', control: 'number' },
  ],
  rename: [
    { key: 'from', label: 'From', control: 'field' },
    { key: 'to', label: 'To', control: 'text' },
  ],
  set: [
    { key: 'field', label: 'Field', control: 'field' },
    { key: 'value', label: 'Value', control: 'text' },
  ],
  set_level: [{ key: 'level', label: 'Level', control: 'level' }],
  lookup: [
    { key: 'field', label: 'Look up the value of', control: 'field' },
    { key: 'table', label: 'Table (JSON: {"KEY": {"Field": "value"}})', control: 'json' },
    { key: 'file', label: 'or CSV file', control: 'text' },
    { key: 'key', label: 'Key column of the file', control: 'text' },
  ],
  parse_json: [
    { key: 'field', label: 'Field', control: 'field' },
    { key: 'prefix', label: 'Prefix for new fields', control: 'text' },
  ],
  parse_kv: [
    { key: 'field', label: 'Field', control: 'field' },
    { key: 'prefix', label: 'Prefix for new fields', control: 'text' },
  ],
  shift_time: [{ key: 'by', label: 'Shift by', control: 'text', placeholder: '+2.4s, -150ms, 1m' }],
  plugin: [{ key: 'id', label: 'Transform plugin', control: 'plugin' }],
};
