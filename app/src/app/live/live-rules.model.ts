import { Severity } from '../core/api.types';
import { ProfileModel } from '../core/profile.model';

/**
 * The `[live]` profile section as the rules editor edits it (snake_case keys as in the TOML; see
 * crates/imt-core/src/default_profile.toml). Validation is the engine's (`validate_profile`); this only
 * reads and writes the model.
 */

/** Detector names as written in the profile. */
export const DETECTORS = ['error_spike', 'silence', 'crash', 'slow_outlier', 'rate_change', 'memory_growth'] as const;
export type Detector = (typeof DETECTORS)[number];

export const DETECTOR_LABELS: Record<Detector, string> = {
  error_spike: $localize`:@@live.rules.detector.errorSpike:Error spike`,
  silence: $localize`:@@live.rules.detector.silence:Silence`,
  crash: $localize`:@@live.rules.detector.crash:Crash`,
  slow_outlier: $localize`:@@live.rules.detector.slowOutlier:Slow outlier`,
  rate_change: $localize`:@@live.rules.detector.rateChange:Rate change`,
  memory_growth: $localize`:@@live.rules.detector.memoryGrowth:Memory growth`,
};

export interface LiveRuleDraft {
  name: string;
  /** Query-language condition. */
  when: string;
  threshold: number;
  /** Duration text, e.g. "1m". */
  window: string;
  severity: Severity;
}

export interface LiveSettingsDraft {
  detectors: Detector[];
  silenceAfter: string;
  cooldown: string;
  window: string;
  clock: 'data' | 'wall';
  rules: LiveRuleDraft[];
}

/** The engine's defaults for a profile without `[live]`. */
export const LIVE_DEFAULTS: LiveSettingsDraft = {
  detectors: [...DETECTORS],
  silenceAfter: '60s',
  cooldown: '5m',
  window: '5m',
  clock: 'data',
  rules: [],
};

export function newRule(index: number): LiveRuleDraft {
  return { name: `rule ${index + 1}`, when: 'level = "Error"', threshold: 5, window: '1m', severity: 'warning' };
}

const str = (v: unknown, fallback: string) => (typeof v === 'string' ? v : typeof v === 'number' ? String(v) : fallback);

function severityOf(v: unknown): Severity {
  const s = typeof v === 'string' ? v.toLowerCase() : '';
  if (s === 'critical' || s === 'error') return 'critical';
  if (s === 'info') return 'info';
  return 'warning';
}

function detectorOf(v: unknown): Detector | null {
  const n = typeof v === 'string' ? v.toLowerCase().replace(/-/g, '_') : '';
  const flat = n.replace(/_/g, '');
  return DETECTORS.find((d) => d.replace(/_/g, '') === flat) ?? null;
}

/** Reads `[live]` (absent keys get the engine defaults; unknown detector names are dropped for editing). */
export function readLive(model: ProfileModel): LiveSettingsDraft {
  const live = (model['live'] && typeof model['live'] === 'object' ? model['live'] : {}) as Record<string, unknown>;
  const alerts = Array.isArray(live['alerts']) ? live['alerts'].map(detectorOf).filter((d): d is Detector => d !== null) : LIVE_DEFAULTS.detectors;
  const rules = Array.isArray(live['rule']) ? (live['rule'] as Record<string, unknown>[]) : [];
  return {
    detectors: [...new Set(alerts)],
    silenceAfter: str(live['silence_after'], LIVE_DEFAULTS.silenceAfter),
    cooldown: str(live['cooldown'], LIVE_DEFAULTS.cooldown),
    window: str(live['window'], LIVE_DEFAULTS.window),
    clock: String(live['clock'] ?? '').toLowerCase() === 'wall' ? 'wall' : 'data',
    rules: rules.map((r, i) => ({
      name: str(r?.['name'], `rule ${i + 1}`),
      when: str(r?.['when'], ''),
      threshold: typeof r?.['threshold'] === 'number' ? r['threshold'] : 1,
      window: str(r?.['window'], '1m'),
      severity: severityOf(r?.['severity']),
    })),
  };
}

/** Writes the draft into `[live]`, keeping keys the editor does not know. */
export function writeLive(model: ProfileModel, draft: LiveSettingsDraft): void {
  const previous = (model['live'] && typeof model['live'] === 'object' ? model['live'] : {}) as Record<string, unknown>;
  const live: Record<string, unknown> = {
    ...previous,
    alerts: DETECTORS.filter((d) => draft.detectors.includes(d)),
    silence_after: draft.silenceAfter.trim(),
    cooldown: draft.cooldown.trim(),
    window: draft.window.trim(),
    clock: draft.clock,
  };
  if (draft.rules.length) {
    live['rule'] = draft.rules.map((r) => ({
      name: r.name.trim(),
      when: r.when,
      threshold: Number.isFinite(r.threshold) ? Math.max(0, Math.round(r.threshold)) : 1,
      window: r.window.trim(),
      severity: r.severity,
    }));
  } else {
    delete live['rule'];
  }
  model['live'] = live;
}

/** Where an engine validation error points in the editor. */
export type ErrorTarget = { kind: 'rule'; index: number } | { kind: 'field'; field: 'alerts' | 'silence_after' | 'cooldown' | 'window' | 'clock' } | { kind: 'general' };

/** Maps an engine message ("live.rule \"x\".window: ...", "live.cooldown: ...") to the editor row it is about. */
export function errorTarget(message: string, rules: readonly LiveRuleDraft[]): ErrorTarget {
  const rule = /live\.rule "([^"]*)"/.exec(message);
  if (rule) {
    const name = rule[1] ?? '';
    let index = rules.findIndex((r) => r.name.trim() === name);
    const numbered = /^rule (\d+)$/.exec(name);
    if (index < 0 && numbered) index = Number(numbered[1]) - 1;
    return index >= 0 && index < rules.length ? { kind: 'rule', index } : { kind: 'general' };
  }
  const field = /live\.(alerts|silence_after|cooldown|window|clock)\b/.exec(message);
  if (field) return { kind: 'field', field: field[1] as 'alerts' };
  return { kind: 'general' };
}
