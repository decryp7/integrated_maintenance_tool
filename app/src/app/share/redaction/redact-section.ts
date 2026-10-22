import { RedactDetector, REDACT_DETECTORS, RedactMode } from '../../core/api.types';
import { ProfileModel } from '../../core/profile.model';

/** One custom rule of `[[redact.rule]]`. */
export interface RedactRuleForm {
  name: string;
  regex: string;
  /** Empty = the section's mode with the rule name as the label. */
  replace: string;
}

/** The profile's `[redact]` section as the settings dialog edits it. */
export interface RedactForm {
  detectors: RedactDetector[];
  mode: RedactMode;
  fields: string[];
  onExport: boolean;
  rules: RedactRuleForm[];
}

/** The engine's defaults (crates/debrief-core/src/default_profile.toml `[redact]`). */
export const DEFAULT_DETECTORS: readonly RedactDetector[] = ['bearer-token', 'jwt', 'api-key', 'connection-string', 'password', 'user-path', 'email', 'credit-card', 'ipv4', 'ipv6'];
export const DEFAULT_FIELDS: readonly string[] = ['@message', '@exception'];

/** One line per detector for the settings dialog (mirrors `Detector::description`). */
export const DETECTOR_DESCRIPTIONS: Record<RedactDetector, string> = {
  'bearer-token': $localize`:@@share.redactSection.detector.bearerToken:Authorization: Bearer tokens`,
  jwt: $localize`:@@share.redactSection.detector.jwt:JSON Web Tokens (eyJ…)`,
  'api-key': $localize`:@@share.redactSection.detector.apiKey:api_key / token / secret = value`,
  'connection-string': $localize`:@@share.redactSection.detector.connectionString:AccountKey, SharedAccessKey and similar connection-string secrets`,
  password: $localize`:@@share.redactSection.detector.password:password / pwd = value`,
  'user-path': $localize`:@@share.redactSection.detector.userPath:user names in Windows and Unix home folders`,
  email: $localize`:@@share.redactSection.detector.email:e-mail addresses`,
  'credit-card': $localize`:@@share.redactSection.detector.creditCard:card numbers (Luhn checked)`,
  ipv4: $localize`:@@share.redactSection.detector.ipv4:IPv4 addresses`,
  ipv6: $localize`:@@share.redactSection.detector.ipv6:IPv6 addresses`,
  guid: $localize`:@@share.redactSection.detector.guid:GUIDs / UUIDs`,
  phone: $localize`:@@share.redactSection.detector.phone:phone numbers (+country code, (area) or 555-123-4567)`,
  hostname: $localize`:@@share.redactSection.detector.hostname:fully qualified lower-case host names (a.b.example)`,
};

const strings = (v: unknown): string[] | null => (Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : null);

/** Reads `[redact]` of a profile model (missing keys take the engine's defaults; unknown values are dropped). */
export function readRedact(model: ProfileModel): RedactForm {
  const r = (model['redact'] && typeof model['redact'] === 'object' ? model['redact'] : {}) as Record<string, unknown>;
  const detectors = strings(r['detectors']);
  const mode = r['mode'];
  const rules = Array.isArray(r['rule']) ? (r['rule'] as Record<string, unknown>[]) : [];
  return {
    detectors: detectors ? REDACT_DETECTORS.filter((d) => detectors.includes(d)) : [...DEFAULT_DETECTORS],
    mode: mode === 'hash' || mode === 'drop' ? mode : 'mask',
    fields: strings(r['fields']) ?? [...DEFAULT_FIELDS],
    onExport: r['on_export'] === true,
    rules: rules
      .filter((x) => !!x && typeof x === 'object')
      .map((x) => ({ name: String(x['name'] ?? ''), regex: String(x['regex'] ?? ''), replace: typeof x['replace'] === 'string' ? x['replace'] : '' })),
  };
}

/** Writes the form into the model's `[redact]` (snake_case keys, as in the profile TOML). */
export function writeRedact(model: ProfileModel, form: RedactForm): void {
  model['redact'] = {
    detectors: REDACT_DETECTORS.filter((d) => form.detectors.includes(d)),
    mode: form.mode,
    fields: form.fields,
    on_export: form.onExport,
    rule: form.rules.map((r) => (r.replace ? { name: r.name, regex: r.regex, replace: r.replace } : { name: r.name, regex: r.regex })),
  };
}

/** Splits "A, B ,C" into field names. */
export function parseFieldList(text: string): string[] {
  return text
    .split(',')
    .map((s) => s.trim())
    .filter((s) => !!s);
}
