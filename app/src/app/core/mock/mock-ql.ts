import { LEVELS, Level, RecordView } from '../api.types';
import { Completion, QlError, QlResult } from '../api.types';

/**
 * A tiny stand-in for the engine's query language, for the browser mock and UI tests only.
 * It understands `field op value`, bare text, `and`/`or`/`not` and the stages
 * `where`, `stats`, `sort`, `head`/`limit`, `fields`. The real grammar lives in `imt-core::ql`.
 */

interface Tok {
  kind: 'word' | 'str' | 'op' | 'pipe' | 'comma' | 'lp' | 'rp';
  text: string;
  offset: number;
  len: number;
}

const OPS = ['>=', '<=', '!=', '!~', '=', '>', '<', '~'];

export class MockQlError extends Error {
  constructor(
    message: string,
    readonly offset: number,
    readonly len: number,
  ) {
    super(message);
  }
  toQlError(): QlError {
    return { message: this.message, offset: this.offset, len: this.len };
  }
}

export function lex(text: string): Tok[] {
  const out: Tok[] = [];
  let i = 0;
  while (i < text.length) {
    const c = text[i];
    if (/\s/.test(c)) {
      i++;
      continue;
    }
    if (c === '"' || c === "'") {
      // Like the engine: only \", \' and \\ are escapes; other backslashes stay (regexes like "\d+").
      let value = '';
      let j = i + 1;
      for (; j < text.length && text[j] !== c; j++) {
        if (text[j] === '\\' && ['"', "'", '\\'].includes(text[j + 1])) j++;
        value += text[j];
      }
      if (j >= text.length) throw new MockQlError('unterminated string', i, text.length - i);
      out.push({ kind: 'str', text: value, offset: i, len: j + 1 - i });
      i = j + 1;
      continue;
    }
    if (c === '`') {
      const end = text.indexOf('`', i + 1);
      if (end < 0) throw new MockQlError('unterminated field name (missing closing `)', i, text.length - i);
      out.push({ kind: 'word', text: text.slice(i + 1, end), offset: i, len: end + 1 - i });
      i = end + 1;
      continue;
    }
    const op = OPS.find((o) => text.startsWith(o, i));
    if (op) {
      out.push({ kind: 'op', text: op, offset: i, len: op.length });
      i += op.length;
      continue;
    }
    const single: Record<string, Tok['kind']> = { '|': 'pipe', ',': 'comma', '(': 'lp', ')': 'rp' };
    if (single[c]) {
      out.push({ kind: single[c], text: c, offset: i, len: 1 });
      i++;
      continue;
    }
    let j = i;
    while (j < text.length && !/[\s|,()"'`=<>!~]/.test(text[j])) j++;
    if (j === i) throw new MockQlError(`unexpected character '${c}'`, i, 1);
    out.push({ kind: 'word', text: text.slice(i, j), offset: i, len: j - i });
    i = j;
  }
  return out;
}

/** What a filter reads from an event (a `RecordView` or the mock engine's raw record plus its source name). */
export type QlRecord = Pick<RecordView, 'level' | 'message' | 'exception' | 'template' | 'sourceName' | 'fields'>;

type Pred = (r: QlRecord) => boolean;

export function fieldValue(r: QlRecord, field: string): unknown {
  switch (field.toLowerCase()) {
    case 'level':
    case '@level':
      return r.level;
    case 'message':
    case '@message':
      return r.message;
    case 'exception':
    case '@exception':
      return r.exception ?? undefined;
    case 'source':
    case '@source':
      return r.sourceName;
    case '@template':
      return r.template ?? undefined;
    default:
      return r.fields[field];
  }
}

function compare(l: unknown, op: string, rhs: string, field: string): boolean {
  if (l === undefined || l === null) return op === '!=' || op === '!~';
  if (field.toLowerCase().replace('@', '') === 'level' && ['>', '>=', '<', '<='].includes(op)) {
    const a = LEVELS.indexOf(String(l) as Level);
    const b = LEVELS.findIndex((x) => x.toLowerCase() === rhs.toLowerCase());
    return op === '>' ? a > b : op === '>=' ? a >= b : op === '<' ? a < b : a <= b;
  }
  const s = String(l);
  const n = Number(l);
  const m = Number(rhs);
  const numeric = !Number.isNaN(n) && !Number.isNaN(m) && rhs.trim() !== '';
  switch (op) {
    case '=':
      return numeric ? n === m : s.toLowerCase() === rhs.toLowerCase();
    case '!=':
      return numeric ? n !== m : s.toLowerCase() !== rhs.toLowerCase();
    case '>':
      return numeric && n > m;
    case '>=':
      return numeric && n >= m;
    case '<':
      return numeric && n < m;
    case '<=':
      return numeric && n <= m;
    case '~':
    case '!~': {
      let hit = false;
      try {
        hit = new RegExp(rhs, 'i').test(s);
      } catch {
        hit = false;
      }
      return op === '~' ? hit : !hit;
    }
    default:
      return s.toLowerCase().includes(rhs.toLowerCase());
  }
}

function parseExpr(toks: Tok[], from: number, to: number, end: number): Pred {
  if (from >= to) throw new MockQlError('expected a filter expression', end, 0);
  const orParts: Tok[][] = [[]];
  let depth = 0;
  for (let i = from; i < to; i++) {
    const t = toks[i];
    if (t.kind === 'lp') depth++;
    if (t.kind === 'rp') depth--;
    if (depth === 0 && t.kind === 'word' && t.text.toLowerCase() === 'or') orParts.push([]);
    else orParts[orParts.length - 1].push(t);
  }
  const ors = orParts.map((part) => parseAnd(part, end));
  return (r) => ors.some((p) => p(r));
}

function parseAnd(toks: Tok[], end: number): Pred {
  if (!toks.length) throw new MockQlError('expected a filter expression', end, 0);
  const terms: Pred[] = [];
  let i = 0;
  while (i < toks.length) {
    let negate = false;
    while (toks[i]?.kind === 'word' && toks[i].text.toLowerCase() === 'not') {
      negate = !negate;
      i++;
    }
    const t = toks[i];
    if (!t) throw new MockQlError('expected a term after not', end, 0);
    let pred: Pred;
    if (t.kind === 'lp') {
      let d = 0;
      let j = i;
      for (; j < toks.length; j++) {
        if (toks[j].kind === 'lp') d++;
        if (toks[j].kind === 'rp' && --d === 0) break;
      }
      if (j >= toks.length) throw new MockQlError('missing )', t.offset, t.len);
      pred = parseExpr(toks, i + 1, j, toks[j].offset);
      i = j + 1;
    } else if (t.kind === 'str') {
      const needle = t.text.toLowerCase();
      pred = (r) => r.message.toLowerCase().includes(needle) || (r.exception ?? '').toLowerCase().includes(needle);
      i++;
    } else if (t.kind === 'word') {
      const next = toks[i + 1];
      const isContains = next?.kind === 'word' && next.text.toLowerCase() === 'contains';
      if (next?.kind === 'op' || isContains) {
        const value = toks[i + 2];
        if (!value || (value.kind !== 'word' && value.kind !== 'str')) throw new MockQlError(`expected a value after ${next.text}`, next.offset + next.len, 0);
        const field = t.text;
        const op = isContains ? 'contains' : next.text;
        const rhs = value.text;
        pred = (r) => compare(fieldValue(r, field), op, rhs, field);
        i += 3;
      } else if (t.text.toLowerCase() === 'and') {
        i++;
        continue;
      } else {
        const needle = t.text.toLowerCase();
        pred = (r) => r.message.toLowerCase().includes(needle) || (r.exception ?? '').toLowerCase().includes(needle);
        i++;
      }
    } else {
      throw new MockQlError(`unexpected '${t.text}'`, t.offset, t.len);
    }
    const final = pred;
    terms.push(negate ? (r) => !final(r) : final);
  }
  return (r) => terms.every((p) => p(r));
}

interface Agg {
  fn: string;
  field: string | null;
  name: string;
}

/** Numbers by value, text alphabetically, missing values last whatever the direction. */
function compareCells(x: unknown, y: unknown, desc: boolean): number {
  const xe = x === undefined || x === null;
  const ye = y === undefined || y === null;
  if (xe || ye) return xe === ye ? 0 : xe ? 1 : -1;
  const c = typeof x === 'number' && typeof y === 'number' ? x - y : String(x).localeCompare(String(y), undefined, { numeric: true });
  return desc ? -c : c;
}

function percentile(sorted: number[], p: number): number {
  if (!sorted.length) return 0;
  return sorted[Math.min(sorted.length - 1, Math.floor((p / 100) * sorted.length))];
}

function runStats(records: RecordView[], stage: Tok[], end: number): QlResult {
  const byIdx = stage.findIndex((t) => t.kind === 'word' && t.text.toLowerCase() === 'by');
  const aggToks = stage.slice(0, byIdx < 0 ? stage.length : byIdx);
  const by = byIdx < 0 ? [] : stage.slice(byIdx + 1).filter((t) => t.kind !== 'comma').map((t) => t.text);
  if (byIdx >= 0 && !by.length) throw new MockQlError('expected a field after by', end, 0);
  const aggs: Agg[] = [];
  for (let i = 0; i < aggToks.length; ) {
    const t = aggToks[i];
    if (t.kind === 'comma') {
      i++;
      continue;
    }
    if (t.kind !== 'word' || aggToks[i + 1]?.kind !== 'lp') throw new MockQlError('expected an aggregate like count()', t.offset, t.len);
    const fn = t.text.toLowerCase();
    if (!['count', 'count_distinct', 'avg', 'min', 'max', 'sum', 'p50', 'p95', 'p99'].includes(fn)) throw new MockQlError(`unknown function '${t.text}'`, t.offset, t.len);
    let j = i + 2;
    let field: string | null = null;
    if (aggToks[j]?.kind === 'word') field = aggToks[j++].text;
    if (aggToks[j]?.kind !== 'rp') throw new MockQlError('missing )', t.offset, t.len);
    j++;
    let name = field ? `${fn}(${field})` : `${fn}()`;
    if (aggToks[j]?.kind === 'word' && aggToks[j].text.toLowerCase() === 'as') {
      const n = aggToks[j + 1];
      if (!n) throw new MockQlError('expected a name after as', end, 0);
      name = n.text;
      j += 2;
    }
    aggs.push({ fn, field, name });
    i = j;
  }
  if (!aggs.length) throw new MockQlError('stats needs at least one aggregate', end, 0);
  const groups = new Map<string, RecordView[]>();
  for (const r of records) {
    const key = JSON.stringify(by.map((f) => String(fieldValue(r, f) ?? '')));
    (groups.get(key) ?? groups.set(key, []).get(key)!).push(r);
  }
  if (!by.length && !groups.size) groups.set('[]', []);
  const rows = [...groups.entries()].map(([key, rs]) => {
    const cells: unknown[] = JSON.parse(key) as string[];
    for (const a of aggs) {
      const nums = rs.map((r) => Number(fieldValue(r, a.field ?? ''))).filter((n) => !Number.isNaN(n)).sort((x, y) => x - y);
      const v =
        a.fn === 'count' ? rs.length
        : a.fn === 'count_distinct' ? new Set(rs.map((r) => String(fieldValue(r, a.field ?? '')))).size
        : a.fn === 'avg' ? (nums.length ? +(nums.reduce((s, x) => s + x, 0) / nums.length).toFixed(3) : null)
        : a.fn === 'min' ? (nums[0] ?? null)
        : a.fn === 'max' ? (nums[nums.length - 1] ?? null)
        : a.fn === 'sum' ? nums.reduce((s, x) => s + x, 0)
        : percentile(nums, Number(a.fn.slice(1)));
      cells.push(v);
    }
    return cells;
  });
  return { kind: 'table', columns: [...by, ...aggs.map((a) => a.name)], rows };
}

function splitStages(text: string): Tok[][] {
  const stages: Tok[][] = [[]];
  for (const t of lex(text)) {
    if (t.kind === 'pipe') stages.push([]);
    else stages[stages.length - 1].push(t);
  }
  return stages;
}

/**
 * The filter `Query.ql` applies to the grid, timeline, charts and facets, like the engine's
 * `CompiledQuery`: the leading expression and the `where` stages up to the first other stage.
 * Null for blank text; throws `MockQlError` when the whole query does not parse.
 */
export function mockQlFilter(text: string): Pred | null {
  if (!text.trim()) return null;
  runMockQl(text, []);
  const end = text.length;
  const [first, ...rest] = splitStages(text);
  const preds: Pred[] = first.length ? [parseExpr(first, 0, first.length, end)] : [];
  for (const st of rest) {
    if (st[0]?.text.toLowerCase() !== 'where') break;
    const body = st.slice(1);
    preds.push(parseExpr(body, 0, body.length, end));
  }
  return (r) => preds.every((p) => p(r));
}

/** Parses and evaluates `text` over `records`; throws `MockQlError` with a position on bad input. */
export function runMockQl(text: string, records: RecordView[]): QlResult {
  const end = text.length;
  const stages = splitStages(text);
  let current = records;
  let table = null as QlResult | null;
  let fields: string[] | null = null;
  for (const [idx, st] of stages.entries()) {
    if (idx === 0) {
      if (st.length) {
        const pred = parseExpr(st, 0, st.length, end);
        current = current.filter(pred);
      }
      continue;
    }
    const head = st[0];
    if (!head) throw new MockQlError('expected a stage after |', end, 0);
    const name = head.text.toLowerCase();
    const rest = st.slice(1);
    const restEnd = rest.length ? rest[rest.length - 1].offset + rest[rest.length - 1].len : head.offset + head.len;
    if (table && name !== 'head' && name !== 'limit' && name !== 'sort') throw new MockQlError(`'${name}' cannot follow stats`, head.offset, head.len);
    switch (name) {
      case 'where': {
        const pred = parseExpr(rest, 0, rest.length, restEnd);
        current = current.filter(pred);
        break;
      }
      case 'stats':
        table = runStats(current, rest, restEnd);
        break;
      case 'head':
      case 'limit': {
        const n = Number(rest[0]?.text);
        if (!rest.length || Number.isNaN(n)) throw new MockQlError(`${name} needs a number`, restEnd, 0);
        if (table && table.kind === 'table') table = { ...table, rows: table.rows.slice(0, n) };
        else current = current.slice(0, n);
        break;
      }
      case 'sort': {
        if (!rest.length) throw new MockQlError('sort needs a field', restEnd, 0);
        const dirAt = rest.findIndex((t) => t.kind === 'word' && ['asc', 'desc'].includes(t.text.toLowerCase()));
        const keyToks = dirAt < 0 ? rest : rest.slice(0, dirAt);
        const key = keyToks.map((t) => t.text).join('');
        const desc = dirAt >= 0 && rest[dirAt].text.toLowerCase() === 'desc';
        if (table && table.kind === 'table') {
          const col = table.columns.indexOf(key);
          if (col < 0) throw new MockQlError(`unknown column '${key}'`, rest[0].offset, rest[0].len);
          const rows = [...table.rows].sort((a, b) => compareCells(a[col], b[col], desc));
          table = { ...table, rows };
        } else {
          current = [...current].sort((a, b) => compareCells(fieldValue(a, key), fieldValue(b, key), desc));
        }
        break;
      }
      case 'fields':
        if (!rest.length) throw new MockQlError('fields needs at least one field', restEnd, 0);
        fields = rest.filter((t) => t.kind !== 'comma').map((t) => t.text);
        break;
      default:
        throw new MockQlError(`unknown stage '${head.text}'`, head.offset, head.len);
    }
  }
  return table ?? { kind: 'events', ids: current.map((r) => r.id), fields };
}

export function checkMockQl(text: string): QlError | null {
  try {
    runMockQl(text, []);
    return null;
  } catch (e) {
    if (e instanceof MockQlError) return e.toQlError();
    throw e;
  }
}

const STAGES = ['where', 'stats', 'sort', 'head', 'limit', 'fields'];
const AGGS = ['count', 'count_distinct', 'avg', 'min', 'max', 'sum', 'p50', 'p95', 'p99'];
const OPERATORS = ['=', '!=', '>', '>=', '<', '<=', '~', '!~', 'contains', 'in'];
const PSEUDO = ['@level', '@message', '@exception', '@template', '@source', 'level', 'message'];

/** Suggestions for the word ending at `cursor`. */
export function mockCompletions(text: string, cursor: number, fieldNames: string[]): Completion[] {
  const before = text.slice(0, cursor);
  const word = /[\w@.`-]*$/.exec(before)?.[0] ?? '';
  const head = before.slice(0, before.length - word.length);
  const lastPipe = head.lastIndexOf('|');
  const stage = lastPipe >= 0 ? head.slice(lastPipe + 1).trim().split(/\s+/)[0]?.toLowerCase() : null;
  const prevToken = head.trim().split(/\s+/).pop() ?? '';
  const mk = (labels: string[], kind: Completion['kind']): Completion[] => labels.map((label) => ({ label, kind, insert: kind === 'function' && label !== 'count' ? `${label}(` : label === 'count' ? 'count()' : label }));
  let pool: Completion[];
  if (head.trim().endsWith('|') || (lastPipe >= 0 && head.slice(lastPipe + 1).trim() === '')) pool = mk(STAGES, 'keyword');
  else if (stage === 'stats' && !/\bby\b/i.test(head.slice(lastPipe))) pool = [...mk(AGGS, 'function'), ...mk(['by', 'as'], 'keyword')];
  else if (OPERATORS.includes(prevToken)) pool = mk(['information', 'warning', 'error', 'fatal'], 'value');
  else if (head.trim() === '' || ['and', 'or', 'not', 'by', ',', '|'].includes(prevToken.toLowerCase())) pool = mk([...PSEUDO, ...fieldNames], 'field');
  else pool = [...mk(OPERATORS, 'operator'), ...mk(['and', 'or'], 'keyword')];
  if (!word) return pool.slice(0, 20);
  const w = word.toLowerCase();
  return pool.filter((c) => c.label.toLowerCase().startsWith(w) && c.label.toLowerCase() !== w).slice(0, 20);
}
