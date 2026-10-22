import { SqlSchema } from '../core/api.types';

/** One completion: what to insert and what it is. */
export interface SqlCompletion {
  text: string;
  kind: 'view' | 'column' | 'function' | 'keyword';
  /** The column's or view's type or description. */
  detail: string;
}

/** The word being typed at `cursor` (letters, digits, `_`), and where it starts. */
export function wordAt(text: string, cursor: number): { word: string; start: number } {
  let start = cursor;
  while (start > 0 && /[A-Za-z0-9_]/.test(text[start - 1] ?? '')) start--;
  return { word: text.slice(start, cursor), start };
}

const KEYWORDS = ['SELECT', 'FROM', 'WHERE', 'GROUP BY', 'ORDER BY', 'LIMIT', 'WITH', 'JOIN', 'HAVING', 'DISTINCT', 'COUNT(*)', 'DESC', 'ASC'];

/**
 * What the editor offers for the word at `cursor`: the engine's views, their columns (a `view.` prefix narrows to that
 * view), its functions and a few keywords, by prefix, at most `max`. Purely a display aid: the engine checks the SQL.
 */
export function completeSql(text: string, cursor: number, schema: SqlSchema | null, max = 12): SqlCompletion[] {
  if (!schema) return [];
  const { word, start } = wordAt(text, cursor);
  const before = text.slice(0, start);
  const qualified = /([A-Za-z_][A-Za-z0-9_]*)\.$/.exec(before);
  const prefix = word.toLowerCase();
  if (!prefix && !qualified) return [];
  const out: SqlCompletion[] = [];
  const push = (c: SqlCompletion) => {
    const t = c.text.toLowerCase();
    // A word already written in full needs no offer.
    if (out.length < max && t.startsWith(prefix) && t !== prefix && !out.some((o) => o.text === c.text && o.kind === c.kind)) out.push(c);
  };
  if (qualified) {
    const view = schema.views.find((v) => v.name === (qualified[1] ?? '').toLowerCase());
    for (const c of view?.columns ?? []) push({ text: c.name, kind: 'column', detail: c.type });
    return out;
  }
  for (const v of schema.views) push({ text: v.name, kind: 'view', detail: v.description });
  for (const v of schema.views) for (const c of v.columns) push({ text: c.name, kind: 'column', detail: `${v.name} · ${c.type}` });
  for (const k of KEYWORDS) push({ text: k, kind: 'keyword', detail: '' });
  for (const f of [...schema.functions, ...schema.tableFunctions]) push({ text: f, kind: 'function', detail: '' });
  return out;
}

/** `text` with the word at `cursor` replaced by `completion`; the cursor after it. */
export function applyCompletion(text: string, cursor: number, completion: string): { text: string; cursor: number } {
  const { start } = wordAt(text, cursor);
  const next = text.slice(0, start) + completion + text.slice(cursor);
  return { text: next, cursor: start + completion.length };
}

/** A UTF-8 byte offset of `text` (the engine's error offsets) as a string index. */
export function byteToIndex(text: string, byteOffset: number): number {
  const encoder = new TextEncoder();
  let bytes = 0;
  let i = 0;
  for (const ch of text) {
    if (bytes >= byteOffset) return i;
    bytes += encoder.encode(ch).length;
    i += ch.length;
  }
  return text.length;
}

/** Rows as tab-separated text (Copy): a header line, then one line per row; tabs and newlines in cells become spaces. */
export function rowsToTsv(columns: readonly string[], rows: readonly (readonly unknown[])[]): string {
  const cell = (v: unknown) => (v === null || v === undefined ? '' : String(v)).replace(/[\t\r\n]+/g, ' ');
  return [columns.map(cell).join('\t'), ...rows.map((r) => r.map(cell).join('\t'))].join('\n');
}
