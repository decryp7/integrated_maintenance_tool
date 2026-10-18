import { ProfileValidation } from '../api.types';
import { ProfileModel } from '../profile.model';
import { checkMockQl } from './mock-ql';

/** Detector names `[live] alerts` accepts (mirrors `alerts::kind_of`, which ignores case, `_` and `-`). */
const DETECTORS = ['errorspike', 'silence', 'crash', 'slowoutlier', 'ratechange', 'memorygrowth'];
const SEVERITIES = ['critical', 'error', 'warning', 'warn', '', 'info'];

/** Parses "60s", "5m", "1h" like `alerts::duration_ms`; null when it is not a positive duration. */
export function mockDurationMs(text: string): number | null {
  const m = /^\s*(\d+(?:\.\d+)?)\s*([a-zA-Z]*)\s*$/.exec(text);
  if (!m) return null;
  const factor: Record<string, number> = { '': 1, ms: 1, s: 1e3, sec: 1e3, secs: 1e3, m: 6e4, min: 6e4, mins: 6e4, h: 3.6e6, hr: 3.6e6, hours: 3.6e6, d: 8.64e7, days: 8.64e7 };
  const f = factor[(m[2] ?? '').toLowerCase()];
  if (f === undefined) return null;
  const ms = Number(m[1]) * f;
  return ms >= 1 ? ms : null;
}

interface LiveRuleCfg {
  name?: unknown;
  when?: unknown;
  threshold?: unknown;
  window?: unknown;
  severity?: unknown;
}

/**
 * Validates `[live]` like `LiveCfg::compile` (mock approximation, same messages). Returns null when valid.
 * `lineOf` finds the line of a text in the TOML for the error position.
 */
export function validateLiveSection(model: ProfileModel, lineOf: (needle: string) => number | null): ProfileValidation | null {
  const live = model['live'] as Record<string, unknown> | undefined;
  if (live === undefined) return null;
  const fail = (error: string, needle: string): ProfileValidation => ({ ok: false, error, line: lineOf(needle) ?? lineOf('[live]') });
  if (typeof live !== 'object' || live === null || Array.isArray(live)) return fail('live: expected a table', '[live]');
  const alerts = live['alerts'];
  if (alerts !== undefined) {
    if (!Array.isArray(alerts)) return fail('live.alerts: expected a list of detector names', 'alerts');
    for (const a of alerts) {
      const n = String(a).replace(/[_-]/g, '').toLowerCase();
      if (!DETECTORS.includes(n)) return fail(`live.alerts: unknown detector "${a}" (use error_spike, silence, crash, slow_outlier, rate_change, memory_growth)`, String(a));
    }
  }
  const clock = String(live['clock'] ?? 'data').toLowerCase();
  if (clock !== 'data' && clock !== 'wall' && clock !== '') return fail(`live.clock must be "data" or "wall", not "${live['clock']}"`, 'clock');
  const rules = (live['rule'] ?? []) as LiveRuleCfg[];
  if (!Array.isArray(rules)) return fail('live.rule: expected [[live.rule]] tables', '[[live.rule]]');
  for (const [i, r] of rules.entries()) {
    const name = typeof r.name === 'string' && r.name.trim() ? r.name : `rule ${i + 1}`;
    const when = typeof r.when === 'string' ? r.when : '';
    const ql = when.trim() ? checkMockQl(when) : null;
    if (ql) return fail(`live.rule "${name}": ${ql.message}`, when);
    if (!when.trim()) return fail(`live.rule "${name}": \`when\` is empty`, '[[live.rule]]');
    if (r.threshold !== undefined && (typeof r.threshold !== 'number' || !Number.isInteger(r.threshold) || r.threshold < 0)) {
      return fail(`live.rule "${name}".threshold: expected a whole number`, 'threshold');
    }
    const window = r.window === undefined ? '1m' : String(r.window);
    if (mockDurationMs(window) === null) return fail(`live.rule "${name}".window: "${window}" is not a duration (e.g. "60s", "5m", "1h")`, window);
    const severity = r.severity === undefined ? 'warning' : String(r.severity).toLowerCase();
    if (!SEVERITIES.includes(severity)) return fail(`live.rule "${name}".severity must be critical, warning or info`, String(r.severity));
  }
  for (const key of ['silence_after', 'cooldown', 'window'] as const) {
    const v = live[key];
    if (v !== undefined && mockDurationMs(String(v)) === null) return fail(`live.${key}: "${v}" is not a duration (e.g. "60s", "5m", "1h")`, key);
  }
  return null;
}
