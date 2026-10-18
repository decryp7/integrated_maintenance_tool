import { Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, MemoryLogger, settle, text, typeInto } from '../../testing/helpers';
import { AgentFieldSchema, AgentConfigFile } from '../core/agent.types';
import { FILE_EXISTS_SUFFIX } from '../core/api.types';
import { Logger } from '../core/logger';
import { Platform } from '../core/platform';
import { DialogService } from '../ui/dialog.service';
import { IMT_COMMANDS, IMT_VIEWS } from '../extensions/tokens';
import { MOCK_AGENT_PATH, MockSchedulesBackend, mockAgentSchema, mockStarter, mockValidate } from './mock/mock-schedules-backend';
import { RunsStore } from './runs.store';
import { RunsView } from './runs-view';
import {
  defaultsOf,
  formatBytes,
  issueIndex,
  listToText,
  newAction,
  newSchedule,
  newTrigger,
  parseJsonObject,
  setKey,
  textToList,
  timingOf,
  withTiming,
} from './schedule-model';
import { createSchedulesBackend, SchedulesBackend, TauriSchedulesBackend } from './schedules-backend';
import { SchedulesStore } from './schedules.store';
import { SchedulesView } from './schedules-view';
import { SchemaField } from './schema-field';
import { provideSchedules, SCHEDULES_COMMANDS, SCHEDULES_VIEWS } from './provide-schedules';
import { at } from '../../testing/must';

const NOW = Date.UTC(2026, 9, 17, 12, 0, 0);

function setup(mock = new MockSchedulesBackend({ now: NOW })) {
  TestBed.configureTestingModule({ providers: [{ provide: SchedulesBackend, useValue: mock }, { provide: Logger, useValue: new MemoryLogger() }] });
  return { mock, store: TestBed.inject(SchedulesStore), runs: TestBed.inject(RunsStore) };
}

describe('schedule model', () => {
  const schema = mockAgentSchema();

  it('builds defaults, triggers, actions and schedules from the schema', () => {
    expect(defaultsOf(schema.actions.find((a) => a.kind === 'monitor')!.fields)).toEqual({ targets: [], duration: '5m', intervalMs: 1000, includeChildren: false, dotnet: false });
    expect(defaultsOf([{ key: 'n', label: '', kind: 'number', required: true, help: '', default: null }])).toEqual({ n: 0 });
    expect(newTrigger('process', schema)).toEqual({ kind: 'process', process: '', metric: 'privateBytes', above: 0, forSecs: 0, cooldownSecs: 900 });
    expect(newTrigger('log', schema)).toEqual({ kind: 'log', path: '', ql: '' });
    expect(newAction('perfmon', schema)).toEqual({ kind: 'perfmon', collector: '', duration: '5m' });
    const empty = { ...schema, triggers: [], actions: [] };
    expect(newAction('collect', empty)).toEqual({ kind: 'collect' });
    const s = newSchedule([{ name: 'schedule-2', enabled: true, actions: [] }], schema);
    expect(s).toEqual({ name: 'schedule-3', enabled: true, every: '1h', actions: [{ kind: 'collect' }] });
  });

  it('switches timing and keeps exactly one of cron, every and trigger', () => {
    const s = newSchedule([], schema);
    expect(timingOf(s)).toBe('every');
    const c = withTiming(s, 'cron', schema);
    expect(c.cron).toBe('0 2 * * *');
    expect(c.every).toBeUndefined();
    expect(timingOf(c)).toBe('cron');
    const t = withTiming(c, 'trigger', schema);
    expect(timingOf(t)).toBe('trigger');
    expect(t.cron).toBeUndefined();
    expect(t.trigger?.kind).toBe('process');
    expect(withTiming(t, 'trigger', { ...schema, triggers: [] }).trigger?.kind).toBe('process');
  });

  it('sets keys: empty optional values are removed, required ones kept', () => {
    const o = { a: 1, b: 'x' };
    expect(setKey(o, 'b', '')).toEqual({ a: 1 });
    expect(setKey(o, 'b', '', true)).toEqual({ a: 1, b: '' });
    expect(setKey(o, 'a', Number.NaN)).toEqual({ b: 'x' });
    expect(setKey(o, 'c', [])).toEqual(o);
    expect(setKey(o, 'c', null)).toEqual(o);
    expect(setKey(o, 'c', [1])).toEqual({ ...o, c: [1] });
  });

  it('indexes issues and converts lists, JSON and sizes', () => {
    const idx = issueIndex([{ path: 'a', message: 'x' }, { path: 'a', message: 'y' }, { path: '', message: 'z' }]);
    expect(idx.get('a')).toEqual(['x', 'y']);
    expect(idx.get('')).toEqual(['z']);
    expect(listToText(['a', 'b'])).toBe('a\nb');
    expect(listToText('nope')).toBe('');
    expect(textToList(' a \n\n b ')).toEqual(['a', 'b']);
    expect(parseJsonObject('')).toEqual({ value: null });
    expect(parseJsonObject('{"msinfo": true}')).toEqual({ value: { msinfo: true } });
    expect(parseJsonObject('[1]')).toEqual({ error: expect.stringContaining('JSON object') });
    expect(parseJsonObject('{nope')).toEqual({ error: expect.stringContaining('Not valid JSON') });
    expect(formatBytes(512)).toBe('512 B');
    expect(formatBytes(12_582_912)).toBe('12.0 MB');
    expect(formatBytes(5 * 1024 ** 5)).toBe('5120.0 TB');
  });
});

describe('mock schedules backend', () => {
  it('validates like the agent for the common mistakes', () => {
    const c = mockStarter();
    expect(mockValidate(c)).toEqual([]);
    const bad: AgentConfigFile = {
      ...c,
      agent: { ...c.agent, outputDir: ' ', tickSecs: 0 },
      schedules: [
        { name: '.x', enabled: true, cron: '99', every: '1h', actions: [] },
        { name: '.X', enabled: true, every: 'soon', actions: [{ kind: 'monitor', targets: [], duration: 'x' }, { kind: 'copyLogs', from: '' }, { kind: 'perfmon', collector: '', duration: '1m' }] },
        { name: 'p', enabled: true, trigger: { kind: 'process', process: '', metric: 'cpu', above: 1 }, actions: [{ kind: 'collect' }] },
        { name: 'l', enabled: true, trigger: { kind: 'log', path: '/x', ql: '' }, actions: [{ kind: 'collect' }] },
      ],
    };
    const paths = mockValidate(bad).map((i) => i.path);
    expect(paths).toEqual(
      expect.arrayContaining([
        'agent.outputDir',
        'agent.tickSecs',
        'schedules[0].name',
        'schedules[0]',
        'schedules[0].cron',
        'schedules[0].actions',
        'schedules[1].name',
        'schedules[1].every',
        'schedules[1].actions[0].duration',
        'schedules[1].actions[0].targets',
        'schedules[1].actions[1].from',
        'schedules[1].actions[2].collector',
        'schedules[2].trigger.process',
        'schedules[3].trigger.ql',
      ]),
    );
  });

  it('serves runs, manifests and opens runs; fails like the shell for unknown runs and unsaved configs', async () => {
    const m = new MockSchedulesBackend({ now: NOW });
    expect((await m.schema()).version).toBe(1);
    const listing = await m.runs();
    expect(listing.runs.map((r) => r.status)).toEqual(['ok', 'partial', 'failed']);
    const ok = await m.manifest(at(listing.runs, 0).id);
    expect(ok.actions[1]?.status).toBe('ok');
    const partial = await m.manifest(at(listing.runs, 1).id);
    expect(partial.reason.kind).toBe('trigger');
    await expect(m.manifest('nope')).rejects.toContain('no longer exists');
    expect(await m.openRun(at(listing.runs, 0).id)).toBe('window-2');
    await expect(m.openRun('nope')).rejects.toContain('no longer exists');
    const fresh = new MockSchedulesBackend({ saved: false });
    expect((await fresh.load()).exists).toBe(false);
    await expect(fresh.runs()).rejects.toContain('save the schedules first');
    expect(await fresh.save({ ...mockStarter(), agent: { ...mockStarter().agent, outputDir: '' } })).toHaveLength(1);
    expect(await fresh.save(mockStarter())).toEqual([]);
    expect((await fresh.load('/x/a.toml')).path).toBe('/x/a.toml');
    // Like the engine: an existing file is replaced only with `overwrite`.
    await expect(fresh.save(mockStarter())).rejects.toThrow(`${MOCK_AGENT_PATH}${FILE_EXISTS_SUFFIX}`);
    expect(await fresh.save(mockStarter(), undefined, true)).toEqual([]);
    expect(fresh.calls.at(-1)).toBe('save::overwrite');
    expect((await fresh.load('/x/a.toml')).exists).toBe(false);
  });
});

describe('TauriSchedulesBackend', () => {
  it('invokes the agent commands with their arguments', async () => {
    const invoke = vi.fn(async () => 'ok');
    const b = new TauriSchedulesBackend(invoke as never);
    const cfg = mockStarter();
    await b.schema();
    await b.load();
    await b.load('/a.toml');
    await b.validate(cfg);
    await b.save(cfg, '/a.toml');
    await b.save(cfg, '/a.toml', true);
    await b.runs();
    await b.manifest('r1');
    await b.openRun('r1', '/a.toml');
    expect(invoke.mock.calls).toEqual([
      ['agent_schema'],
      ['agent_config_load', { path: null }],
      ['agent_config_load', { path: '/a.toml' }],
      ['agent_config_validate', { path: null, config: cfg }],
      ['agent_config_save', { path: '/a.toml', config: cfg }],
      ['agent_config_save', { path: '/a.toml', config: cfg, overwrite: true }],
      ['agent_runs', { path: null }],
      ['agent_run_manifest', { path: null, runId: 'r1' }],
      ['agent_open_run', { path: '/a.toml', runId: 'r1' }],
    ]);
  });

  it('uses the mock outside Tauri', async () => {
    TestBed.configureTestingModule({ providers: [{ provide: Logger, useValue: new MemoryLogger() }] });
    const b = TestBed.runInInjectionContext(() => createSchedulesBackend());
    expect((await b.load()).path).toBe(MOCK_AGENT_PATH);
  });
});

describe('SchedulesStore', () => {
  it('loads, edits, validates and saves', async () => {
    const { store, mock } = setup();
    await store.load();
    expect(store.config()?.schedules[0]?.name).toBe('nightly');
    expect(store.path()).toBe(MOCK_AGENT_PATH);
    expect(store.exists()).toBe(true);
    store.addSchedule();
    expect(store.schedules()).toHaveLength(2);
    expect(store.dirty()).toBe(true);
    store.setScheduleKey(1, 'name', 'spike', true);
    store.setTiming(1, 'trigger');
    store.setTriggerKind(1, 'log');
    store.setTriggerKey(1, 'path', '/var/log/app.log', true);
    store.setTriggerKey(1, 'ql', 'level >= Error', true);
    store.addAction(1, 'monitor');
    store.setActionKey(1, 1, 'targets', ['svc*'], true);
    store.setActionKind(1, 0, 'perfmon');
    store.setActionKey(1, 0, 'collector', 'IMT', true);
    store.removeAction(1, 2);
    store.setAgent('stateDir', '');
    store.setAgent('tickSecs', 30, true);
    store.setLog('level', 'debug');
    store.setRetention('maxRuns', undefined);
    const c = store.config()!;
    expect(c.schedules[1]).toEqual({
      name: 'spike',
      enabled: true,
      trigger: { kind: 'log', path: '/var/log/app.log', ql: 'level >= Error' },
      actions: [{ kind: 'perfmon', collector: 'IMT', duration: '5m' }, { kind: 'monitor', targets: ['svc*'], duration: '5m', intervalMs: 1000, includeChildren: false, dotnet: false }],
    });
    expect(c.agent.tickSecs).toBe(30);
    expect(c.agent.log.level).toBe('debug');
    expect(c.retention.maxRuns).toBeNull();

    await store.validate();
    expect(store.issues()).toEqual([]);
    expect(store.notice()).toContain('valid');
    await store.save();
    expect(store.dirty()).toBe(false);
    expect(store.notice()).toBe(`Saved ${MOCK_AGENT_PATH}`);
    // Saving back to the loaded file replaces it without asking.
    expect(mock.calls).toContain(`save:${MOCK_AGENT_PATH}:overwrite`);

    store.removeSchedule(1);
    expect(store.schedules()).toHaveLength(1);
  });

  it('never replaces another existing file without asking', async () => {
    const mock = new MockSchedulesBackend({ saved: false });
    TestBed.configureTestingModule({
      providers: [{ provide: SchedulesBackend, useValue: mock }, { provide: Logger, useValue: new MemoryLogger() }, { provide: Platform, useValue: {} }],
    });
    const store = TestBed.inject(SchedulesStore);
    const dialogs = TestBed.inject(DialogService);
    await store.load();
    expect(store.exists()).toBe(false);
    // Created meanwhile (e.g. by imt-agent itself): the starter would replace it, so the app asks.
    await mock.save(mockStarter());
    let saving = store.save();
    await vi.waitFor(() => expect(dialogs.stack()[0]?.options.title).toBe('Replace file?'));
    expect(dialogs.stack()[0]?.options.inputs?.['detail']).toBe(MOCK_AGENT_PATH);
    at(dialogs.stack(), 0).ref.close(false);
    await saving;
    expect(store.notice()).toBe('Not saved: the existing file was kept.');
    expect(store.error()).toBeNull();
    saving = store.save();
    await vi.waitFor(() => expect(dialogs.stack()).toHaveLength(1));
    at(dialogs.stack(), 0).ref.close(true);
    await saving;
    expect(store.notice()).toBe(`Saved ${MOCK_AGENT_PATH}`);
    // Now it is the editor's own file: saved again without asking.
    await store.save();
    expect(dialogs.stack()).toHaveLength(0);
    expect(mock.calls.slice(-1)).toEqual([`save:${MOCK_AGENT_PATH}:overwrite`]);
    // Another path typed in that already exists: asked again.
    await mock.save(mockStarter(), '/etc/other.toml');
    store.setPath('/etc/other.toml');
    saving = store.save();
    await vi.waitFor(() => expect(dialogs.stack()).toHaveLength(1));
    at(dialogs.stack(), 0).ref.close(true);
    await saving;
    expect(mock.calls.slice(-2)).toEqual(['save:/etc/other.toml', 'save:/etc/other.toml:overwrite']);
  });

  it('keeps an invalid draft unsaved and shows its issues by path', async () => {
    const { store } = setup();
    await store.load();
    store.setScheduleKey(0, 'cron', '99', true);
    await store.validate();
    expect(store.issuesByPath().get('schedules[0].cron')?.[0]).toContain('invalid cron');
    await store.save();
    expect(store.error()).toBe('Not saved: 1 problem to fix.');
    expect(store.dirty()).toBe(true);
    store.dismissError();
    expect(store.error()).toBeNull();
    store.addSchedule();
    store.setScheduleKey(1, 'cron', '99', true);
    store.setTiming(1, 'cron');
    store.setScheduleKey(1, 'cron', '1', true);
    await store.save();
    expect(store.error()).toBe('Not saved: 2 problems to fix.');
  });

  it('starts from a starter document and reports backend failures', async () => {
    const mock = new MockSchedulesBackend({ saved: false });
    const { store } = setup(mock);
    store.setPath('/etc/imt-agent.toml');
    await store.load();
    expect(store.exists()).toBe(false);
    expect(store.notice()).toContain('starter configuration');
    expect(mock.calls).toContain('load:/etc/imt-agent.toml');
    vi.spyOn(mock, 'validate').mockRejectedValueOnce('engine down');
    await store.validate();
    expect(store.error()).toBe('engine down');
    // A second load reuses the schema.
    const schema = vi.spyOn(mock, 'schema');
    await store.load();
    expect(schema).not.toHaveBeenCalled();
  });

  it('does nothing without a document', async () => {
    const { store, mock } = setup();
    await store.validate();
    await store.save();
    store.addSchedule();
    store.setTiming(0, 'cron');
    store.setTriggerKind(0, 'log');
    store.addAction(0);
    store.setActionKind(0, 0, 'collect');
    store.setAgent('x', 1);
    expect(store.config()).toBeNull();
    expect(mock.calls).toEqual([]);
    // A schedule without a trigger ignores trigger keys.
    await store.load();
    const before = store.config()!.schedules[0];
    store.setTriggerKey(0, 'path', '/x');
    expect(store.config()!.schedules[0]).toEqual(before);
  });
});

describe('RunsStore', () => {
  it('lists runs, shows a manifest, opens a run in a new window', async () => {
    const { runs, mock } = setup();
    await runs.refresh();
    expect(runs.outputDir()).toBe('/demo/agent/runs');
    expect(runs.runs()).toHaveLength(3);
    const id = at(runs.runs(), 0).id;
    await runs.toggle(id);
    expect(runs.manifest()?.id).toBe(id);
    await runs.toggle(id);
    expect(runs.selected()).toBeNull();
    await runs.open(id);
    expect(runs.notice()).toContain('new window');
    expect(mock.calls).toContain(`openRun:${id}`);
    await runs.open('nope');
    expect(runs.error()).toContain('no longer exists');
    await runs.toggle('nope');
    expect(runs.error()).toContain('no longer exists');
    runs.dismiss();
    expect(runs.error()).toBeNull();
  });

  it('shows why runs cannot be listed', async () => {
    const { runs } = setup(new MockSchedulesBackend({ saved: false }));
    await runs.refresh();
    expect(runs.error()).toContain('save the schedules first');
    expect(runs.runs()).toEqual([]);
  });
});

@Component({
  imports: [SchemaField],
  template: `<imt-schema-field [field]="field()" [value]="value()" [issues]="issues()" (valueChange)="got.push($event)" />`,
})
class FieldHost {
  readonly field = signal<AgentFieldSchema>({ key: 'k', label: 'Label', kind: 'text', required: true, help: 'Help', default: null });
  readonly value = signal<unknown>('x');
  readonly issues = signal<string[] | undefined>(undefined);
  readonly got: unknown[] = [];
}

describe('SchemaField', () => {
  async function render(field: Partial<AgentFieldSchema>, value: unknown) {
    const fixture = TestBed.createComponent(FieldHost);
    fixture.componentInstance.field.update((f) => ({ ...f, ...field }));
    fixture.componentInstance.value.set(value);
    await settle(fixture);
    return fixture;
  }

  it('renders each kind and emits typed values', async () => {
    let f = await render({ kind: 'text' }, 'abc');
    const input = el<HTMLInputElement>(f, 'input');
    expect(input.value).toBe('abc');
    expect(input.getAttribute('aria-describedby')).toContain('-help');
    typeInto(input, 'def');
    expect(f.componentInstance.got).toEqual(['def']);
    expect(text(f)).toContain('Label *');

    f = await render({ kind: 'duration', help: '' }, undefined);
    expect(el<HTMLInputElement>(f, 'input').placeholder).toBe('e.g. 15m');
    expect(el<HTMLInputElement>(f, 'input').getAttribute('aria-describedby')).toBeNull();
    f = await render({ kind: 'cron' }, '0 2 * * *');
    expect(el<HTMLInputElement>(f, 'input').placeholder).toBe('0 2 * * *');
    f = await render({ kind: 'query' }, null);
    expect(el<HTMLInputElement>(f, 'input').placeholder).toBe('level >= Error');
    f = await render({ kind: 'path' }, '/x');
    expect(el<HTMLInputElement>(f, 'input').placeholder).toBe('');

    f = await render({ kind: 'number', min: 1, max: 9 }, 5);
    const num = el<HTMLInputElement>(f, 'input');
    expect(num.getAttribute('min')).toBe('1');
    typeInto(num, '7');
    typeInto(num, '');
    expect(f.componentInstance.got).toEqual([7, undefined]);

    f = await render({ kind: 'boolean' }, true);
    const box = el<HTMLInputElement>(f, 'input[type=checkbox]');
    expect(box.checked).toBe(true);
    box.checked = false;
    box.dispatchEvent(new Event('change'));
    expect(f.componentInstance.got).toEqual([false]);

    f = await render({ kind: 'select', options: [{ value: 'a', label: 'A' }, { value: 'b', label: 'B' }] }, 'b');
    const sel = el<HTMLSelectElement>(f, 'select');
    expect(sel.value).toBe('b');
    typeInto(sel, 'a', 'change');
    expect(f.componentInstance.got).toEqual(['a']);
    f = await render({ kind: 'select', options: undefined }, 'b');
    expect(all(f, 'option')).toHaveLength(0);

    f = await render({ kind: 'list' }, ['a', 'b']);
    const ta = el<HTMLTextAreaElement>(f, 'textarea');
    expect(ta.value).toBe('a\nb');
    typeInto(ta, 'x\n y ');
    expect(f.componentInstance.got).toEqual([['x', 'y']]);
  });

  it('parses JSON fields and shows local and engine problems', async () => {
    const f = await render({ kind: 'json', required: false }, { msinfo: true });
    const ta = el<HTMLTextAreaElement>(f, 'textarea');
    expect(ta.value).toBe('{"msinfo":true}');
    typeInto(ta, '{bad');
    await settle(f);
    expect(text(f)).toContain('Not valid JSON');
    expect(ta.getAttribute('aria-invalid')).toBe('true');
    typeInto(ta, '{"a": 1}');
    typeInto(ta, '');
    expect(f.componentInstance.got).toEqual([{ a: 1 }, undefined]);
    f.componentInstance.issues.set(['plan: name must not be empty']);
    await settle(f);
    expect(el(f, '[role=alert]').textContent).toContain('name must not be empty');
    const g = await render({ kind: 'json' }, 'not an object');
    expect(el<HTMLTextAreaElement>(g, 'textarea').value).toBe('');
  });
});

describe('SchedulesView', () => {
  async function view(mock = new MockSchedulesBackend({ now: NOW })) {
    const { store } = setup(mock);
    const fixture = TestBed.createComponent(SchedulesView);
    await settle(fixture);
    await settle(fixture);
    return { fixture, store, mock };
  }

  it('renders the document from the schema and edits it', async () => {
    const { fixture, store } = await view();
    expect(text(fixture)).toContain('Output folder');
    expect(el(fixture, '[data-testid=schedule-0] legend').textContent).toContain('nightly');
    expect(el(fixture, '[data-testid=schedules-state]').textContent).toContain('Saved');
    buttonByText(fixture, 'Add schedule').click();
    await settle(fixture);
    expect(all(fixture, 'fieldset')).toHaveLength(2);
    expect(el(fixture, '[data-testid=schedules-state]').textContent).toContain('Unsaved changes');
    // Switch the new schedule to a trigger, then to a log trigger.
    const timing = el<HTMLSelectElement>(fixture, '[data-testid=schedule-1] select');
    typeInto(timing, 'trigger', 'change');
    await settle(fixture);
    expect(text(fixture)).toContain('Process name');
    const triggerSel = at(all<HTMLSelectElement>(fixture, '[data-testid=schedule-1] select'), 1);
    typeInto(triggerSel, 'log', 'change');
    await settle(fixture);
    expect(text(fixture)).toContain('Log file or folder');
    // Change the action kind and add / remove actions.
    const actionSel = el<HTMLSelectElement>(fixture, '[data-testid=action-1-0] select');
    typeInto(actionSel, 'copyLogs', 'change');
    await settle(fixture);
    expect(at(store.config()!.schedules, 1).actions[0]?.kind).toBe('copyLogs');
    all<HTMLButtonElement>(fixture, '[data-testid=schedule-1] button').find((b) => b.textContent?.includes('Add action'))!.click();
    await settle(fixture);
    expect(store.config()!.schedules[1]?.actions).toHaveLength(2);
    el<HTMLButtonElement>(fixture, '[aria-label="Remove action 2 of schedule-2"]').click();
    await settle(fixture);
    expect(store.config()!.schedules[1]?.actions).toHaveLength(1);
    // Edit a field through the generic renderer.
    const name = el<HTMLInputElement>(fixture, '[data-testid=schedule-1] imt-schema-field input');
    typeInto(name, 'spike');
    expect(store.config()!.schedules[1]?.name).toBe('spike');
    // Agent, log and retention fields.
    const tick = at(all<HTMLInputElement>(fixture, 'section imt-schema-field input[type=number]'), 1);
    typeInto(tick, '20');
    expect(store.config()!.agent.tickSecs).toBe(20);
    const logLevel = at(all<HTMLSelectElement>(fixture, 'section imt-schema-field select'), 1);
    typeInto(logLevel, 'warning', 'change');
    expect(store.config()!.agent.log.level).toBe('warning');
    const keepRuns = el<HTMLInputElement>(fixture, '[aria-labelledby=sch-retention] input, section:nth-of-type(2) input');
    typeInto(keepRuns, '');
    expect(store.config()!.retention.maxRuns).toBeNull();
    buttonByText(fixture, 'Remove schedule').click();
    await settle(fixture);
    expect(all(fixture, 'fieldset')).toHaveLength(1);
  });

  it('validates and shows engine issues next to fields and for the file', async () => {
    const { fixture, store } = await view();
    store.setScheduleKey(0, 'cron', '99', true);
    store.setTiming(0, 'cron');
    store.setScheduleKey(0, 'cron', '1', true);
    store.removeAction(0, 0);
    buttonByText(fixture, 'Validate').click();
    await settle(fixture);
    expect(text(fixture)).toContain('invalid cron expression');
    expect(text(fixture)).toContain('add at least one action');
    expect(el(fixture, '[data-testid=schedules-state]').textContent).toContain('2 problems');
    store.addSchedule();
    store.setScheduleKey(1, 'cron', '0 * * * *');
    await store.validate();
    await settle(fixture);
    expect(text(fixture)).toContain('set exactly one of');
    buttonByText(fixture, 'Save').click();
    await settle(fixture);
    expect(text(fixture)).toContain('Not saved');
    el<HTMLButtonElement>(fixture, '[aria-label=Dismiss]').click();
    await settle(fixture);
    expect(text(fixture)).not.toContain('Not saved');
  });

  it('shows file-level issues, the path input and the empty state', async () => {
    const mock = new MockSchedulesBackend();
    vi.spyOn(mock, 'load').mockResolvedValueOnce({ path: '/x.toml', exists: true, config: null, issues: [{ path: '', message: 'not a valid imt-agent.toml' }, { path: 'version', message: 'too new' }] });
    const { fixture, store } = await view(mock);
    expect(el(fixture, '[data-testid=file-issues]').textContent).toContain('not a valid imt-agent.toml');
    expect(el(fixture, '[data-testid=file-issues]').textContent).toContain('version: too new');
    expect(text(fixture)).toContain('Load the configuration');
    const path = el<HTMLInputElement>(fixture, 'input.mono');
    typeInto(path, '/etc/imt-agent.toml');
    expect(store.path()).toBe('/etc/imt-agent.toml');
    buttonByText(fixture, 'Load').click();
    await settle(fixture);
    expect(store.config()).not.toBeNull();
    // A busy state is shown as the status text.
    store.setPath('');
    const pending = store.load();
    await settle(fixture);
    await pending;
  });
});

describe('RunsView', () => {
  it('lists runs, toggles details and opens a run', async () => {
    const { mock } = setup();
    const fixture = TestBed.createComponent(RunsView);
    await settle(fixture);
    await settle(fixture);
    expect(all(fixture, 'tbody tr')).toHaveLength(3);
    expect(text(fixture)).toContain('/demo/agent/runs');
    expect(text(fixture)).toContain('Partial');
    expect(text(fixture)).toContain('12.0 MB');
    const id = '20261017-020000-nightly';
    el<HTMLButtonElement>(fixture, `[aria-label="Details of ${id}"]`).click();
    await settle(fixture);
    expect(el(fixture, '[data-testid=run-manifest]').textContent).toContain('Host WEB01');
    el<HTMLButtonElement>(fixture, `[aria-label="Open ${id} in a new window"]`).click();
    await settle(fixture);
    expect(mock.calls).toContain(`openRun:${id}`);
    expect(text(fixture)).toContain('new window');
    // Details of a failing run show the action message.
    const failed = '20261016-020000-nightly';
    el<HTMLButtonElement>(fixture, `[aria-label="Details of ${failed}"]`).click();
    await settle(fixture);
    expect(el(fixture, '[data-testid=run-manifest]').textContent).toContain('path does not exist');
    buttonByText(fixture, 'Refresh').click();
    await settle(fixture);
    expect(mock.calls.filter((c) => c === 'runs')).toHaveLength(2);
  });

  it('shows the empty state, truncation and errors', async () => {
    const mock = new MockSchedulesBackend();
    vi.spyOn(mock, 'runs').mockResolvedValueOnce({ outputDir: '/o', runs: [], truncated: 0 });
    setup(mock);
    const fixture = TestBed.createComponent(RunsView);
    await settle(fixture);
    expect(text(fixture)).toContain('No runs yet');
    const store = TestBed.inject(RunsStore);
    vi.spyOn(mock, 'runs').mockResolvedValueOnce({ outputDir: '/o', runs: (await new MockSchedulesBackend().runs()).runs, truncated: 4 });
    await store.refresh();
    await settle(fixture);
    expect(text(fixture)).toContain('4 older runs are not listed');
    vi.spyOn(mock, 'manifest').mockReturnValueOnce(new Promise(() => undefined));
    void store.toggle(at(store.runs(), 0).id);
    await settle(fixture);
    expect(text(fixture)).toContain('Loading…');
    vi.spyOn(mock, 'runs').mockRejectedValueOnce('boom');
    await store.refresh();
    await settle(fixture);
    expect(el(fixture, '[role=alert]').textContent).toContain('boom');
    expect(text(fixture)).toContain('Runs of the collection schedules');
  });
});

describe('provideSchedules', () => {
  it('registers the two views and their commands', () => {
    TestBed.configureTestingModule({ providers: [provideSchedules()] });
    const views = TestBed.inject(IMT_VIEWS).flat();
    expect(views.map((v) => v.id)).toEqual(['collection-schedules', 'collection-runs']);
    expect(TestBed.inject(IMT_COMMANDS).flat()).toEqual(SCHEDULES_COMMANDS);
    const shown: string[] = [];
    for (const c of SCHEDULES_COMMANDS) void c.run({ showView: (id: string) => shown.push(id) } as never);
    expect(shown).toEqual(['collection-schedules', 'collection-runs']);
  });

  it('loads the view components lazily', async () => {
    expect(await SCHEDULES_VIEWS[0]?.component()).toBe(SchedulesView);
    expect(await SCHEDULES_VIEWS[1]?.component()).toBe(RunsView);
  });
});
