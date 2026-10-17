import { cellText, downloadText, sortRows, toCsv } from './table-utils';

describe('table utils', () => {
  it('renders cells', () => {
    expect(cellText(null)).toBe('');
    expect(cellText(undefined)).toBe('');
    expect(cellText(3)).toBe('3');
    expect(cellText({ a: 1 })).toBe('{"a":1}');
  });

  it('sorts numbers by value, text naturally, empties last, stably', () => {
    const rows: unknown[][] = [['b', 10], ['a', 9], ['c', null], ['d', 9]];
    expect(sortRows(rows, 1, 'asc').map((r) => r[0])).toEqual(['a', 'd', 'b', 'c']);
    expect(sortRows(rows, 1, 'desc').map((r) => r[0])).toEqual(['b', 'a', 'd', 'c']);
    expect(sortRows(rows, 0, 'desc').map((r) => r[0])).toEqual(['d', 'c', 'b', 'a']);
    expect(sortRows([['x2'], ['x10'], [''], [null]], 0, 'asc').map((r) => r[0])).toEqual(['x2', 'x10', '', null]);
    expect(sortRows([[null], [undefined]], 0, 'asc')).toEqual([[null], [undefined]]);
    expect(rows[0][0]).toBe('b');
  });

  it('builds CSV with quoting and formula neutralisation', () => {
    const csv = toCsv(['a', 'b'], [['x,y', 'say "hi"'], ['=SUM(A1)', -5], ['line\nbreak', null]]);
    expect(csv).toBe('a,b\r\n"x,y","say ""hi"""\r\n\'=SUM(A1),-5\r\n"line\nbreak",\r\n');
  });

  it('downloads text through an object URL', () => {
    vi.useFakeTimers();
    const created: Blob[] = [];
    const url = { createObjectURL: vi.fn((b: Blob) => (created.push(b), 'blob:x')), revokeObjectURL: vi.fn() };
    vi.stubGlobal('URL', url);
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => undefined);
    downloadText('f.csv', 'a,b');
    expect(click).toHaveBeenCalled();
    expect(created[0].type).toContain('text/csv');
    vi.runAllTimers();
    expect(url.revokeObjectURL).toHaveBeenCalledWith('blob:x');
    click.mockRestore();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });
});
