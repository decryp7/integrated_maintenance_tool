import { RecordView } from '../core/api.types';
import { generalizeText, proposeHighlight, qlToConditions } from './highlight';
import { at } from '../../testing/must';

const rec = (p: Partial<RecordView>) => ({ message: 'Slow request /api took 1500 ms', template: null, ...p }) as RecordView;

describe('generalizeText', () => {
  it('escapes literals and generalises numbers, guids and hex', () => {
    expect(generalizeText('took 1500 ms (a+b)')).toBe('took \\d+ ms \\(a\\+b\\)');
    expect(generalizeText('id 3f2504e0-4f89-11d3-9a0c-0305e82c3301 ok')).toBe('id [0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12} ok');
    expect(generalizeText('ptr 0xDEADbeef')).toBe('ptr 0x[0-9a-fA-F]+');
    expect(generalizeText('hash deadbeefcafe1234')).toBe('hash [0-9a-fA-F]{12,}');
    expect(generalizeText('plain')).toBe('plain');
  });
});

describe('proposeHighlight', () => {
  it('uses the selection first', () => {
    const p = proposeHighlight(rec({}), ' took 1500 ms ');
    expect(p).toMatchObject({ basis: 'selection', label: 'took 1500 ms', conditions: [{ field: '@message', op: 'matches', value: 'took \\d+ ms' }] });
  });
  it('then the template, then the message', () => {
    expect(proposeHighlight(rec({ template: 'Slow {Path}' }))).toMatchObject({ basis: 'template', conditions: [{ field: '@template', op: 'eq', value: 'Slow {Path}' }] });
    const m = proposeHighlight(rec({}), '   ');
    expect(m).toMatchObject({ basis: 'message' });
    expect(at(m.conditions, 0).value).toBe('^Slow request /api took \\d+ ms$');
  });
  it('shortens long labels', () => {
    expect(proposeHighlight(rec({ message: 'x'.repeat(80) })).label).toHaveLength(41);
  });
});

describe('qlToConditions', () => {
  it('converts conjunctions of simple terms', () => {
    expect(qlToConditions('level = "Error" and message ~ "time\\d+out" and Foo contains \'a and b\' and x !contains "q" and y != "1"')).toEqual([
      { field: '@level', op: 'eq', value: 'Error' },
      { field: '@message', op: 'matches', value: 'time\\d+out' },
      { field: 'Foo', op: 'contains', value: 'a and b' },
      { field: 'x', op: 'notContains', value: 'q' },
      { field: 'y', op: 'ne', value: '1' },
    ]);
    expect(qlToConditions('z !~ "w"')).toBeNull();
  });
  it('unescapes quotes', () => {
    expect(qlToConditions('m = "say \\"hi\\""')).toEqual([{ field: 'm', op: 'eq', value: 'say "hi"' }]);
  });
  it('refuses anything more complex', () => {
    expect(qlToConditions('')).toBeNull();
    expect(qlToConditions('a = "1" or b = "2"')).toBeNull();
    expect(qlToConditions('(a = "1")')).toBeNull();
    expect(qlToConditions('a = "1" | head 3')).toBeNull();
    expect(qlToConditions('timeout')).toBeNull();
    expect(qlToConditions('a > 3')).toBeNull();
  });
});
