import { ActionFile, ActionKind, AgentFieldSchema, AgentIssue, AgentSchema, ScheduleFile, TriggerFile, TriggerKind } from '../core/agent.types';

/** How a schedule is started. */
export type TimingKind = 'cron' | 'every' | 'trigger';

/** A plain object view of tagged records, for generic schema-driven editing. */
type Bag = Record<string, unknown>;

/** Initial values of a variant: each field's default, and '' for required text without one. */
export function defaultsOf(fields: AgentFieldSchema[]): Bag {
  const out: Bag = {};
  for (const f of fields) {
    if (f.default !== null && f.default !== undefined) out[f.key] = structuredClone(f.default);
    else if (f.required) out[f.key] = f.kind === 'list' ? [] : f.kind === 'number' ? 0 : '';
  }
  return out;
}

export function timingOf(s: ScheduleFile): TimingKind {
  if (s.trigger) return 'trigger';
  if (s.every !== undefined) return 'every';
  return 'cron';
}

export function newTrigger(kind: TriggerKind, schema: AgentSchema): TriggerFile {
  const v = schema.triggers.find((t) => t.kind === kind);
  return { ...defaultsOf(v?.fields ?? []), kind } as TriggerFile;
}

export function newAction(kind: ActionKind, schema: AgentSchema): ActionFile {
  const v = schema.actions.find((a) => a.kind === kind);
  return { ...defaultsOf(v?.fields ?? []), kind } as ActionFile;
}

/** The schedule switched to another timing; the other timings' keys are dropped. */
export function withTiming(s: ScheduleFile, kind: TimingKind, schema: AgentSchema): ScheduleFile {
  const { cron: _c, every: _e, trigger: _t, ...rest } = s;
  const fields = schema.timing.find((t) => t.kind === kind)?.fields ?? [];
  if (kind === 'trigger') return { ...rest, trigger: newTrigger((schema.triggers[0]?.kind as TriggerKind) ?? 'process', schema) };
  return { ...rest, ...defaultsOf(fields) } as ScheduleFile;
}

/** A new schedule with a unique name ("schedule-2", ...). */
export function newSchedule(existing: ScheduleFile[], schema: AgentSchema): ScheduleFile {
  const taken = new Set(existing.map((s) => s.name.toLowerCase()));
  let n = existing.length + 1;
  while (taken.has(`schedule-${n}`)) n++;
  const base: ScheduleFile = { name: `schedule-${n}`, enabled: true, actions: [newAction('collect', schema)] };
  return withTiming(base, 'every', schema);
}

/**
 * Sets one key of a record. Empty values (undefined, '', NaN, an empty optional list) remove an
 * optional key, so the engine sees "not set" rather than an empty value; required keys keep them
 * so the engine reports them.
 */
export function setKey<T extends object>(obj: T, key: string, value: unknown, required = false): T {
  const out = { ...(obj as Bag) };
  const empty = value === undefined || value === null || value === '' || (typeof value === 'number' && Number.isNaN(value)) || (Array.isArray(value) && value.length === 0);
  if (empty && !required) delete out[key];
  else out[key] = value;
  return out as T;
}

/** Issue messages by key path. */
export function issueIndex(issues: AgentIssue[]): Map<string, string[]> {
  const map = new Map<string, string[]>();
  for (const i of issues) map.set(i.path, [...(map.get(i.path) ?? []), i.message]);
  return map;
}

/** Lines of a list field ↔ its values. */
export function listToText(v: unknown): string {
  return Array.isArray(v) ? v.join('\n') : '';
}
export function textToList(text: string): string[] {
  return text
    .split('\n')
    .map((l) => l.trim())
    .filter(Boolean);
}

/** Parses a JSON object field; `null` for empty text; an error message for anything else. */
export function parseJsonObject(text: string): { value: Record<string, unknown> | null } | { error: string } {
  if (!text.trim()) return { value: null };
  try {
    const v: unknown = JSON.parse(text);
    if (v && typeof v === 'object' && !Array.isArray(v)) return { value: v as Record<string, unknown> };
    return { error: 'Enter a JSON object, e.g. {"msinfo": true}' };
  } catch (e) {
    return { error: `Not valid JSON: ${(e as Error).message}` };
  }
}

/** "12.0 MB", "512 B". */
export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let v = n / 1024;
  let u = 0;
  while (v >= 1024 && u < units.length - 1) {
    v /= 1024;
    u++;
  }
  return `${v.toFixed(1)} ${units[u]}`;
}
