import { CollectPlan, LogSpec, Manifest } from '../../core/tools.types';
import { localizeDecimal } from '../../i18n/locale';

/** Mirrors the engine's defaults; used by the mock and as the base when a pasted plan omits keys. */
export const DEFAULT_PLAN: CollectPlan = {
  name: 'imt-diagnostics',
  logs: [],
  eventLogs: ['System', 'Application'],
  eventLogDays: 7,
  msinfo: false,
  registry: [],
  counters: false,
  counterPaths: ['\\Processor(_Total)\\% Processor Time', '\\Memory\\Available MBytes', '\\PhysicalDisk(_Total)\\% Disk Time'],
  environment: true,
  envAllowlist: ['PATH', 'OS', 'PROCESSOR_ARCHITECTURE', 'NUMBER_OF_PROCESSORS', 'COMPUTERNAME', 'USERDOMAIN', 'LANG', 'TZ', 'SHELL'],
  macosUnifiedLog: false,
  linuxJournal: false,
  recentLogHours: 1,
  redact: { builtIn: true, ipAddresses: true, patterns: [] },
  maxBundleMb: 500,
  commandTimeoutSecs: 120,
};

export const DEFAULT_INCLUDE = ['*.log', '*.txt', '*.json', '*.clef', '*.log.gz'];

export function newLogSpec(): LogSpec {
  return { path: '', include: [...DEFAULT_INCLUDE], maxAgeDays: null, maxTotalMb: null };
}

export type PlanParse = { ok: true; plan: CollectPlan } | { ok: false; error: string };

/**
 * Parses plan JSON for the form. Missing keys take the defaults (the engine does the same);
 * unknown keys are kept so the engine can reject them with its own message.
 */
export function parsePlan(json: string): PlanParse {
  let raw: unknown;
  try {
    raw = JSON.parse(json);
  } catch (e) {
    return { ok: false, error: $localize`:@@tools.planModel.notJson:Not valid JSON: ${e instanceof Error ? e.message : String(e)}:detail:` };
  }
  if (raw === null || typeof raw !== 'object' || Array.isArray(raw)) return { ok: false, error: $localize`:@@tools.planModel.notObject:The plan must be a JSON object.` };
  const given = raw as Partial<CollectPlan>;
  const redact = { ...DEFAULT_PLAN.redact, ...(given.redact ?? {}) };
  const logs = (given.logs ?? []).map((l) => ({ ...newLogSpec(), ...l }));
  return { ok: true, plan: { ...structuredClone(DEFAULT_PLAN), ...given, redact, logs } };
}

export function stringifyPlan(plan: CollectPlan): string {
  return JSON.stringify(plan, null, 2);
}

export function splitLines(text: string): string[] {
  return text
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter(Boolean);
}

export function joinLines(lines: readonly string[]): string {
  return lines.join('\n');
}

/** A number input's text as an optional positive integer: empty = null. */
export function optionalNumber(text: string): number | null {
  const t = text.trim();
  if (!t) return null;
  const n = Number(t);
  return Number.isFinite(n) && n >= 0 ? Math.floor(n) : null;
}

export interface ManifestSummary {
  items: number;
  bytes: number;
  skipped: { what: string; reason: string }[];
  redactionTotal: number;
  redactionByRule: { rule: string; count: number }[];
  errors: string[];
}

export function summarize(manifest: Manifest): ManifestSummary {
  const byRule = Object.entries(manifest.redaction ?? {})
    .map(([rule, count]) => ({ rule, count }))
    .sort((a, b) => b.count - a.count || a.rule.localeCompare(b.rule));
  return {
    items: manifest.items.length,
    bytes: manifest.totalBytes,
    skipped: manifest.skipped.map((s) => ({ what: s.what, reason: s.reason })),
    redactionTotal: byRule.reduce((n, r) => n + r.count, 0),
    redactionByRule: byRule,
    errors: [...manifest.errors],
  };
}

const UNITS = ['B', 'KB', 'MB', 'GB'];

export function formatBytes(n: number): string {
  let v = n;
  let i = 0;
  while (v >= 1024 && i < UNITS.length - 1) {
    v /= 1024;
    i++;
  }
  return `${localizeDecimal(i === 0 ? v : v.toFixed(1))} ${UNITS[i]}`;
}
