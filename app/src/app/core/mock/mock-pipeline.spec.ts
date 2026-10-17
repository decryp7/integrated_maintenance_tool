import { tinyDataset } from '../../../testing/helpers';
import { PipelineStep } from '../profile.model';
import { applyPipeline, jsonFields, kvFields, parseShift } from './mock-pipeline';

const names = (id: number) => ['app.clef', 'app.log'][id];
const run = (steps: PipelineStep[]) => applyPipeline(tinyDataset().records, steps, names);
const total = tinyDataset().records.length;

describe('applyPipeline', () => {
  it('passes everything through without steps and does not mutate the input', () => {
    const input = tinyDataset().records;
    const out = applyPipeline(input, [{ step: 'set', field: 'X', value: '1' }], names);
    expect(out.records).toHaveLength(total);
    expect(input[0].fields['X']).toBeUndefined();
    expect(run([]).stats).toEqual([]);
  });

  it('drops and keeps with per-source counts and stats', () => {
    const drop = run([{ step: 'drop', when: [{ field: '@level', op: 'eq', value: 'Debug' }] }]);
    expect(drop.records).toHaveLength(total - 1);
    expect(drop.dropped.get(1)).toBe(1);
    expect(drop.stats).toEqual([{ index: 0, step: 'drop', input: total, output: total - 1 }]);
    const keep = run([{ step: 'keep', when: [{ field: '@source', op: 'eq', value: 'app.clef' }] }]);
    expect(keep.records.every((r) => r.source === 0)).toBe(true);
    expect(keep.dropped.get(1)).toBe(4);
    expect(run([{ step: 'drop' }]).records).toHaveLength(0);
  });

  it('skips disabled steps and reports the original index of the others', () => {
    const out = run([{ step: 'drop', disabled: true }, { step: 'drop', when: [{ field: '@level', op: 'eq', value: 'Fatal' }] }]);
    expect(out.records).toHaveLength(total - 1);
    expect(out.stats.map((s) => s.index)).toEqual([1]);
  });

  it('masks text with group references and ignores an invalid regex', () => {
    const out = run([{ step: 'mask', field: '@message', regex: '(\\w+) took', replace: '<$1> took' }]);
    expect(out.records.find((r) => r.message.includes('took'))!.message).toBe('Slow request /<api> took 1500 ms');
    const masked = run([{ step: 'mask', regex: 'Cache', replace: 'C' }]).records.find((r) => r.message.startsWith('C hit'));
    expect(masked).toBeTruthy();
    expect(run([{ step: 'mask', regex: '(', replace: '' }]).records.map((r) => r.message)).toEqual(tinyDataset().records.map((r) => r.message));
    const onField = run([{ step: 'mask', field: 'MachineName', regex: 'WEB', replace: 'W' }]).records[0];
    expect(onField.fields['MachineName']).toBe('W01');
    const exc = run([{ step: 'mask', field: '@exception', regex: 'System\\.', replace: '' }]).records.find((r) => r.exception);
    expect(exc!.exception).toContain('TimeoutException');
    const tpl = run([{ step: 'mask', field: '@template', regex: 'Application', replace: 'App' }]).records[0];
    expect(tpl.template).toContain('App starting');
    const lvl = run([{ step: 'mask', field: '@level', regex: 'x', replace: '' }]);
    expect(lvl.records).toHaveLength(total);
  });

  it('sets levels, fields, renames, removes and truncates', () => {
    expect(run([{ step: 'set_level', level: 'Warning', when: [{ field: '@level', op: 'eq', value: 'Debug' }] }]).records.filter((r) => r.level === 'Warning')).toHaveLength(2);
    expect(run([{ step: 'set_level', level: 'Bogus' }]).records.map((r) => r.level)).toEqual(tinyDataset().records.map((r) => r.level));
    expect(run([{ step: 'remove_fields', fields: ['Application'] }]).records.every((r) => !('Application' in r.fields))).toBe(true);
    const renamed = run([{ step: 'rename', from: 'MachineName', to: 'Host' }, { step: 'rename', from: 'Nope', to: 'X' }, { step: 'rename' }]).records[0];
    expect(renamed.fields['Host']).toBe('WEB01');
    expect(renamed.fields['MachineName']).toBeUndefined();
    const set = run([{ step: 'set', field: '@message', value: 'm' }, { step: 'set', field: '@template', value: 't' }, { step: 'set', field: '@exception', value: 'e' }, { step: 'set', field: 'Env', value: 'prod' }, { step: 'set', field: '@level', value: 'x' }, { step: 'set' }]).records[0];
    expect([set.message, set.template, set.exception, set.fields['Env'], set.level]).toEqual(['m', 't', 'e', 'prod', 'Information']);
    const cut = run([{ step: 'truncate', max: 5 }, { step: 'truncate', field: 'Nope', max: 1 }, { step: 'truncate' }]).records[0];
    expect(cut.message).toBe('Appli');
  });

  it('parses JSON and key=value text into fields', () => {
    const base = tinyDataset().records;
    base[0].message = '{"a":1,"b":{"c":2},"d":null}';
    base[1].message = 'user=bob action="log in" n=3';
    base[2].message = '[1]';
    const out = applyPipeline(base, [{ step: 'parse_json', prefix: 'j.' }, { step: 'parse_kv', prefix: 'kv.' }], names).records;
    expect(out[0].fields).toMatchObject({ 'j.a': 1, 'j.b': '{"c":2}', 'j.d': null });
    expect(out[1].fields).toMatchObject({ 'kv.user': 'bob', 'kv.action': 'log in', 'kv.n': '3' });
    expect(out[2].fields['j.0']).toBeUndefined();
    expect(jsonFields('nope')).toBeNull();
    expect(jsonFields('[1]')).toBeNull();
    expect(kvFields('no pairs')).toEqual({});
  });

  it('looks values up in an inline table', () => {
    const out = run([{ step: 'lookup', field: 'MachineName', table: { WEB01: { Site: 'AMS' } } }, { step: 'lookup', field: 'Missing' }, { step: 'lookup', field: '@level' }]).records;
    expect(out.find((r) => r.fields['MachineName'] === 'WEB01')!.fields['Site']).toBe('AMS');
    expect(out.find((r) => r.fields['MachineName'] === 'WEB02')!.fields['Site']).toBeUndefined();
  });

  it('shifts timestamps and leaves untimed events alone', () => {
    const before = tinyDataset().records;
    const out = run([{ step: 'shift_time', by: '-2s' }, { step: 'shift_time', by: 'junk' }, { step: 'shift_time' }]).records;
    expect(out[0].ts).toBe(before[0].ts! - 2000);
    expect(out.find((r) => r.ts === null)).toBeTruthy();
  });

  it('passes dedupe and plugin steps through', () => {
    const out = run([{ step: 'dedupe', fields: ['@message'] }, { step: 'plugin', id: 'x' }]);
    expect(out.records).toHaveLength(total);
    expect(out.stats.map((s) => s.output)).toEqual([total, total]);
  });
});

describe('parseShift', () => {
  it('parses signed durations', () => {
    expect(parseShift('+2.4s')).toBe(2400);
    expect(parseShift('-150ms')).toBe(-150);
    expect(parseShift('1m')).toBe(60000);
    expect(parseShift('1x')).toBeNull();
  });
});
