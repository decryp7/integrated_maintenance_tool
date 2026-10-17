import { ChartRule, Condition, ExtractField, ExtractFieldType, ExtractRule, MarkerRule, PipelineStep, ProfileModel } from '../core/profile.model';
import { RecordView } from '../core/api.types';
import { valueToString } from '../core/format';

/** Text of a record field the way conditions see it (pseudo-fields included). */
export function recordFieldText(record: RecordView, field: string): string | null {
  switch (field) {
    case '@message':
      return record.message;
    case '@level':
      return record.level;
    case '@template':
      return record.template ?? null;
    case '@exception':
      return record.exception ?? null;
    case '@source':
      return record.sourceName;
    case '@line':
      return String(record.line);
    default: {
      const v = record.fields[field];
      return v === undefined ? null : valueToString(v);
    }
  }
}

/** The "events like this" conditions for a record: its logger (SourceContext) plus its message template. */
export function likeThisConditions(record: RecordView): Condition[] {
  const out: Condition[] = [];
  const context = recordFieldText(record, 'SourceContext');
  if (context) out.push({ field: 'SourceContext', op: 'eq', value: context });
  if (record.template) out.push({ field: '@template', op: 'eq', value: record.template });
  else out.push({ field: '@message', op: 'eq', value: record.message });
  return out;
}

/** Conditions for "events with this value of the field". */
export function fieldValueConditions(field: string, value: string): Condition[] {
  return [{ field, op: 'eq', value }];
}

/** Plain-English rendering of conditions ("SourceContext eq X and @template eq Y"). */
export function describeConditions(conditions: readonly Condition[]): string {
  if (!conditions.length) return 'every event';
  return conditions.map((c) => (c.value === undefined ? `${c.field} ${c.op}` : `${c.field} ${c.op} "${c.value}"`)).join(' and ');
}

/** Conditions that can run as a query filter (the query language lacks only `in`). */
export function asFilters(conditions: readonly Condition[]): { field: string; op: Exclude<Condition['op'], 'in'>; value?: string }[] {
  return conditions.filter((c): c is Condition & { op: Exclude<Condition['op'], 'in'> } => c.op !== 'in').map((c) => ({ field: c.field, op: c.op, value: c.value }));
}

export function dropStep(when: Condition[]): PipelineStep {
  return { step: 'drop', when };
}

export function maskStep(regex: string, replace: string, field = '@message'): PipelineStep {
  return { step: 'mask', field, regex, replace };
}

export function addPipelineStep(model: ProfileModel, step: PipelineStep): void {
  model.pipeline = [...(model.pipeline ?? []), step];
}

export interface ExtractDraft {
  name: string;
  field: string;
  regex: string;
  type: ExtractFieldType;
  unit?: string;
  to?: string;
  when: Condition[];
}

export function extractRule(d: ExtractDraft): ExtractRule {
  const field: ExtractField = { type: d.type };
  if (d.unit) field.unit = d.unit;
  if (d.to && (d.type === 'bytes' || d.type === 'duration')) field.to = d.to;
  return { name: `extract-${d.field}`.toLowerCase(), from: '@message', regex: d.regex, ...(d.when.length ? { when: d.when } : {}), fields: { [d.field]: field } };
}

export function addExtractRule(model: ProfileModel, rule: ExtractRule): void {
  const taken = new Set((model.extract ?? []).map((r) => r.name));
  let name = rule.name;
  for (let i = 2; taken.has(name); i++) name = `${rule.name}-${i}`;
  model.extract = [...(model.extract ?? []), { ...rule, name }];
}

/** `avg(field)` chart split by the instance key fields. */
export function chartForField(field: string, splitBy: string[]): ChartRule {
  return { id: `${field}-chart`.toLowerCase(), title: field, y: `avg(${field})`, split_by: splitBy, type: 'line' };
}

export function addChartRule(model: ProfileModel, chart: ChartRule): void {
  const taken = new Set((model.chart ?? []).map((c) => c.id));
  let id = chart.id;
  for (let i = 2; taken.has(id); i++) id = `${chart.id}-${i}`;
  model.chart = [...(model.chart ?? []), { ...chart, id }];
}

export interface MarkerDraft {
  kind: string;
  label: string;
  color: string;
  lifecycle: boolean;
  when: Condition[];
}

export function markerRule(d: MarkerDraft): MarkerRule {
  return { kind: d.kind, label: d.label || d.kind, color: d.color, ...(d.lifecycle ? { lifecycle: true } : {}), when: d.when };
}

export function addMarkerRule(model: ProfileModel, rule: MarkerRule): void {
  model.marker = [...(model.marker ?? []), rule];
}

/** Makes a safe marker kind / id from free text ("Payment timeout!" -> "payment-timeout"). */
export function slug(text: string): string {
  return text.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '') || 'rule';
}
