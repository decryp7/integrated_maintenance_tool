import { parse } from 'smol-toml';
import { RedactDetector, RedactionPreview, RedactMode, RuleHits } from '../../core/api.types';

/**
 * A plausible stand-in for the engine's redaction (crates/imt-core/src/redact) for browser development
 * and UI tests. It mirrors the detectors' intent with simpler patterns; the engine is the reference.
 */

/** The `[redact]` section, normalised (defaults as in default_profile.toml). */
export interface MockRedactCfg {
  detectors: RedactDetector[];
  mode: RedactMode;
  fields: string[];
  onExport: boolean;
  rules: { name: string; regex: string; replace?: string }[];
}

const DEFAULT_DETECTORS: RedactDetector[] = ['bearer-token', 'jwt', 'api-key', 'connection-string', 'password', 'user-path', 'email', 'credit-card', 'ipv4', 'ipv6'];

/** Label in masks (`<email>`) per detector. */
const LABEL: Record<RedactDetector, string> = {
  'bearer-token': 'token',
  jwt: 'token',
  'api-key': 'secret',
  'connection-string': 'secret',
  password: 'password',
  'user-path': 'user',
  email: 'email',
  'credit-card': 'card',
  ipv4: 'ipv4',
  ipv6: 'ipv6',
  guid: 'guid',
  phone: 'phone',
  hostname: 'host',
};

/** Patterns; a `value` group is the part replaced. */
const PATTERN: Record<RedactDetector, RegExp> = {
  'bearer-token': /\bBearer\s+(?<value>[A-Za-z0-9._~+/=-]{8,})/gi,
  jwt: /\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]*/g,
  'api-key': /(?:api[_-]?key|access[_-]?token|secret|token)["']?\s*[=:]\s*["']?(?<value>[A-Za-z0-9._~+/=-]{8,})/gi,
  'connection-string': /(?:AccountKey|SharedAccessKey)\s*=\s*(?<value>[^;\s"']+)/gi,
  password: /(?:password|passwd|pwd)["']?\s*[=:]\s*["']?(?<value>[^\s;,&"']+)/gi,
  'user-path': /(?:\b[A-Z]:\\Users\\|\/(?:home|Users)\/)(?<value>[^\\/\s:]+)/gi,
  email: /\b[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}\b/g,
  'credit-card': /\b\d(?:[ -]?\d){12,18}\b/g,
  ipv4: /\b(?:(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)\.){3}(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)\b/g,
  ipv6: /\b(?:[0-9a-f]{1,4}:){7}[0-9a-f]{1,4}\b/gi,
  guid: /\b[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}\b/g,
  phone: /\+\d{1,3}[ .-]?\d{1,4}(?:[ .-]?\d{2,4}){1,4}\b/g,
  hostname: /\b(?:[a-z0-9-]+\.){2,}(?:com|net|org|io|local|corp|internal)\b/g,
};

function asStrings(v: unknown): string[] | null {
  return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : null;
}

/** Reads `[redact]` of a profile model or TOML text. Throws (like the engine) on an invalid section. */
export function mockRedactCfg(profile: Record<string, unknown> | string): MockRedactCfg {
  const model = typeof profile === 'string' ? (parse(profile) as Record<string, unknown>) : profile;
  const r = (model['redact'] ?? {}) as Record<string, unknown>;
  const detectors = (asStrings(r['detectors']) ?? DEFAULT_DETECTORS) as RedactDetector[];
  for (const d of detectors) if (!(d in PATTERN)) throw new Error(`[redact]: unknown variant \`${d}\``);
  const mode = (r['mode'] ?? 'mask') as RedactMode;
  if (!['mask', 'hash', 'drop'].includes(mode)) throw new Error(`[redact]: unknown variant \`${mode}\``);
  const rules = Array.isArray(r['rule']) ? (r['rule'] as { name?: unknown; regex?: unknown; replace?: unknown }[]) : [];
  return {
    detectors: DEFAULT_DETECTORS.concat(['guid', 'phone', 'hostname']).filter((d) => detectors.includes(d)),
    mode,
    fields: asStrings(r['fields']) ?? ['@message', '@exception'],
    onExport: r['on_export'] === true,
    rules: rules.map((x) => {
      const name = String(x.name ?? '');
      const regex = String(x.regex ?? '');
      if (!name.trim()) throw new Error('[redact]: a redaction rule needs a name');
      try {
        new RegExp(regex.replace(/\(\?P</g, '(?<'));
      } catch {
        throw new Error(`[redact]: redaction rule "${name}": invalid regex`);
      }
      return { name, regex, replace: typeof x.replace === 'string' ? x.replace : undefined };
    }),
  };
}

/** A small stable hash (FNV-1a) standing in for the engine's salted SHA-256. */
function hash(salt: string, value: string): string {
  let h = 0x811c9dc5;
  for (const c of salt + value) h = Math.imul(h ^ c.charCodeAt(0), 0x01000193) >>> 0;
  return h.toString(16).padStart(8, '0').slice(0, 8) + 'ab';
}

/** Redacts text with a config; counts per rule. */
export class MockRedactor {
  constructor(
    readonly cfg: MockRedactCfg,
    private readonly salt = 'preview',
  ) {}

  get empty(): boolean {
    return !this.cfg.detectors.length && !this.cfg.rules.length;
  }

  covers(field: string): boolean {
    const f = this.cfg.fields;
    return f.includes('*') || f.includes(field) || (field === '@template' && f.includes('@message'));
  }

  private token(label: string, value: string): string {
    if (this.cfg.mode === 'drop') return '';
    if (this.cfg.mode === 'hash') return `<${label}:${hash(this.salt, value)}>`;
    return `<${label}>`;
  }

  apply(text: string, counts: Record<string, number> = {}): string {
    let out = text;
    for (const d of this.cfg.detectors) {
      out = out.replace(PATTERN[d], (...args) => {
        const whole = args[0] as string;
        const groups = args[args.length - 1] as Record<string, string> | undefined;
        const value = typeof groups === 'object' && groups?.['value'] !== undefined ? groups['value'] : whole;
        if (d === 'credit-card' && !luhn(whole)) return whole;
        counts[d] = (counts[d] ?? 0) + 1;
        return whole.replace(value, this.token(LABEL[d], value));
      });
    }
    for (const rule of this.cfg.rules) {
      const re = new RegExp(rule.regex.replace(/\(\?P</g, '(?<'), 'g');
      out = out.replace(re, (...args) => {
        const whole = args[0] as string;
        counts[rule.name] = (counts[rule.name] ?? 0) + 1;
        if (rule.replace !== undefined) return rule.replace.replace(/\$(\d)/g, (_, i: string) => String(args[Number(i)] ?? ''));
        const groups = args[args.length - 1] as Record<string, string> | undefined;
        const value = typeof groups === 'object' && groups?.['value'] !== undefined ? groups['value'] : whole;
        return whole.replace(value, this.token(rule.name, value));
      });
    }
    return out;
  }

  /** The text of a field as the redacted view shows it. */
  field(name: string, text: string): string {
    return this.covers(name) ? this.apply(text) : text;
  }

  hits(counts: Record<string, number>): RuleHits[] {
    return [...this.cfg.detectors, ...this.cfg.rules.map((r) => r.name)].map((rule) => ({ rule, count: counts[rule] ?? 0 }));
  }
}

function luhn(s: string): boolean {
  const digits = [...s].filter((c) => c >= '0' && c <= '9').map(Number);
  if (digits.length < 13 || digits.length > 19) return false;
  const sum = digits.reverse().reduce((acc, d, i) => acc + (i % 2 ? (d * 2 > 9 ? d * 2 - 9 : d * 2) : d), 0);
  return sum % 10 === 0;
}

/** `preview_redaction` against a profile (model or TOML). */
export function mockRedactionPreview(profile: Record<string, unknown> | string, sample: string): RedactionPreview {
  try {
    const r = new MockRedactor(mockRedactCfg(profile));
    const counts: Record<string, number> = {};
    const after = r.apply(sample, counts);
    return { before: sample, after, hits: r.hits(counts), error: null };
  } catch (e) {
    return { before: sample, after: sample, hits: [], error: e instanceof Error ? e.message : String(e) };
  }
}
