import { PipelineStat, PluginInfo } from '../core/api.types';
import { formatCount } from '../core/format';
import { plural } from '../i18n/locale';
import { Condition, ConditionOp, PipelineStep, PipelineStepKind } from '../core/profile.model';

export type StepGroup = 'Filter' | 'Clean' | 'Enrich' | 'Extend';
export const STEP_GROUPS: StepGroup[] = ['Filter', 'Clean', 'Enrich', 'Extend'];

/** Display names of the palette groups (the `StepGroup` values are keys). */
export const STEP_GROUP_LABELS: Record<StepGroup, string> = {
  Filter: $localize`:Group of pipeline step kinds|@@profile.pipeline.group.filter:Filter`,
  Clean: $localize`:Group of pipeline step kinds|@@profile.pipeline.group.clean:Clean`,
  Enrich: $localize`:Group of pipeline step kinds|@@profile.pipeline.group.enrich:Enrich`,
  Extend: $localize`:Group of pipeline step kinds|@@profile.pipeline.group.extend:Extend`,
};

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
  { kind: 'drop', group: 'Filter', label: $localize`:Pipeline step kind|@@profile.pipeline.step.drop.label:Drop`, icon: 'trash', blurb: $localize`:@@profile.pipeline.step.drop.blurb:Remove events that match the conditions.` },
  { kind: 'keep', group: 'Filter', label: $localize`:Pipeline step kind|@@profile.pipeline.step.keep.label:Keep only`, icon: 'check', blurb: $localize`:@@profile.pipeline.step.keep.blurb:Remove every event that does NOT match the conditions.` },
  { kind: 'dedupe', group: 'Filter', label: $localize`:Pipeline step kind|@@profile.pipeline.step.dedupe.label:Collapse repeats`, icon: 'copy', blurb: $localize`:@@profile.pipeline.step.dedupe.blurb:Merge identical events within a time window (adds RepeatCount).` },
  { kind: 'mask', group: 'Clean', label: $localize`:Pipeline step kind|@@profile.pipeline.step.mask.label:Mask text`, icon: 'minus', blurb: $localize`:@@profile.pipeline.step.mask.blurb:Replace text that matches a regex, e.g. to hide secrets.` },
  { kind: 'remove_fields', group: 'Clean', label: $localize`:Pipeline step kind|@@profile.pipeline.step.removeFields.label:Remove fields`, icon: 'close', blurb: $localize`:@@profile.pipeline.step.removeFields.blurb:Delete properties from events.` },
  { kind: 'truncate', group: 'Clean', label: $localize`:Pipeline step kind|@@profile.pipeline.step.truncate.label:Truncate`, icon: 'sortAsc', blurb: $localize`:@@profile.pipeline.step.truncate.blurb:Cut a long text field to a maximum length.` },
  { kind: 'rename', group: 'Clean', label: $localize`:Pipeline step kind|@@profile.pipeline.step.rename.label:Rename field`, icon: 'profile', blurb: $localize`:@@profile.pipeline.step.rename.blurb:Give a property a different name.` },
  { kind: 'set', group: 'Enrich', label: $localize`:Pipeline step kind|@@profile.pipeline.step.set.label:Set field`, icon: 'plus', blurb: $localize`:@@profile.pipeline.step.set.blurb:Set a property (or message) to a fixed value.` },
  { kind: 'set_level', group: 'Enrich', label: $localize`:Pipeline step kind|@@profile.pipeline.step.setLevel.label:Set level`, icon: 'alert', blurb: $localize`:@@profile.pipeline.step.setLevel.blurb:Change the severity of matching events.` },
  { kind: 'lookup', group: 'Enrich', label: $localize`:Pipeline step kind|@@profile.pipeline.step.lookup.label:Look up`, icon: 'search', blurb: $localize`:@@profile.pipeline.step.lookup.blurb:Add properties from a table keyed by a field value.` },
  { kind: 'parse_json', group: 'Enrich', label: $localize`:Pipeline step kind|@@profile.pipeline.step.parseJson.label:Parse JSON`, icon: 'columns', blurb: $localize`:@@profile.pipeline.step.parseJson.blurb:Turn a JSON text into fields.` },
  { kind: 'parse_kv', group: 'Enrich', label: $localize`:Pipeline step kind|@@profile.pipeline.step.parseKv.label:Parse key=value`, icon: 'group', blurb: $localize`:@@profile.pipeline.step.parseKv.blurb:Turn key=value pairs in a text into fields.` },
  { kind: 'shift_time', group: 'Enrich', label: $localize`:Pipeline step kind|@@profile.pipeline.step.shiftTime.label:Shift time`, icon: 'refresh', blurb: $localize`:@@profile.pipeline.step.shiftTime.blurb:Correct clock skew by adding or subtracting a duration.` },
  { kind: 'calc', group: 'Enrich', label: $localize`:Pipeline step kind|@@profile.pipeline.step.calc.label:Calculated field`, icon: 'chart', blurb: $localize`:@@profile.pipeline.step.calc.blurb:Compute a new field from an expression, e.g. ElapsedMs / 1000.` },
  { kind: 'script', group: 'Extend', label: $localize`:Pipeline step kind|@@profile.pipeline.step.script.label:Script`, icon: 'file', blurb: $localize`:@@profile.pipeline.step.script.blurb:Run a small script on every event for custom transforms (evaluated by the engine).` },
  { kind: 'plugin', group: 'Extend', label: $localize`:Pipeline step kind|@@profile.pipeline.step.plugin.label:Plugin`, icon: 'puzzle', blurb: $localize`:@@profile.pipeline.step.plugin.blurb:Run a transform provided by a backend plugin.` },
];

export function kindInfo(kind: string): StepKindInfo | undefined {
  return STEP_CATALOG.find((k) => k.kind === kind);
}

/** Starting point of a new script step: names the variables and changes nothing by itself. */
export const SCRIPT_TEMPLATE = '// message, level, timestamp, fields and source describe the event.\nif fields.ElapsedMs > 5000 { fields.Slow = true; }\n';

/** What a script sees and can call, for the editor's help text. */
export const SCRIPT_HELP = $localize`:@@profile.pipeline.scriptHelp:Runs once per event. Variables: message, level (e.g. "Warning"), timestamp (ms or ()), fields (fields.Name or fields["odd name"]; a missing field is ()) and source (read-only). Assign to them to change the event; fields.remove("Name") removes a field. Functions: drop() drops the event, regex_match(s, re), regex_replace(s, re, with), regex_capture(s, re, group), and Rhai string functions such as s.contains(x). An event the script fails on (an error, or a limit such as too many operations) stays unchanged and is counted as failed.`;

/** Lines of a script, ignoring leading and trailing blank lines. */
export function scriptLines(script: string | undefined): number {
  const text = (script ?? '').trim();
  return text ? text.split(/\r?\n/).length : 0;
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
    case 'calc':
      return { step: kind, field: '', expr: '' };
    case 'script':
      return { step: kind, script: SCRIPT_TEMPLATE };
    case 'plugin':
      return { step: kind, id: pluginId };
  }
}

const OP_TEXT: Record<ConditionOp, string> = {
  eq: $localize`:Condition operator in a step summary|@@profile.pipeline.op.eq:equals`,
  ne: $localize`:Condition operator in a step summary|@@profile.pipeline.op.ne:is not`,
  contains: $localize`:Condition operator in a step summary|@@profile.pipeline.op.contains:contains`,
  notContains: $localize`:Condition operator in a step summary|@@profile.pipeline.op.notContains:does not contain`,
  matches: $localize`:Condition operator in a step summary|@@profile.pipeline.op.matches:matches`,
  exists: $localize`:Condition operator in a step summary|@@profile.pipeline.op.exists:exists`,
  missing: $localize`:Condition operator in a step summary|@@profile.pipeline.op.missing:is missing`,
  gt: '>',
  lt: '<',
  in: $localize`:Condition operator in a step summary|@@profile.pipeline.op.in:is one of`,
};

const NO_FIELD = $localize`:Placeholder for an unset field name|@@profile.pipeline.noField:(field)`;
const NO_FROM = $localize`:Placeholder for an unset field name|@@profile.pipeline.noFrom:(from)`;
const NO_TO = $localize`:Placeholder for an unset field name|@@profile.pipeline.noTo:(to)`;
const NO_EXPR = $localize`:Placeholder for an unset expression|@@profile.pipeline.noExpression:(expression)`;
const NO_CHOICE = $localize`:Placeholder for an unset choice|@@profile.pipeline.chooseOne:(choose one)`;

/** "SourceContext contains HealthChecks" */
export function describeCondition(c: Condition): string {
  const op = OP_TEXT[c.op] ?? c.op;
  return c.op === 'exists' || c.op === 'missing' ? `${c.field || NO_FIELD} ${op}` : `${c.field || NO_FIELD} ${op} ${c.value ?? ''}`.trimEnd();
}

/** " where A contains x and B equals y", or "" without conditions. */
export function describeWhen(when: Condition[] | undefined): string {
  if (!when?.length) return '';
  const conditions = when.map(describeCondition).join($localize`:Joins two step conditions|@@profile.pipeline.and: and `);
  return ' ' + $localize`:@@profile.pipeline.where:where ${conditions}:conditions:`;
}

const list = (xs: string[] | undefined) => (xs ?? []).filter(Boolean).join(', ') || $localize`:@@profile.pipeline.none:(none)`;

/** Plain-English description of a step, e.g. "Drop events where SourceContext contains HealthChecks". */
export function summarizeStep(s: PipelineStep): string {
  const where = describeWhen(s.when);
  const field = s.field || '@message';
  switch (s.step) {
    case 'drop':
      return (where ? $localize`:@@profile.pipeline.summary.dropEvents:Drop events` : $localize`:@@profile.pipeline.summary.dropEvery:Drop every event`) + where;
    case 'keep':
      return (where ? $localize`:@@profile.pipeline.summary.keepEvents:Keep only events` : $localize`:@@profile.pipeline.summary.keepEvery:Keep only every event`) + where;
    case 'dedupe': {
      const fields = list(s.fields);
      const window = s.window || '5s';
      return $localize`:@@profile.pipeline.summary.dedupe:Collapse repeated events with the same ${fields}:fields: within ${window}:window:` + where;
    }
    case 'mask': {
      const regex = s.regex ?? '';
      const replace = s.replace ?? '';
      return $localize`:@@profile.pipeline.summary.mask:Mask text matching /${regex}:regex:/ in ${field}:field: with "${replace}:replace:"` + where;
    }
    case 'remove_fields':
      return $localize`:@@profile.pipeline.summary.removeFields:Remove fields ${list(s.fields)}:fields:` + where;
    case 'truncate': {
      const max = s.max ?? '?';
      return $localize`:@@profile.pipeline.summary.truncate:Truncate ${field}:field: to ${max}:max: characters` + where;
    }
    case 'rename': {
      const from = s.from || NO_FROM;
      const to = s.to || NO_TO;
      return $localize`:@@profile.pipeline.summary.rename:Rename field ${from}:from: to ${to}:to:` + where;
    }
    case 'set': {
      const target = s.field || NO_FIELD;
      const value = s.value ?? '';
      return $localize`:@@profile.pipeline.summary.set:Set ${target}:field: to "${value}:value:"` + where;
    }
    case 'set_level': {
      const level = s.level ?? '?';
      return $localize`:@@profile.pipeline.summary.setLevel:Set level to ${level}:level:` + where;
    }
    case 'lookup': {
      const target = s.field || NO_FIELD;
      const source = s.file
        ? $localize`:@@profile.pipeline.summary.lookupFile:file ${s.file}:file:`
        : $localize`:@@profile.pipeline.summary.lookupTable:a table of ${Object.keys(s.table ?? {}).length}:count: keys`;
      return $localize`:@@profile.pipeline.summary.lookup:Look up ${target}:field: in ${source}:source:` + where;
    }
    case 'parse_json':
      return (
        (s.prefix
          ? $localize`:@@profile.pipeline.summary.parseJsonPrefix:Parse JSON in ${field}:field: into fields prefixed "${s.prefix}:prefix:"`
          : $localize`:@@profile.pipeline.summary.parseJson:Parse JSON in ${field}:field: into fields`) + where
      );
    case 'parse_kv':
      return (
        (s.prefix
          ? $localize`:@@profile.pipeline.summary.parseKvPrefix:Parse key=value pairs in ${field}:field: into fields prefixed "${s.prefix}:prefix:"`
          : $localize`:@@profile.pipeline.summary.parseKv:Parse key=value pairs in ${field}:field: into fields`) + where
      );
    case 'shift_time': {
      const by = s.by || '?';
      return $localize`:@@profile.pipeline.summary.shiftTime:Shift timestamps by ${by}:by:` + where;
    }
    case 'calc': {
      const target = s.field || NO_FIELD;
      const expr = s.expr || NO_EXPR;
      return $localize`:@@profile.pipeline.summary.calc:Set ${target}:field: = ${expr}:expression:` + where;
    }
    case 'script': {
      const n = scriptLines(s.script);
      return (
        plural(n, {
          one: $localize`:@@profile.pipeline.summary.script.one:Run script (${n}:count: line)`,
          other: $localize`:@@profile.pipeline.summary.script.other:Run script (${n}:count: lines)`,
        }) + where
      );
    }
    case 'plugin': {
      const id = s.id || NO_CHOICE;
      return $localize`:@@profile.pipeline.summary.plugin:Run transform plugin "${id}:id:"` + where;
    }
    default:
      return $localize`:@@profile.pipeline.summary.unknown:Unknown step "${String(s.step)}:step:"`;
  }
}

/** Returns a copy with the element moved; out-of-range targets are clamped. */
export function moveItem<T>(items: readonly T[], from: number, to: number): T[] {
  const next = [...items];
  if (from < 0 || from >= next.length) return next;
  const moved = next.splice(from, 1);
  next.splice(Math.max(0, Math.min(next.length, to)), 0, ...moved);
  return next;
}

/** Copy of a step inserted right after the original. */
export function duplicateStep(steps: readonly PipelineStep[], index: number): PipelineStep[] {
  const step = steps[index];
  if (!step) return [...steps];
  return [...steps.slice(0, index + 1), structuredClone(step), ...steps.slice(index + 1)];
}

/** Plugins that offer pipeline transforms: `contributes` has an entry starting with "transform". */
export function transformPlugins(plugins: readonly PluginInfo[]): PluginInfo[] {
  return plugins.filter((p) => p.contributes.some((c) => c === 'transform' || c.startsWith('transform:')));
}

export interface StepStatus {
  state: 'ok' | 'failed' | 'stale' | 'disabled';
  text: string;
}

/** "31,204 in → 18,950 out (−12,254)" */
export function formatStats(input: number, output: number): string {
  const delta = output - input;
  const change = delta === 0 ? '±0' : `${delta < 0 ? '−' : '+'}${formatCount(Math.abs(delta))}`;
  return $localize`:@@profile.pipeline.stats:${formatCount(input)}:in: in → ${formatCount(output)}:out: out (${change}:change:)`;
}

/**
 * What to show on a step's badge. Stats come from the last import and are only trusted while the
 * edited profile equals the saved one (`dirty` false) and the step has a matching entry.
 */
export function stepStatus(step: PipelineStep, index: number, stats: readonly PipelineStat[], dirty: boolean): StepStatus {
  if (step.disabled) return { state: 'disabled', text: $localize`:Pipeline step status|@@profile.pipeline.status.disabled:disabled` };
  const stat = dirty ? undefined : stats.find((s) => s.index === index && s.step === step.step);
  if (!stat) return { state: 'stale', text: $localize`:Pipeline step status|@@profile.pipeline.status.notRun:not run yet` };
  const text = formatStats(stat.input, stat.output);
  const errors = stat.errors ?? 0;
  return errors > 0 ? { state: 'failed', text: $localize`:@@profile.pipeline.status.failed:${text}:stats:, ${formatCount(errors)}:count: failed` } : { state: 'ok', text };
}

/**
 * The engine's message about step `index` (0-based) in a profile validation error such as
 * "pipeline step 3 (script): `script` syntax error at line 2, column 5: …", or null when the error is about something else.
 */
export function stepProblem(error: string | null | undefined, index: number): string | null {
  const m = /pipeline step (\d+) \([^)]*\): ([\s\S]*)$/.exec(error ?? '');
  return m && Number(m[1]) === index + 1 ? (m[2] ?? null) : null;
}
