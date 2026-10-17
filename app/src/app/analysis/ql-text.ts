import { QlError } from '../core/api.types';

/** Presentation helpers for the query editor: highlighting segments, byte/char offsets, completion edits. */

export type SegClass = 'kw' | 'fn' | 'field' | 'str' | 'num' | 'op' | 'punct' | 'ws' | 'text';

export interface Seg {
  text: string;
  cls: SegClass;
  /** Part of the span the engine reported as an error. */
  err: boolean;
}

const KEYWORDS = new Set(['and', 'or', 'not', 'by', 'as', 'asc', 'desc', 'in', 'contains', 'where', 'stats', 'sort', 'head', 'limit', 'fields']);
const FUNCTIONS = new Set(['count', 'count_distinct', 'avg', 'min', 'max', 'sum', 'p50', 'p95', 'p99', 'exists', 'missing']);
const TOKEN = /(\s+)|("(?:[^"\\]|\\.)*"?|'(?:[^'\\]|\\.)*'?)|(`[^`]*`?)|(>=|<=|!=|!~|[=<>~])|([|(),])|(-?\d+(?:\.\d+)?)(?![\w.])|(@?[\w.-]+)|(.)/gy;

function classify(m: RegExpExecArray, text: string): SegClass {
  if (m[1]) return 'ws';
  if (m[2]) return 'str';
  if (m[3]) return 'field';
  if (m[4]) return 'op';
  if (m[5]) return 'punct';
  if (m[6]) return 'num';
  if (m[7]) {
    const lower = m[7].toLowerCase();
    if (text[m.index + m[7].length] === '(' && FUNCTIONS.has(lower)) return 'fn';
    if (KEYWORDS.has(lower)) return 'kw';
    return m[7].startsWith('@') ? 'field' : 'text';
  }
  return 'text';
}

/** UTF-8 byte offset of the UTF-16 index `index` in `text`. */
export function charToByte(text: string, index: number): number {
  return new TextEncoder().encode(text.slice(0, index)).length;
}

/** UTF-16 index of the UTF-8 byte offset `offset` in `text` (clamped to the text). */
export function byteToChar(text: string, offset: number): number {
  const bytes = new TextEncoder().encode(text);
  return new TextDecoder().decode(bytes.slice(0, Math.max(0, Math.min(offset, bytes.length)))).length;
}

/** The error span as UTF-16 indices; a zero-length error stays zero-length. */
export function errorSpan(text: string, error: QlError | null): { start: number; end: number } | null {
  if (!error) return null;
  const start = byteToChar(text, error.offset);
  const end = error.len > 0 ? byteToChar(text, error.offset + error.len) : start;
  return { start, end };
}

/** Splits the text into classed segments, further split at the error span boundaries. */
export function highlight(text: string, error: QlError | null = null): Seg[] {
  const span = errorSpan(text, error);
  const out: Seg[] = [];
  const push = (start: number, end: number, cls: SegClass) => {
    const cuts = span && span.end > span.start ? [start, span.start, span.end, end].filter((c) => c >= start && c <= end) : [start, end];
    const sorted = [...new Set(cuts)].sort((a, b) => a - b);
    for (let i = 0; i + 1 < sorted.length; i++) {
      const a = sorted[i];
      const b = sorted[i + 1];
      out.push({ text: text.slice(a, b), cls, err: !!span && span.end > span.start && a >= span.start && b <= span.end });
    }
  };
  TOKEN.lastIndex = 0;
  let m: RegExpExecArray | null;
  while (TOKEN.lastIndex < text.length && (m = TOKEN.exec(text))) {
    push(m.index, m.index + m[0].length, classify(m, text));
  }
  return out;
}

const WORD = /[\w@.`-]*$/;

/** Replaces the word before `cursor` (UTF-16 index) with `insert`. */
export function applyCompletion(text: string, cursor: number, insert: string): { text: string; cursor: number } {
  const before = text.slice(0, cursor);
  const word = WORD.exec(before)?.[0] ?? '';
  const start = cursor - word.length;
  const next = text.slice(0, start) + insert + text.slice(cursor);
  // `count()` and `fn(` leave the caret inside the parentheses.
  const caret = insert.endsWith('()') ? start + insert.length - 1 : start + insert.length;
  return { text: next, cursor: caret };
}

/** The word being typed before `cursor`. */
export function wordAt(text: string, cursor: number): string {
  return WORD.exec(text.slice(0, cursor))?.[0] ?? '';
}

/** Escapes a value for a query-language string literal (only `\"` and `\\` are escapes). */
export function qlQuote(value: string): string {
  return `"${value.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"`;
}

const BARE_FIELD = /^[^\s()|,=!<>~"'`]+$/;

/** A field reference: bare when the engine reads it as one word, otherwise in backticks. */
export function qlField(name: string): string {
  return BARE_FIELD.test(name) && !KEYWORDS.has(name.toLowerCase()) ? name : `\`${name.replace(/`/g, '')}\``;
}

/** `field = "value"`, quoted for the engine. */
export function qlEquals(field: string, value: string): string {
  return `${qlField(field)} = ${qlQuote(value)}`;
}

/** Index of the first top-level `|` (outside strings and backticks), or -1. */
function firstPipe(text: string): number {
  let quote: string | null = null;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (quote) {
      if (c === '\\' && quote !== '`') i++;
      else if (c === quote) quote = null;
    } else if (c === '"' || c === "'" || c === '`') quote = c;
    else if (c === '|') return i;
  }
  return -1;
}

/** True when `expr` has an `or` outside parentheses, strings and backticks (so ANDing needs parentheses). */
export function hasTopLevelOr(expr: string): boolean {
  let depth = 0;
  for (const seg of highlight(expr)) {
    if (seg.cls === 'punct' && seg.text === '(') depth++;
    else if (seg.cls === 'punct' && seg.text === ')') depth = Math.max(0, depth - 1);
    else if (depth === 0 && seg.cls === 'kw' && seg.text.toLowerCase() === 'or') return true;
  }
  return false;
}

function wrap(expr: string): string {
  return hasTopLevelOr(expr) ? `(${expr})` : expr;
}

/**
 * ANDs `clause` into the filter of `current` (the part before the first `|`), keeping any later
 * stages; either side is parenthesised when it has a top-level `or`. A blank side yields the other,
 * and a clause the filter already equals is not added twice.
 */
export function andQl(current: string, clause: string): string {
  const add = clause.trim();
  const cur = current.trim();
  if (!add) return cur;
  if (!cur) return add;
  const pipe = firstPipe(cur);
  const filter = (pipe < 0 ? cur : cur.slice(0, pipe)).trim();
  const rest = pipe < 0 ? '' : ` ${cur.slice(pipe).trim()}`;
  if (!filter) return `${add}${rest}`;
  if (filter === add) return cur;
  return `${wrap(filter)} and ${wrap(add)}${rest}`;
}
