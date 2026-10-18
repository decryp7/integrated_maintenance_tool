import { LEVELS, Level, PipelineStat } from '../api.types';
import { valueToString } from '../format';
import { PipelineStep, toJsRegex } from '../profile.model';
import { MockRecord } from './mock-data';
import { compileConditions } from './mock-engine';

export interface PipelineResult {
  records: MockRecord[];
  /** Dropped events per source id. */
  dropped: Map<number, number>;
  /** Events in/out per executed step, in profile order. Disabled steps are not listed. */
  stats: PipelineStat[];
}

function read(r: MockRecord, field: string): string | null {
  switch (field) {
    case '@message':
      return r.message;
    case '@exception':
      return r.exception;
    case '@template':
      return r.template;
    case '@level':
      return r.level;
    default: {
      const v = r.fields[field];
      return v === undefined ? null : valueToString(v);
    }
  }
}

function write(r: MockRecord, field: string, value: string): void {
  if (field === '@message') r.message = value;
  else if (field === '@exception') r.exception = value;
  else if (field === '@template') r.template = value;
  else if (!field.startsWith('@')) r.fields[field] = value;
}

const UNITS: Record<string, number> = { ms: 1, s: 1000, m: 60_000, h: 3_600_000, d: 86_400_000 };

/** "+2.4s", "-150ms", "1m" to signed milliseconds; null when malformed. */
export function parseShift(by: string): number | null {
  const m = /^([+-]?)(\d+(?:\.\d+)?)(ms|s|m|h|d)$/.exec(by.trim());
  if (!m) return null;
  const ms = Number(m[2]) * (UNITS[m[3] ?? ''] ?? 0);
  return m[1] === '-' ? -ms : ms;
}

/** Fields of a JSON object in text (primitives as they are, nested values as JSON), or null. */
export function jsonFields(text: string): Record<string, unknown> | null {
  try {
    const v: unknown = JSON.parse(text);
    if (!v || typeof v !== 'object' || Array.isArray(v)) return null;
    return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, x !== null && typeof x === 'object' ? JSON.stringify(x) : x]));
  } catch {
    return null;
  }
}

/** key=value and key="quoted value" pairs in text. */
export function kvFields(text: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const m of text.matchAll(/([\w.-]+)=(?:"([^"]*)"|(\S+))/g)) {
    const key = m[1];
    if (key !== undefined) out[key] = m[2] ?? m[3] ?? '';
  }
  return out;
}

/**
 * Mock of the engine's ordered `[[pipeline]]`: drop, keep, mask, set, set_level, remove_fields,
 * rename, truncate, parse_json, parse_kv, lookup (inline table), shift_time. Steps with
 * `disabled = true` are skipped. `dedupe`, `calc` (expressions are evaluated only by the engine), `script`
 * (scripts are compiled and run only by the engine) and `plugin` need the real engine and pass events through.
 * Input records are not mutated.
 */
export function applyPipeline(input: MockRecord[], steps: PipelineStep[], sourceName: (id: number) => string): PipelineResult {
  const dropped = new Map<number, number>();
  const stats: PipelineStat[] = [];
  let records = input.map((r) => ({ ...r, fields: { ...r.fields }, markers: [...r.markers] }));
  const lookup = (r: MockRecord) => (f: string) => (f === '@source' ? sourceName(r.source) : read(r, f));
  steps.forEach((step, index) => {
    if (step.disabled) return;
    const before = records.length;
    const when = compileConditions(step.when ?? []);
    const matches = (r: MockRecord) => when(lookup(r));
    const remove = (pred: (r: MockRecord) => boolean) => {
      records = records.filter((r) => {
        if (!pred(r)) return true;
        dropped.set(r.source, (dropped.get(r.source) ?? 0) + 1);
        return false;
      });
    };
    const each = (fn: (r: MockRecord) => void) => records.filter(matches).forEach(fn);
    const field = step.field ?? '@message';
    const prefix = step.prefix ?? '';
    switch (step.step) {
      case 'drop':
        remove(matches);
        break;
      case 'keep':
        remove((r) => !matches(r));
        break;
      case 'mask': {
        let re: RegExp | null = null;
        try {
          re = toJsRegex(step.regex ?? '', 'g');
        } catch {
          /* an invalid regex masks nothing */
        }
        if (re) {
          const rx = re;
          each((r) => {
            const text = read(r, field);
            if (text !== null) write(r, field, text.replace(rx, step.replace ?? ''));
          });
        }
        break;
      }
      case 'set_level':
        if (LEVELS.includes(step.level as Level)) each((r) => (r.level = step.level as Level));
        break;
      case 'remove_fields':
        each((r) => (step.fields ?? []).forEach((f) => delete r.fields[f]));
        break;
      case 'rename':
        each((r) => {
          if (!step.from || !step.to || !(step.from in r.fields)) return;
          r.fields[step.to] = r.fields[step.from];
          delete r.fields[step.from];
        });
        break;
      case 'set':
        if (step.field) each((r) => write(r, step.field!, step.value ?? ''));
        break;
      case 'truncate':
        each((r) => {
          const text = read(r, field);
          if (text !== null && step.max !== undefined && text.length > step.max) write(r, field, text.slice(0, step.max));
        });
        break;
      case 'parse_json':
        each((r) => {
          const parsed = jsonFields(read(r, field) ?? '');
          if (parsed) for (const [k, v] of Object.entries(parsed)) r.fields[prefix + k] = v;
        });
        break;
      case 'parse_kv':
        each((r) => {
          for (const [k, v] of Object.entries(kvFields(read(r, field) ?? ''))) r.fields[prefix + k] = v;
        });
        break;
      case 'lookup':
        each((r) => {
          const key = read(r, field);
          const row = key === null ? undefined : step.table?.[key];
          if (row) Object.assign(r.fields, row);
        });
        break;
      case 'shift_time': {
        const by = parseShift(step.by ?? '');
        if (by !== null) each((r) => (r.ts = r.ts === null ? null : r.ts + by));
        break;
      }
      default:
        break;
    }
    stats.push({ index, step: step.step, input: before, output: records.length });
  });
  return { records, dropped, stats };
}
