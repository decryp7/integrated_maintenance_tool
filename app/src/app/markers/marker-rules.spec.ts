import { ProfileModel } from '../core/profile.model';
import { cleanRule, describeCondition, describeWhen, iconNameFromFile, moveRow, nextFreeBadge, pruneIcons, readRows, ruleProblems, uniqueKind, upsertIcon, writeRows } from './marker-rules';
import { RuleRow } from './markers.types';

const row = (kind: string, extra: Partial<RuleRow['rule']> = {}, enabled = true): RuleRow => ({ rule: { kind, ...extra }, enabled });

describe('rows', () => {
  it('round-trips disabled rules at their old positions', () => {
    const model: ProfileModel = { name: 'x', marker: [{ kind: 'a' }, { kind: 'c' }], unknown: 1 };
    const rows = [row('a'), row('b', {}, false), row('c'), row('d', {}, false)];
    writeRows(model, rows);
    expect(model.marker?.map((m) => m.kind)).toEqual(['a', 'c']);
    expect((model['imt_disabled_markers'] as { kind: string; imt_index: number }[]).map((d) => [d.kind, d.imt_index])).toEqual([['b', 1], ['d', 3]]);
    expect(model['unknown']).toBe(1);
    expect(readRows(model).map((r) => [r.rule.kind, r.enabled])).toEqual([['a', true], ['b', false], ['c', true], ['d', false]]);
    expect(readRows(model)[1].rule).not.toHaveProperty('imt_index');
  });
  it('drops the disabled key when nothing is disabled and tolerates a missing index', () => {
    const model: ProfileModel = { imt_disabled_markers: [{ kind: 'z' }] };
    expect(readRows(model).map((r) => r.rule.kind)).toEqual(['z']);
    writeRows(model, [row('a')]);
    expect(model).not.toHaveProperty('imt_disabled_markers');
    expect(readRows({})).toEqual([]);
  });
  it('moves rows within bounds', () => {
    expect(moveRow([1, 2, 3], 0, 2)).toEqual([2, 3, 1]);
    expect(moveRow([1, 2, 3], 2, -5)).toEqual([3, 1, 2]);
    expect(moveRow([1, 2, 3], 9, 0)).toEqual([1, 2, 3]);
  });
});

describe('suggestions', () => {
  it('numbers, then letters, then two digits', () => {
    expect(nextFreeBadge([])).toBe('1');
    expect(nextFreeBadge([row('a', { badge: '1' }), row('b', { badge: '2' })])).toBe('3');
    const nine = Array.from({ length: 9 }, (_, i) => row('k' + i, { badge: String(i + 1) }));
    expect(nextFreeBadge(nine)).toBe('A');
    const letters = [...nine, ...Array.from({ length: 26 }, (_, i) => row('l' + i, { badge: String.fromCharCode(97 + i) }))];
    expect(nextFreeBadge(letters)).toBe('10');
    const all = [...letters, ...Array.from({ length: 90 }, (_, i) => row('m' + i, { badge: String(i + 10) }))];
    expect(nextFreeBadge(all)).toBe('');
  });
  it('makes unique kinds, ignoring the rule being edited', () => {
    const rows = [row('slow'), row('slow-2')];
    expect(uniqueKind(rows, 'slow')).toBe('slow-3');
    expect(uniqueKind(rows, 'slow', 'slow')).toBe('slow');
    expect(uniqueKind(rows, 'fresh')).toBe('fresh');
  });
});

describe('descriptions', () => {
  it('writes conditions in plain English', () => {
    expect(describeCondition({ field: '@level', op: 'eq', value: 'Error' })).toBe('level is "Error"');
    expect(describeCondition({ field: '@exception', op: 'exists' })).toBe('exception is present');
    expect(describeCondition({ field: 'Foo', op: 'matches', value: 'a+' })).toBe('Foo matches regex "a+"');
    expect(describeCondition({ field: 'Foo', op: 'missing', value: 'x' })).toBe('Foo is absent');
    expect(describeCondition({ field: 'Foo', op: 'bogus' as never, value: 'x' })).toBe('Foo bogus "x"');
    expect(describeCondition({ field: 'Foo', op: 'gt' })).toBe('Foo is greater than');
    expect(describeWhen([{ field: '@message', op: 'contains', value: 'x' }, { field: '@source', op: 'ne', value: 'y' }])).toBe('message contains "x" and source file is not "y"');
    expect(describeWhen([])).toBe('every event');
    expect(describeWhen(undefined)).toBe('every event');
  });
});

describe('custom icons', () => {
  it('adds, replaces and prunes icons', () => {
    const model: ProfileModel = {};
    upsertIcon(model, { name: 'a', svg: '<svg/>' });
    upsertIcon(model, { name: 'b', png: 'AAAA', pngDataUrl: 'x' });
    upsertIcon(model, { name: 'a', svg: '<svg v2/>' });
    expect(model['icon']).toEqual([{ name: 'b', png: 'AAAA' }, { name: 'a', svg: '<svg v2/>' }]);
    pruneIcons(model, [row('k', { icon: 'a' })]);
    expect(model['icon']).toEqual([{ name: 'a', svg: '<svg v2/>' }]);
    pruneIcons(model, []);
    expect(model).not.toHaveProperty('icon');
    pruneIcons(model, []);
  });
  it('names icons after files', () => {
    expect(iconNameFromFile('My Logo!.svg', new Set())).toBe('My-Logo');
    expect(iconNameFromFile('a.svg', new Set(['a', 'a-2']))).toBe('a-3');
    expect(iconNameFromFile('???.png', new Set())).toBe('icon');
  });
});

describe('rule checks', () => {
  it('reports early problems', () => {
    expect(ruleProblems({ kind: 'k', when: [{ field: 'f', op: 'eq', value: '1' }] })).toEqual([]);
    expect(ruleProblems({ kind: '', color: 'red', badge: 'ABCD' })).toHaveLength(4);
    expect(ruleProblems({ kind: 'k', badge: '', when: [{ field: 'f', op: 'eq' }] })).toEqual(['A badge is 1 to 3 characters.']);
  });
  it('removes empty optional keys', () => {
    expect(cleanRule({ kind: 'k', label: '', description: '', icon: undefined, badge: '1', lifecycle: false, color: '#fff', x: 1 })).toEqual({ kind: 'k', badge: '1', color: '#fff', x: 1 });
    expect(cleanRule({ kind: 'k', lifecycle: true }).lifecycle).toBe(true);
  });
});
