import { ExtractFieldType } from '../core/profile.model';

/** Escapes text so it matches itself literally in a Rust/JS regular expression. */
export function escapeRegex(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

/** Escapes literal text but generalises every run of digits to `\d+` (so ids and counts still match). */
export function generalizeDigits(text: string): string {
  return text
    .split(/\d+/)
    .map(escapeRegex)
    .join('\\d+');
}

/** Regex that masks the selected text in other events too: the text itself with digit runs generalised. */
export function maskRegexFor(selected: string): string {
  return generalizeDigits(selected);
}

const CONTEXT_BEFORE = 16;
const CONTEXT_AFTER = 12;

/** Literal text around the selection, trimmed to whole words where it was cut. */
export function surroundingContext(message: string, start: number, end: number): { before: string; after: string } {
  let before = message.slice(Math.max(0, start - CONTEXT_BEFORE), start);
  if (start > CONTEXT_BEFORE) before = before.replace(/^\S*\s/, '');
  let after = message.slice(end, end + CONTEXT_AFTER);
  if (end + CONTEXT_AFTER < message.length) after = after.replace(/\s\S*$/, '');
  return { before, after };
}

export interface ExtractProposal {
  /** Regex with a named group, in Rust syntax. */
  regex: string;
  /** Suggested field (and group) name. */
  name: string;
  type: ExtractFieldType;
  /** Unit the number is written in, when it is obvious (MB, ms, %, ...). */
  unit?: string;
  /** Unit to convert to, for bytes/duration. */
  to?: string;
}

const NUMBER = /^-?\d+(?:\.\d+)?$/;

/** A valid field name suggested by the word before the selection ("CPU " -> "CPU", fallback "Value"). */
export function suggestFieldName(before: string): string {
  const word = /([A-Za-z][A-Za-z0-9_]*)\W*$/.exec(before)?.[1];
  return word ?? 'Value';
}

function unitAfter(after: string): string | undefined {
  return /^\s?(%|KB|MB|GB|TB|ms|s|m|h)\b/i.exec(after)?.[1] ?? (after.startsWith('%') ? '%' : undefined);
}

/**
 * Proposes an `[[extract]]` regex for the selected part of a message: the surrounding literal text is
 * escaped (digits generalised) and the selection becomes a named group (`[\d.]+` for numbers).
 */
export function proposeExtraction(message: string, start: number, end: number): ExtractProposal {
  const selected = message.slice(start, end);
  const { before, after } = surroundingContext(message, start, end);
  const name = suggestFieldName(before);
  const numeric = NUMBER.test(selected.trim());
  const group = numeric ? '[\\d.]+' : /\s/.test(selected) ? '.+?' : '\\S+';
  const regex = `${generalizeDigits(before)}(?P<${name}>${group})${generalizeDigits(after)}`;
  if (!numeric) return { regex, name, type: 'string' };
  const unit = unitAfter(after);
  const lower = unit?.toLowerCase();
  if (lower && ['kb', 'mb', 'gb', 'tb'].includes(lower)) return { regex, name, type: 'bytes', unit: unit!.toUpperCase(), to: 'MB' };
  if (lower && ['ms', 's', 'm', 'h'].includes(lower)) return { regex, name, type: 'duration', unit: lower, to: 'ms' };
  return { regex, name, type: 'number', ...(unit === '%' ? { unit } : {}) };
}
