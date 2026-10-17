import { RecordView } from '../api.types';
import { checkMockQl, fieldValue, lex, MockQlError, mockCompletions, mockQlFilter, runMockQl } from './mock-ql';

const rec = (id: number, level: RecordView['level'], message: string, fields: Record<string, unknown> = {}): RecordView => ({
  id, ts: id * 1000, level, message, template: id === 1 ? 'T {x}' : null, exception: id === 2 ? 'Boom: x' : null, fields, source: 0, sourceName: 'a.log', line: id, group: 'g', markers: [],
});
const data: RecordView[] = [
  rec(1, 'Information', 'started ok', { Ms: 10, Host: 'a' }),
  rec(2, 'Error', 'timeout talking', { Ms: 30, Host: 'b' }),
  rec(3, 'Warning', 'slow call', { Ms: 20, Host: 'a' }),
  rec(4, 'Fatal', 'crash', { Host: 'b' }),
];
const ids = (q: string) => {
  const r = runMockQl(q, data);
  if (r.kind !== 'events') throw new Error('table');
  return r.ids;
};

describe('mock ql', () => {
  it('lexes strings, operators and words with offsets', () => {
    expect(lex(`a >= "b c" | x,(y)`).map((t) => [t.kind, t.text, t.offset])).toEqual([
      ['word', 'a', 0], ['op', '>=', 2], ['str', 'b c', 5], ['pipe', '|', 11], ['word', 'x', 13], ['comma', ',', 14], ['lp', '(', 15], ['word', 'y', 16], ['rp', ')', 17],
    ]);
    expect(() => lex('"open')).toThrow(MockQlError);
    expect(() => lex('a ! b')).toThrow(/unexpected character/);
  });

  it('filters by level, field comparisons, text, not, or, parentheses', () => {
    expect(ids('')).toEqual([1, 2, 3, 4]);
    expect(ids('level >= warning')).toEqual([2, 3, 4]);
    expect(ids('level < warning')).toEqual([1]);
    expect(ids('level > warning')).toEqual([2, 4]);
    expect(ids('level <= warning')).toEqual([1, 3]);
    expect(ids('@level = error')).toEqual([2]);
    expect(ids('Ms > 15')).toEqual([2, 3]);
    expect(ids('Ms >= 20 and Host = a')).toEqual([3]);
    expect(ids('Ms < 20')).toEqual([1]);
    expect(ids('Ms <= 20')).toEqual([1, 3]);
    expect(ids('Ms = 10')).toEqual([1]);
    expect(ids('Ms != 10')).toEqual([2, 3, 4]);
    expect(ids('Host != a')).toEqual([2, 4]);
    expect(ids('Host ~ ^a$')).toEqual([1, 3]);
    expect(ids('Host !~ ^a$')).toEqual([2, 4]);
    expect(ids('message contains CRASH')).toEqual([4]);
    expect(ids('timeout')).toEqual([2]);
    expect(ids('"slow call"')).toEqual([3]);
    expect(ids('not Host = a')).toEqual([2, 4]);
    expect(ids('not not Host = a')).toEqual([1, 3]);
    expect(ids('Host = a or level = fatal')).toEqual([1, 3, 4]);
    expect(ids('(Host = a or Host = b) and level >= error')).toEqual([2, 4]);
    expect(ids('Host = a and and Ms > 0')).toEqual([1, 3]);
    expect(ids('Host = a | where Ms > 10')).toEqual([3]);
    expect(ids('Missing = x')).toEqual([]);
    expect(ids('@template = "T {x}"')).toEqual([1]);
    expect(ids('Host ~ "["')).toEqual([]);
    expect(ids('exception = Boom')).toEqual([]);
    expect(fieldValue(data[1], '@exception')).toBe('Boom: x');
    expect(fieldValue(data[0], '@source')).toBe('a.log');
    expect(fieldValue(data[0], 'message')).toBe('started ok');
  });

  it('sorts, limits and selects fields', () => {
    expect(ids('| sort Ms desc')).toEqual([2, 3, 1, 4]);
    expect(ids('| sort Ms | head 2')).toEqual([1, 3]);
    expect(ids('| limit 1')).toEqual([1]);
    const r = runMockQl('| fields Host, Ms', data);
    expect(r).toMatchObject({ kind: 'events', fields: ['Host', 'Ms'] });
  });

  it('aggregates with stats, by, as, sort and head on tables', () => {
    expect(runMockQl('| stats count()', data)).toEqual({ kind: 'table', columns: ['count()'], rows: [[4]] });
    expect(runMockQl('| stats count() by Host | sort count() desc', data)).toMatchObject({ columns: ['Host', 'count()'], rows: [['a', 2], ['b', 2]] });
    const t = runMockQl('| stats avg(Ms) as mean, min(Ms), max(Ms), sum(Ms), p50(Ms), p95(Ms), p99(Ms), count_distinct(Host) by level | sort level desc | head 3', data);
    expect(t).toMatchObject({ kind: 'table', columns: ['level', 'mean', 'min(Ms)', 'max(Ms)', 'sum(Ms)', 'p50(Ms)', 'p95(Ms)', 'p99(Ms)', 'count_distinct(Host)'] });
    if (t.kind === 'table') {
      expect(t.rows).toHaveLength(3);
      expect(t.rows.find((r) => r[0] === 'Warning')).toEqual(['Warning', 20, 20, 20, 20, 20, 20, 20, 1]);
      expect(t.rows.find((r) => r[0] === 'Fatal')?.slice(1, 5)).toEqual([null, null, null, 0]);
    }
    expect(runMockQl('Host = zzz | stats count()', data)).toMatchObject({ rows: [[0]] });
    expect(runMockQl('Host = zzz | stats count() by Host', data)).toMatchObject({ rows: [] });
  });

  it('reports errors with a position', () => {
    const err = (q: string) => {
      try {
        runMockQl(q, data);
      } catch (e) {
        return e as MockQlError;
      }
      throw new Error('expected an error: ' + q);
    };
    expect(err('level >= warning | frob')).toMatchObject({ message: "unknown stage 'frob'", offset: 19, len: 4 });
    expect(err('a | ')).toMatchObject({ message: 'expected a stage after |', offset: 4, len: 0 });
    expect(err('level =')).toMatchObject({ message: expect.stringContaining('expected a value'), offset: 7 });
    expect(err('(a')).toMatchObject({ message: 'missing )' });
    expect(err('not')).toMatchObject({ message: expect.stringContaining('after not') });
    expect(err(') x')).toMatchObject({ message: "unexpected ')'" });
    expect(err('| where')).toMatchObject({ message: 'expected a filter expression' });
    expect(err('| stats')).toMatchObject({ message: 'stats needs at least one aggregate' });
    expect(err('| stats frob()')).toMatchObject({ message: "unknown function 'frob'" });
    expect(err('| stats count')).toMatchObject({ message: 'expected an aggregate like count()' });
    expect(err('| stats count(')).toMatchObject({ message: 'missing )' });
    expect(err('| stats count() as')).toMatchObject({ message: 'expected a name after as' });
    expect(err('| stats count() by')).toMatchObject({ message: 'expected a field after by' });
    expect(err('| head')).toMatchObject({ message: 'head needs a number' });
    expect(err('| head x')).toMatchObject({ message: 'head needs a number' });
    expect(err('| sort')).toMatchObject({ message: 'sort needs a field' });
    expect(err('| fields')).toMatchObject({ message: 'fields needs at least one field' });
    expect(err('| stats count() | where x')).toMatchObject({ message: "'where' cannot follow stats" });
    expect(err('| stats count() | sort zzz')).toMatchObject({ message: "unknown column 'zzz'" });
    expect(err('"x')).toMatchObject({ message: 'unterminated string', offset: 0 });
    expect(err('"x').toQlError()).toEqual({ message: 'unterminated string', offset: 0, len: 2 });
  });

  it('checks without running', () => {
    expect(checkMockQl('level >= warning')).toBeNull();
    expect(checkMockQl('| nope')).toMatchObject({ message: "unknown stage 'nope'" });
    expect(() => checkMockQl(undefined as never)).toThrow();
  });

  it('completes stages, aggregates, operators, values and fields', () => {
    const labels = (t: string, c = t.length) => mockCompletions(t, c, ['Host', 'Ms']).map((x) => x.label);
    expect(labels('level >= warning | ')).toEqual(['where', 'stats', 'sort', 'head', 'limit', 'fields']);
    expect(labels('x | st')).toEqual(['stats']);
    expect(labels('x | stats ')).toEqual(expect.arrayContaining(['count', 'p95', 'by', 'as']));
    expect(mockCompletions('x | stats ', 10, [])[0]).toMatchObject({ insert: 'count()', kind: 'function' });
    expect(mockCompletions('x | stats av', 12, [])[0]).toMatchObject({ insert: 'avg(' });
    expect(labels('level >= ')).toEqual(['information', 'warning', 'error', 'fatal']);
    expect(labels('')).toEqual(expect.arrayContaining(['@level', 'Host']));
    expect(labels('Ho')).toEqual(['Host']);
    expect(labels('level ')).toEqual(expect.arrayContaining(['=', 'contains', 'and']));
    expect(labels('a and Ho')).toEqual(['Host']);
    expect(labels('a and ')).toContain('Ms');
    expect(labels('x | stats count() by ')).toEqual(expect.arrayContaining(['Host']));
  });

  it('filters like the engine applies Query.ql: leading expression and where stages only', () => {
    expect(mockQlFilter('')).toBeNull();
    expect(mockQlFilter('   ')).toBeNull();
    const ids = (text: string) => data.filter(mockQlFilter(text)!).map((r) => r.id);
    expect(ids('level >= warning')).toEqual([2, 3, 4]);
    expect(ids('level >= warning | where Host = a')).toEqual([3]);
    // Stages after the first non-where one do not filter (they shape query results only).
    expect(ids('level >= warning | head 1 | where Host = a')).toEqual([2, 3, 4]);
    expect(ids('| stats count() by Host')).toEqual([1, 2, 3, 4]);
    expect(() => mockQlFilter('| frob')).toThrow(MockQlError);
  });

  it('reads backtick field names and escaped quotes like the engine', () => {
    const odd = [rec(1, 'Information', 'say "hi"', { 'User Name': 'ann' }), rec(2, 'Information', 'x', { 'User Name': 'bob' })];
    expect(odd.filter(mockQlFilter('`User Name` = "ann"')!).map((r) => r.id)).toEqual([1]);
    expect(odd.filter(mockQlFilter('@message = "say \\"hi\\""')!).map((r) => r.id)).toEqual([1]);
    expect(odd.filter(mockQlFilter('@message ~ "\\w+"')!).map((r) => r.id)).toEqual([1, 2]);
    expect(() => lex('`open')).toThrow('unterminated field name');
  });
});
