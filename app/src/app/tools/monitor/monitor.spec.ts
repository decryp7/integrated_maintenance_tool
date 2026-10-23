import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, settle, text, typeInto } from '../../../testing/helpers';
import { WorkspaceStore } from '../../core/workspace.store';
import { LifecycleService } from '../../lifecycle/lifecycle.service';
import { LiveTail } from '../../workspace/live-tail';
import { MockToolsOptions } from '../mock/mock-tools-backend';
import { mockTools, provideToolsTesting } from '../mock/test-support';
import { MonitorView } from './monitor-view';
import { ACCESS_DENIED_FIELD, MonitorStore } from './monitor.store';
import { at } from '../../../testing/must';
import { pick } from '../../settings/themed-select.testing';
import { FakeECharts } from '../../../testing/fake-echarts';

// The view hosts the live charts once a run starts: a fake ECharts keeps canvas out of jsdom.
const charts = new FakeECharts();

async function setup(options: Partial<MockToolsOptions> = {}) {
  TestBed.configureTestingModule({ providers: [...provideToolsTesting(options), charts.provider] });
  const fixture = TestBed.createComponent(MonitorView);
  await TestBed.inject(LifecycleService).init();
  await settle(fixture);
  return { fixture, store: TestBed.inject(MonitorStore), tools: mockTools(), workspace: TestBed.inject(WorkspaceStore) };
}

describe('MonitorStore', () => {
  it('edits target rows and never leaves the editor without a row', async () => {
    const { store } = await setup();
    store.addTarget();
    store.setTarget(1, { kind: 'pid', value: '12' });
    expect(store.draft().targets).toEqual([{ kind: 'name', value: '' }, { kind: 'pid', value: '12' }]);
    store.removeTarget(0);
    store.removeTarget(0);
    expect(store.draft().targets).toEqual([{ kind: 'name', value: '' }]);
    expect(store.problems()).toHaveLength(1);
  });

  it('starts: opens the folder, turns live tail on, then stops', async () => {
    const { store, tools, workspace } = await setup();
    store.setTarget(0, { value: 'svc*' });
    store.patchDraft({ dir: ' /out ' });
    await store.start();
    expect(store.running()).toBe(true);
    expect(store.dir()).toBe('/out');
    expect(tools.calls).toEqual(['monitorStart:/out']);
    expect(TestBed.inject(LiveTail).enabled()).toBe(true);
    expect(workspace.summary()?.sources.length).toBeGreaterThan(0);
    expect(store.notice()).toContain('started');
    await store.stop();
    expect(store.running()).toBe(false);
    expect(store.notice()).toBe('Monitoring stopped.');
    await store.stop();
    expect(store.dir()).toBe('/out');
  });

  it('uses the default folder and picks up a run that is already going', async () => {
    const { store } = await setup();
    store.setTarget(0, { value: 'svc' });
    await store.start();
    expect(store.dir()).toBe('/demo/monitor/run-1');
    store.dismissError();
    await store.start();
    expect(store.error()).toContain('already running');
    await store.refresh();
    expect(store.running()).toBe(true);
    await store.stop();
    await store.refresh();
    expect(store.running()).toBe(false);
  });

  it('counts events that carry AccessDenied, and offers counters on Windows only', async () => {
    const { store, workspace } = await setup();
    expect(store.accessDeniedSeen()).toBe(0);
    expect(store.countersAvailable()).toBe(true);
    const s = workspace.summary()!;
    workspace.applySummary({ ...s, fields: [...s.fields, { name: ACCESS_DENIED_FIELD, count: 3, distinct: 1, numeric: false, derived: false }] });
    expect(store.accessDeniedSeen()).toBe(3);
  });
});

describe('MonitorView', () => {
  it('configures targets and sampling, starts and stops with a running status', async () => {
    const { fixture, store } = await setup();
    expect(el(fixture, '[data-testid=monitor-status]').textContent).toContain('Stopped');
    expect(buttonByText(fixture, 'Start').disabled).toBe(true);
    // No validation message before the user has touched the form.
    expect(text(fixture)).not.toContain('Add at least one process');

    await pick(fixture, 'imt-select[data-testid=target-type-0] button', 'Executable path');
    await settle(fixture);
    expect(text(fixture)).toContain('Add at least one process');
    typeInto(el(fixture, 'input[aria-label="Target value 1"]'), 'C:\\svc\\*');
    buttonByText(fixture, /Add target/).click();
    await settle(fixture);
    await pick(fixture, 'imt-select[data-testid=target-type-1] button', 'Process id');
    typeInto(el(fixture, 'input[aria-label="Target value 2"]'), 'abc');
    await settle(fixture);
    expect(text(fixture)).toContain('"abc" is not a valid process id');
    el<HTMLButtonElement>(fixture, 'button[aria-label="Remove target 2"]').click();
    await settle(fixture);

    typeInto(el(fixture, 'input[type=number]'), '1000');
    await pick(fixture, 'imt-select[data-testid=cpu-mode] button', 'one core');
    el<HTMLInputElement>(fixture, 'input[type=checkbox]').click();
    at(all<HTMLInputElement>(fixture, 'input[type=checkbox]'), 1).click();
    typeInto(el(fixture, 'textarea'), '\\Process(*)\\Handle Count');
    typeInto(at(all<HTMLInputElement>(fixture, 'input.mono'), 0), '/out');
    expect(store.draft()).toMatchObject({ intervalMs: 1000, cpu: 'core', includeChildren: true, counters: '\\Process(*)\\Handle Count', dir: '/out', dotnet: true });
    expect(text(fixture)).toContain('.NET runtime counters (Framework)');

    buttonByText(fixture, 'Start').click();
    await settle(fixture);
    expect(el(fixture, '[data-testid=monitor-status]').textContent).toContain('Running');
    expect(el(fixture, '[data-testid=monitor-status]').textContent).toContain('/out');
    expect(TestBed.inject(LiveTail).enabled()).toBe(true);
    buttonByText(fixture, 'Stop').click();
    await settle(fixture);
    expect(el(fixture, '[data-testid=monitor-status]').textContent).toContain('Stopped');
    el<HTMLButtonElement>(fixture, 'button[aria-label="Refresh status"]').click();
    await settle(fixture);
  });

  it('offers counters on Windows even when already elevated', async () => {
    TestBed.configureTestingModule({ providers: [...provideToolsTesting(), charts.provider] });
    vi.spyOn(mockTools(), 'elevation').mockResolvedValue({ elevated: true, canRestartElevated: false, platform: 'windows' });
    await TestBed.inject(LifecycleService).init();
    expect(TestBed.inject(MonitorStore).countersAvailable()).toBe(true);
  });

  it('hides counters away from Windows', async () => {
    const { fixture } = await setup({ platform: 'macos' });
    expect(fixture.nativeElement.querySelector('textarea')).toBeNull();
  });

  it('suggests elevation when AccessDenied events appear', async () => {
    const { fixture, workspace, tools } = await setup();
    const s = workspace.summary()!;
    workspace.applySummary({ ...s, fields: [...s.fields, { name: ACCESS_DENIED_FIELD, count: 1, distinct: 1, numeric: false, derived: false }] });
    await settle(fixture);
    expect(el(fixture, '[data-testid=access-denied-note]').textContent).toContain('1 sample could not read');
    buttonByText(fixture, 'Restart as administrator').click();
    await settle(fixture);
    expect(tools.calls.some((c) => c.startsWith('restartElevated'))).toBe(true);
    workspace.applySummary({ ...s, fields: [...s.fields, { name: ACCESS_DENIED_FIELD, count: 2, distinct: 1, numeric: false, derived: false }] });
    await settle(fixture);
    expect(text(fixture)).toContain('2 samples');
  });

  it('asks for administrator rights by hand where a restart is not possible', async () => {
    const { fixture, workspace } = await setup({ platform: 'macos' });
    const s = workspace.summary()!;
    workspace.applySummary({ ...s, fields: [...s.fields, { name: ACCESS_DENIED_FIELD, count: 1, distinct: 1, numeric: false, derived: false }] });
    await settle(fixture);
    expect(text(fixture)).toContain('Run IMT as administrator to include them');
  });

  it('shows start errors', async () => {
    const { fixture, store } = await setup();
    store.setTarget(0, { value: 'a' });
    await store.start();
    await store.start();
    await settle(fixture);
    expect(text(fixture)).toContain('already running');
  });
});
