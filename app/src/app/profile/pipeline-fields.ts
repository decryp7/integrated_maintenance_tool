import { PipelineStepKind } from '../core/profile.model';

export type FieldControl = 'text' | 'field' | 'number' | 'list' | 'level' | 'json' | 'plugin' | 'script';

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
    { key: 'fields', label: $localize`:@@profile.pipelineFields.sameValuesIn:Same values in`, control: 'list' },
    { key: 'window', label: $localize`:@@profile.pipelineFields.within:Within`, control: 'text', placeholder: '5s' },
  ],
  mask: [
    { key: 'field', label: $localize`:@@profile.pipelineFields.field:Field`, control: 'field' },
    { key: 'regex', label: $localize`:@@profile.pipelineFields.regex:Regex`, control: 'text', placeholder: 'password=\\S+' },
    { key: 'replace', label: $localize`:@@profile.pipelineFields.replaceWith:Replace with`, control: 'text', placeholder: '***  ($1 = first group)' },
  ],
  remove_fields: [{ key: 'fields', label: $localize`:@@profile.pipelineFields.fieldsToRemove:Fields to remove`, control: 'list' }],
  truncate: [
    { key: 'field', label: $localize`:@@profile.pipelineFields.field:Field`, control: 'field' },
    { key: 'max', label: $localize`:@@profile.pipelineFields.maxCharacters:Max characters`, control: 'number' },
  ],
  rename: [
    { key: 'from', label: $localize`:@@profile.pipelineFields.from:From`, control: 'field' },
    { key: 'to', label: $localize`:@@profile.pipelineFields.to:To`, control: 'text' },
  ],
  set: [
    { key: 'field', label: $localize`:@@profile.pipelineFields.field:Field`, control: 'field' },
    { key: 'value', label: $localize`:@@profile.pipelineFields.value:Value`, control: 'text' },
  ],
  set_level: [{ key: 'level', label: $localize`:@@profile.pipelineFields.level:Level`, control: 'level' }],
  lookup: [
    { key: 'field', label: $localize`:@@profile.pipelineFields.lookUpTheValueOf:Look up the value of`, control: 'field' },
    { key: 'table', label: $localize`:@@profile.pipelineFields.tableJsonKeyFieldValue:Table (JSON: {"KEY": {"Field": "value"}})`, control: 'json' },
    { key: 'file', label: $localize`:@@profile.pipelineFields.orCsvFile:or CSV file`, control: 'text' },
    { key: 'key', label: $localize`:@@profile.pipelineFields.keyColumnOfTheFile:Key column of the file`, control: 'text' },
  ],
  parse_json: [
    { key: 'field', label: $localize`:@@profile.pipelineFields.field:Field`, control: 'field' },
    { key: 'prefix', label: $localize`:@@profile.pipelineFields.prefixForNewFields:Prefix for new fields`, control: 'text' },
  ],
  parse_kv: [
    { key: 'field', label: $localize`:@@profile.pipelineFields.field:Field`, control: 'field' },
    { key: 'prefix', label: $localize`:@@profile.pipelineFields.prefixForNewFields:Prefix for new fields`, control: 'text' },
  ],
  shift_time: [{ key: 'by', label: $localize`:@@profile.pipelineFields.shiftBy:Shift by`, control: 'text', placeholder: '+2.4s, -150ms, 1m' }],
  calc: [
    { key: 'field', label: $localize`:@@profile.pipelineFields.newField:New field`, control: 'text', placeholder: 'ElapsedSec' },
    { key: 'expr', label: $localize`:@@profile.pipelineFields.expression:Expression`, control: 'text', placeholder: 'round(ElapsedMs / 1000, 1)' },
  ],
  script: [{ key: 'script', label: $localize`:@@profile.pipelineFields.script:Script`, control: 'script' }],
  plugin: [{ key: 'id', label: $localize`:@@profile.pipelineFields.transformPlugin:Transform plugin`, control: 'plugin' }],
};
