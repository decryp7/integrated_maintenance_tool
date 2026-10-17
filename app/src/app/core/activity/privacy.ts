import { redactPaths } from '../logger';
import { ActivityParams } from './types';

/**
 * What an activity entry may carry. Never: file contents, log lines, record messages, AI prompts or answers,
 * web content, secrets. Query/search text and paths depend on the detail setting (Diagnostics ▸ Activity
 * log detail). Mirrors app/src-tauri/src/activity/privacy.rs; the fingerprint is the same 32-bit FNV-1a.
 */
export type Detail = 'normal' | 'detailed';

export const MAX_PARAMS = 16;
export const MAX_VALUE = 200;
export const SECRET = '***';

/** 32-bit FNV-1a of the UTF-8 bytes, 8 lowercase hex digits (not cryptographic: it shows sameness only). */
export function fingerprint(text: string): string {
  let h = 0x811c9dc5;
  for (const b of new TextEncoder().encode(text)) {
    h ^= b;
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h.toString(16).padStart(8, '0');
}

const SECRET_KEYS = ['secret', 'token', 'password', 'passwd', 'credential', 'cookie', 'authorization', 'apikey'];

/** Names of secret values (`ApiKey`, `braveKey`, `api_key`, `Token`, ...), not `Keys` or `Keybinding`. Same rule as the engine. */
export function isSecretKey(key: string): boolean {
  const k = key.toLowerCase();
  return k === 'auth' || k.endsWith('key') || k.includes('_key') || SECRET_KEYS.some((s) => k.includes(s));
}

/** Values that look like secrets even under an innocent name. */
const SECRET_VALUE = /(sk-[A-Za-z0-9_-]{8,}|(?:password|pwd|token|secret|api[_-]?key)\s*[=:]\s*\S+|Bearer\s+\S+|-----BEGIN [A-Z ]*PRIVATE KEY-----)/gi;

function cap(text: string): string {
  return [...text].length <= MAX_VALUE ? text : `${[...text].slice(0, MAX_VALUE).join('')}…`;
}

/** A parameter value as it may be logged: secrets masked by name or content, capped; absolute paths redacted in normal mode. */
export function safeParam(key: string, value: unknown, detail: Detail = 'normal'): string {
  if (isSecretKey(key)) return SECRET;
  const text = String(value).replace(SECRET_VALUE, SECRET);
  return cap(detail === 'detailed' ? text : redactPaths(text));
}

/** Converts parameters to safe strings (at most `MAX_PARAMS`, undefined/null dropped). */
export function safeParams(params: ActivityParams = {}, detail: Detail = 'normal'): Record<string, string> {
  const out: Record<string, string> = {};
  let n = 0;
  for (const [k, v] of Object.entries(params)) {
    if (v === undefined || v === null) continue;
    if (n++ >= MAX_PARAMS) break;
    out[k] = safeParam(k, v, detail);
  }
  return out;
}

/**
 * Text the user typed (query, search): `{Prefix}Hash` and `{Prefix}Length` always, `{Prefix}` itself only in
 * detailed mode (secrets still masked).
 */
export function safeText(prefix: string, text: string, detail: Detail): ActivityParams {
  const out: ActivityParams = { [`${prefix}Hash`]: fingerprint(text), [`${prefix}Length`]: [...text].length };
  if (detail === 'detailed') out[prefix] = cap(text.replace(SECRET_VALUE, SECRET));
  return out;
}

/** A path: `<dir#1a2b3c4d>/name.log` in normal mode, the full path in detailed mode. */
export function safePath(path: string, detail: Detail): string {
  if (detail === 'detailed') return cap(path);
  const trimmed = path.replace(/[\\/]+$/, '');
  const i = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'));
  if (i < 0) return cap(trimmed);
  const dir = trimmed.slice(0, i);
  return cap(dir ? `<dir#${fingerprint(dir)}>/${trimmed.slice(i + 1)}` : trimmed);
}
