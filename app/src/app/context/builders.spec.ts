import { RecordView } from '../core/api.types';
import { ProfileModel, toJsRegex } from '../core/profile.model';
import { escapeRegex, generalizeDigits, maskRegexFor, proposeExtraction, suggestFieldName, surroundingContext } from './regex-builder';
import {
  addChartRule,
  addExtractRule,
  addMarkerRule,
  addPipelineStep,
  asFilters,
  chartForField,
  describeConditions,
  dropStep,
  extractRule,
  fieldValueConditions,
  likeThisConditions,
  markerRule,
  maskStep,
  recordFieldText,
  slug,
} from './rule-builders';

function record(patch: Partial<RecordView> = {}): RecordView {
  return {
    id: 1,
    ts: 1, tsState: 'parsed',
    level: 'Warning',
    message: 'Slow request /api/orders took 1500 ms',
    template: 'Slow request {Path} took {Elapsed} ms',
    exception: 'boom',
    fields: { SourceContext: 'Web.Requests', Elapsed: 1500, Nested: { a: 1 } },
    source: 0,
    sourceName: 'app.clef',
    line: 7, byteOffset: 0,
    group: 'g',
    markers: [],
    ...patch,
  };
}

describe('regex-builder', () => {
  it('escapes regex metacharacters', () => {
    expect(escapeRegex('a.b*c(d)[e]{f}|g\\h^$+?')).toBe('a\\.b\\*c\\(d\\)\\[e\\]\\{f\\}\\|g\\\\h\\^\\$\\+\\?');
  });

  it('generalises digit runs', () => {
    expect(generalizeDigits('order 123 of 45.6')).toBe('order \\d+ of \\d+\\.\\d+');
    expect(maskRegexFor('id=42')).toBe('id=\\d+');
  });

  it('trims context to whole words only where it was cut', () => {
    const msg = 'aaaaaaaa bbbbbbbb cccccccc VALUE dddddddd eeeeeeee ffffffff';
    const start = msg.indexOf('VALUE');
    const ctx = surroundingContext(msg, start, start + 5);
    expect(ctx.before.startsWith(' ')).toBe(false);
    expect(ctx.before.endsWith(' ')).toBe(true);
    expect(ctx.before.length).toBeLessThanOrEqual(16);
    expect(ctx.after.length).toBeLessThanOrEqual(12);
    expect(surroundingContext('x 1 y', 2, 3)).toEqual({ before: 'x ', after: ' y' });
  });

  it('suggests a field name from the preceding word', () => {
    expect(suggestFieldName('Process CPU ')).toBe('CPU');
    expect(suggestFieldName('took = ')).toBe('took');
    expect(suggestFieldName('123 ')).toBe('Value');
  });

  it('proposes numeric extractions that match the original message', () => {
    const msg = 'Process stats CPU 10.5% WorkingSet 100 MB';
    const i = msg.indexOf('10.5');
    const p = proposeExtraction(msg, i, i + 4);
    expect(p).toMatchObject({ name: 'CPU', type: 'number', unit: '%' });
    expect(p.regex).toContain('(?P<CPU>[\\d.]+)');
    const m = toJsRegex(p.regex).exec(msg);
    expect(m?.groups?.['CPU']).toBe('10.5');
  });

  it('detects bytes and duration units', () => {
    const bytes = 'WorkingSet 100 MB used';
    const b = bytes.indexOf('100');
    expect(proposeExtraction(bytes, b, b + 3)).toMatchObject({ type: 'bytes', unit: 'MB', to: 'MB', name: 'WorkingSet' });
    const dur = 'took 1500 ms';
    const d = dur.indexOf('1500');
    expect(proposeExtraction(dur, d, d + 4)).toMatchObject({ type: 'duration', unit: 'ms', to: 'ms' });
    const plain = 'count 7 items';
    expect(proposeExtraction(plain, 6, 7)).toMatchObject({ type: 'number' });
    expect(proposeExtraction(plain, 6, 7).unit).toBeUndefined();
  });

  it('proposes string groups for text selections', () => {
    const single = proposeExtraction('user alice logged in', 5, 10);
    expect(single).toMatchObject({ type: 'string', name: 'user' });
    expect(single.regex).toContain('(?P<user>\\S+)');
    const multi = proposeExtraction('reason: disk is full now', 8, 20);
    expect(multi.regex).toContain('(?P<reason>.+?)');
  });
});

describe('rule-builders', () => {
  it('reads record fields including pseudo-fields', () => {
    const r = record();
    expect(recordFieldText(r, '@message')).toBe(r.message);
    expect(recordFieldText(r, '@level')).toBe('Warning');
    expect(recordFieldText(r, '@template')).toBe(r.template);
    expect(recordFieldText(r, '@exception')).toBe('boom');
    expect(recordFieldText(r, '@source')).toBe('app.clef');
    expect(recordFieldText(r, '@line')).toBe('7');
    expect(recordFieldText(r, 'Elapsed')).toBe('1500');
    expect(recordFieldText(r, 'Missing')).toBeNull();
    expect(recordFieldText(record({ template: null, exception: null }), '@template')).toBeNull();
    expect(recordFieldText(record({ template: null, exception: null }), '@exception')).toBeNull();
  });

  it('derives "like this" conditions from logger and template or message', () => {
    expect(likeThisConditions(record())).toEqual([
      { field: 'SourceContext', op: 'eq', value: 'Web.Requests' },
      { field: '@template', op: 'eq', value: 'Slow request {Path} took {Elapsed} ms' },
    ]);
    expect(likeThisConditions(record({ template: null, fields: {} }))).toEqual([{ field: '@message', op: 'eq', value: record().message }]);
    expect(fieldValueConditions('Elapsed', '5')).toEqual([{ field: 'Elapsed', op: 'eq', value: '5' }]);
  });

  it('describes conditions and converts them to filters', () => {
    expect(describeConditions([])).toBe('every event');
    expect(describeConditions([{ field: 'a', op: 'eq', value: 'b' }, { field: 'c', op: 'exists' }])).toBe('a eq "b" and c exists');
    expect(asFilters([{ field: 'a', op: 'in', value: 'x' }, { field: 'b', op: 'eq', value: 'y' }])).toEqual([{ field: 'b', op: 'eq', value: 'y' }]);
  });

  it('appends pipeline steps, extract rules, charts and markers without clashing names', () => {
    const model: ProfileModel = {};
    addPipelineStep(model, dropStep([{ field: 'a', op: 'eq', value: 'b' }]));
    addPipelineStep(model, maskStep('\\d+', '#'));
    expect(model.pipeline).toEqual([
      { step: 'drop', when: [{ field: 'a', op: 'eq', value: 'b' }] },
      { step: 'mask', field: '@message', regex: '\\d+', replace: '#' },
    ]);

    const rule = extractRule({ name: 'x', field: 'CPU', regex: 'r', type: 'number', unit: '%', when: [{ field: 'a', op: 'eq', value: 'b' }] });
    expect(rule).toMatchObject({ name: 'extract-cpu', fields: { CPU: { type: 'number', unit: '%' } } });
    addExtractRule(model, rule);
    addExtractRule(model, rule);
    expect(model.extract?.map((r) => r.name)).toEqual(['extract-cpu', 'extract-cpu-2']);
    const bare = extractRule({ name: 'x', field: 'V', regex: 'r', type: 'bytes', to: 'MB', when: [] });
    expect(bare.when).toBeUndefined();
    expect(bare.fields?.['V']).toEqual({ type: 'bytes', to: 'MB' });
    expect(extractRule({ name: 'x', field: 'V', regex: 'r', type: 'string', to: 'MB', when: [] }).fields?.['V']).toEqual({ type: 'string' });

    const chart = chartForField('CPU', ['MachineName']);
    expect(chart).toMatchObject({ id: 'cpu-chart', y: 'avg(CPU)', split_by: ['MachineName'] });
    addChartRule(model, chart);
    addChartRule(model, chart);
    expect(model.chart?.map((c) => c.id)).toEqual(['cpu-chart', 'cpu-chart-2']);

    addMarkerRule(model, markerRule({ kind: 'k', label: '', color: '#fff', lifecycle: true, when: [] }));
    addMarkerRule(model, markerRule({ kind: 'j', label: 'J', color: '#000', lifecycle: false, when: [] }));
    expect(model.marker).toEqual([
      { kind: 'k', label: 'k', color: '#fff', lifecycle: true, when: [] },
      { kind: 'j', label: 'J', color: '#000', when: [] },
    ]);
  });

  it('makes slugs', () => {
    expect(slug('Payment timeout!')).toBe('payment-timeout');
    expect(slug('!!!')).toBe('rule');
  });
});
