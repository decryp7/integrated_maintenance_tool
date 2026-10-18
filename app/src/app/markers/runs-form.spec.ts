import { ProfileModel } from '../core/profile.model';
import { applyPlan, emptyForm, fromRule, PROCESS_START_PATTERN, presets, slug, toDraft, withEnabled, without } from './runs-form';

describe('runs form', () => {
  it('derives an id from the label', () => {
    expect(slug('Service running')).toBe('service-running');
    expect(slug('  Ünïcode / job #2 ')).toBe('unicode-job-2');
    expect(slug('!!!')).toBe('run');
    expect(slug('x'.repeat(100))).toHaveLength(64);
  });

  it('turns the form into the engine draft', () => {
    const f = { ...emptyForm(), label: 'Batch job', field: 'ProcessId', maxDuration: ' 12h ', color: '#abc' };
    expect(toDraft(f)).toEqual({
      kind: 'batch-job',
      label: 'Batch job',
      start: { type: 'markers', kinds: ['start'] },
      stop: { type: 'markers', kinds: ['stop'] },
      per: 'field:ProcessId',
      maxDuration: '12h',
      restartOnStart: true,
      color: '#abc',
    });
    const p = toDraft({ ...emptyForm(), kind: 'k', start: { mode: 'pattern', marker: '', pattern: 'up' }, stop: { mode: 'marker', marker: '', pattern: '' } });
    expect(p.start).toEqual({ type: 'pattern', pattern: 'up' });
    expect(p.stop).toEqual({ type: 'markers', kinds: [] });
    expect([p.per, p.maxDuration, p.color]).toEqual(['instance', null, null]);
  });

  it('applies presets, keeping the name of a rule being edited', () => {
    const [service, process, custom] = presets();
    const s = service!.apply(emptyForm());
    expect([s.start.marker, s.stop.marker, s.field, s.kind]).toEqual(['start', 'stop', '', 'service-running']);
    const p = process!.apply(emptyForm());
    expect([p.start.mode, p.start.pattern, p.field, p.kind]).toEqual(['pattern', PROCESS_START_PATTERN, 'ProcessId', 'process']);
    const editing = { ...emptyForm(), label: 'Mine', kind: 'mine', editing: 'mine' };
    expect(process!.apply(editing).label).toBe('Mine');
    expect(custom!.apply(editing)).toBe(editing);
  });

  it('reads an existing rule back into the form', () => {
    const f = fromRule({ kind: 'svc', start: ['a', 'b'], stop: 'c', per: 'field: Worker', max_duration: '1h', restart_on_start: false, color: '#fff' });
    expect(f).toMatchObject({ label: 'svc', editing: 'svc', field: 'Worker', maxDuration: '1h', restartOnStart: false, color: '#fff' });
    expect([f.start.marker, f.stop.marker]).toEqual(['a', 'c']);
    expect(toDraft(f).kind).toBe('svc');
    expect(fromRule({ kind: 'x', label: 'X', start: [], stop: 's' })).toMatchObject({ field: '', maxDuration: '', restartOnStart: true, color: '' });
  });

  it('writes the planned span list and upserts the marker rules', () => {
    const model: ProfileModel = { marker: [{ kind: 'svc-start', when: [] }, { kind: 'fatal' }] };
    applyPlan(model, {
      span: { kind: 'svc', start: 'svc-start', stop: 'svc-stop' },
      spans: [{ kind: 'run', start: 'start', stop: 'stop' }, { kind: 'svc', start: 'svc-start', stop: 'svc-stop' }],
      markers: [
        { kind: 'svc-start', lifecycle: true },
        { kind: 'svc-stop', lifecycle: true },
      ],
    });
    expect(model.span?.map((s) => s.kind)).toEqual(['run', 'svc']);
    expect(model.marker?.map((m) => [m.kind, m.lifecycle])).toEqual([
      ['svc-start', true],
      ['fatal', undefined],
      ['svc-stop', true],
    ]);
    const bare: ProfileModel = {};
    applyPlan(bare, { span: { kind: 'a', start: 's', stop: 't' }, spans: [{ kind: 'a', start: 's', stop: 't' }], markers: [] });
    expect(bare.marker).toEqual([]);
  });

  it('switches rules on and off and removes them', () => {
    const rules = [
      { kind: 'a', start: 's', stop: 't', enabled: true },
      { kind: 'b', start: 's', stop: 't' },
    ];
    const off = withEnabled(rules, 'a', false);
    expect(off[0]?.enabled).toBe(false);
    expect(off[1]).toEqual(rules[1]);
    expect('enabled' in (withEnabled(off, 'a', true)[0] ?? {})).toBe(false);
    expect(without(rules, 'a').map((r) => r.kind)).toEqual(['b']);
  });
});
