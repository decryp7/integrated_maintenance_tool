import { PipelineStat, PluginInfo } from '../core/api.types';
import { formatCount } from '../core/format';
import { Condition, ConditionOp, PipelineStep, PipelineStepKind } from '../core/profile.model';

export type StepGroup = 'Filter' | 'Clean' | 'Enrich' | 'Extend';
export const STEP_GROUPS: StepGroup[] = ['Filter', 'Clean', 'Enrich', 'Extend'];

export interface StepKindInfo {
  kind: PipelineStepKind;
  group: StepGroup;
  label: string;
  /** Name of an `<imt-icon>`. */
  icon: string;
  /** One sentence for the palette tooltip. */
  blurb: string;
}

/** Everything the activity palette offers, in display order. */
export const STEP_CATALOG: StepKindInfo[] = [
  { kind: 'drop', group: 'Filter', label: 'Drop', icon: 'trash', blurb: 'Remove events that match the conditions.' },
  { kind: 'keep', group: 'Filter', label: 'Keep only', icon: 'check', blurb: 'Remove every event that does NOT match the conditions.' },
  { kind: 'dedupe', group: 'Filter', label: 'Collapse repeats', icon: 'copy', blurb: 'Merge identical events within a time window (adds RepeatCount).' },
  { kind: 'mask', group: 'Clean', label: 'Mask text', icon: 'minus', blurb: 'Replace text that matches a regex, e.g. to hide secrets.' },
  { kind: 'remove_fields', group: 'Clean', label: 'Remove fields', icon: 'close', blurb: 'Delete properties from events.' },
  { kind: 'truncate', group: 'Clean', label: 'Truncate', icon: 'sortAsc', blurb: 'Cut a long text field to a maximum length.' },
  { kind: 'rename', group: 'Clean', label: 'Rename field', icon: 'profile', blurb: 'Give a property a different name.' },
  { kind: 'set', group: 'Enrich', label: 'Set field', icon: 'plus', blurb: 'Set a property (or message) to a fixed value.' },
  { kind: 'set_level', group: 'Enrich', label: 'Set level', icon: 'alert', blurb: 'Change the severity of matching events.' },
  { kind: 'lookup', group: 'Enrich', label: 'Look up', icon: 'search', blurb: 'Add properties from a table keyed by a field value.' },
  { kind: 'parse_json', group: 'Enrich', label: 'Parse JSON', icon: 'columns', blurb: 'Turn a JSON text into fields.' },
  { kind: 'parse_kv', group: 'Enrich', label: 'Parse key=value', icon: 'group', blurb: 'Turn key=value pairs in a text into fields.' },
  { kind: 'shift_time', group: 'Enrich', label: 'Shift time', icon: 'refresh', blurb: 'Correct clock skew by adding or subtracting a duration.' },
  { kind: 'plugin', group: 'Extend', label: 'Plugin', icon: 'puzzle', blurb: 'Run a transform provided by a backend plugin.' },
];

export function kindInfo(kind: string): StepKindInfo | undefined {
  return STEP_CATALOG.find((k) => k.kind === kind);
}

/** A new step with sensible defaults, ready to be edited. */
export function defaultStep(kind: PipelineStepKind, pluginId = ''): PipelineStep {
  const when: Condition[] = [{ field: '@message', op: 'contains', value: '' }];
  switch (kind) {
    case 'drop':
    case 'keep':
      return { step: kind, when };
    case 'dedupe':
      return { step: kind, fields: ['@message'], window: '5s' };
    case 'mask':
      return { step: kind, field: '@message', regex: '', replace: '***' };
    case 'remove_fields':
      return { step: kind, fields: [''] };
    case 'truncate':
      return { step: kind, field: '@message', max: 2000 };
    case 'rename':
      return { step: kind, from: '', to: '' };
    case 'set':
      return { step: kind, field: '', value: '' };
    case 'set_level':
      return { step: kind, level: 'Warning', when };
    case 'lookup':
      return { step: kind, field: '', table: {} };
    case 'parse_json':
    case 'parse_kv':
      return { step: kind, field: '@message' };
    case 'shift_time':
      return { step: kind, by: '+1s' };
    case 'plugin':
      return { step: kind, id: pluginId };
  }
}

const OP_TEXT: Record<ConditionOp, string> = {
  eq: 'equals',
  ne: 'is not',
  contains: 'contains',
  notContains: 'does not contain',
  matches: 'matches',
  exists: 'exists',
  missing: 'is missing',
  gt: '>',
  lt: '<',
  in: 'is one of',
};

/** "SourceContext contains HealthChecks" */
export function describeCondition(c: Condition): string {
  const op = OP_TEXT[c.op] ?? c.op;
  return c.op === 'exists' || c.op === 'missing' ? `${c.field || '(field)'} ${op}` : `${c.field || '(field)'} ${op} ${c.value ?? ''}`.trimEnd();
}

/** " where A contains x and B equals y", or "" without conditions. */
export function describeWhen(when: Condition[] | undefined): string {
  return when?.length ? ` where ${when.map(describeCondition).join(' and ')}` : '';
}

const list = (xs: string[] | undefined) => (xs ?? []).filter(Boolean).join(', ') || '(none)';

/** Plain-English description of a step, e.g. "Drop events where SourceContext contains HealthChecks". */
export function summarizeStep(s: PipelineStep): string {
  const where = describeWhen(s.when);
  const field = s.field || '@message';
  switch (s.step) {
    case 'drop':
      return `Drop ${where ? 'events' : 'every event'}${where}`;
    case 'keep':
      return `Keep only ${where ? 'events' : 'every event'}${where}`;
    case 'dedupe':
      return `Collapse repeated events with the same ${list(s.fields)} within ${s.window || '5s'}${where}`;
    case 'mask':
      return `Mask text matching /${s.regex ?? ''}/ in ${field} with "${s.replace ?? ''}"${where}`;
    case 'remove_fields':
      return `Remove fields ${list(s.fields)}${where}`;
    case 'truncate':
      return `Truncate ${field} to ${s.max ?? '?'} characters${where}`;
    case 'rename':
      return `Rename field ${s.from || '(from)'} to ${s.to || '(to)'}${where}`;
    case 'set':
      return `Set ${s.field || '(field)'} to "${s.value ?? ''}"${where}`;
    case 'set_level':
      return `Set level to ${s.level ?? '?'}${where}`;
    case 'lookup': {
      const source = s.file ? `file ${s.file}` : `a table of ${Object.keys(s.table ?? {}).length} keys`;
      return `Look up ${s.field || '(field)'} in ${source}${where}`;
    }
    case 'parse_json':
      return `Parse JSON in ${field} into fields${s.prefix ? ` prefixed "${s.prefix}"` : ''}${where}`;
    case 'parse_kv':
      return `Parse key=value pairs in ${field} into fields${s.prefix ? ` prefixed "${s.prefix}"` : ''}${where}`;
    case 'shift_time':
      return `Shift timestamps by ${s.by || '?'}${where}`;
    case 'plugin':
      return `Run transform plugin "${s.id || '(choose one)'}"${where}`;
    default:
      return `Unknown step "${String(s.step)}"`;
  }
}

/** Returns a copy with the element moved; out-of-range targets are clamped. */
export function moveItem<T>(items: readonly T[], from: number, to: number): T[] {
  const next = [...items];
  if (from < 0 || from >= next.length) return next;
  const [x] = next.splice(from, 1);
  next.splice(Math.max(0, Math.min(next.length, to)), 0, x);
  return next;
}

/** Copy of a step inserted right after the original. */
export function duplicateStep(steps: readonly PipelineStep[], index: number): PipelineStep[] {
  if (index < 0 || index >= steps.length) return [...steps];
  return [...steps.slice(0, index + 1), structuredClone(steps[index]), ...steps.slice(index + 1)];
}

/** Plugins that offer pipeline transforms: `contributes` has an entry starting with "transform". */
export function transformPlugins(plugins: readonly PluginInfo[]): PluginInfo[] {
  return plugins.filter((p) => p.contributes.some((c) => c === 'transform' || c.startsWith('transform:')));
}

export interface StepStatus {
  state: 'ok' | 'stale' | 'disabled';
  text: string;
}

/** "31,204 in → 18,950 out (−12,254)" */
export function formatStats(input: number, output: number): string {
  const delta = output - input;
  const change = delta === 0 ? '±0' : `${delta < 0 ? '−' : '+'}${formatCount(Math.abs(delta))}`;
  return `${formatCount(input)} in → ${formatCount(output)} out (${change})`;
}

/**
 * What to show on a step's badge. Stats come from the last import and are only trusted while the
 * edited profile equals the saved one (`dirty` false) and the step has a matching entry.
 */
export function stepStatus(step: PipelineStep, index: number, stats: readonly PipelineStat[], dirty: boolean): StepStatus {
  if (step.disabled) return { state: 'disabled', text: 'disabled' };
  const stat = dirty ? undefined : stats.find((s) => s.index === index && s.step === step.step);
  return stat ? { state: 'ok', text: formatStats(stat.input, stat.output) } : { state: 'stale', text: 'not run yet' };
}
