import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, settle, text, typeInto } from '../../../testing/helpers';
import { Platform } from '../../core/platform';
import { WorkspaceStore } from '../../core/workspace.store';
import { LifecycleService } from '../../lifecycle/lifecycle.service';
import { LiveTail } from '../../workspace/live-tail';
import { DialogService } from '../../ui/dialog.service';
import { mockTools, provideToolsTesting } from '../mock/test-support';
import { MockToolsOptions } from '../mock/mock-tools-backend';
import { PerfmonStore } from './perfmon.store';
import { PerfmonView } from './perfmon-view';

async function setup(options: Partial<MockToolsOptions> = {}, picker: string | null = '/x/IMT.xml') {
  TestBed.configureTestingModule({
    providers: [...provideToolsTesting(options), { provide: Platform, useValue: { pickExportPath: async () => picker, pickPaths: async () => [], onFilesDropped: async () => () => undefined } }],
  });
  const fixture = TestBed.createComponent(PerfmonView);
  await TestBed.inject(LifecycleService).init();
  await settle(fixture);
  return { fixture, store: TestBed.inject(PerfmonStore), tools: mockTools(), workspace: TestBed.inject(WorkspaceStore) };
}

describe('PerfmonStore', () => {
  it('loads presets once and selects CPU and memory by default', async () => {
    const { store } = await setup();
    expect(store.presets().map((p) => p.id)).toEqual(['cpu', 'memory', 'disk']);
    expect(store.draft().counters).toContain('\\Process({process})\\Private Bytes');
    expect(store.draft().counters).not.toContain('\\PhysicalDisk(_Total)\\Avg. Disk sec/Read');
    await store.loadPresets();
    expect(store.busy()).toBeNull();
  });

  it('keeps the counters the user already chose when presets load later', async () => {
    TestBed.configureTestingModule({ providers: provideToolsTesting() });
    const store = TestBed.inject(PerfmonStore);
    store.patchDraft({ counters: ['x'] });
    await store.loadPresets();
    expect(store.draft().counters).toEqual(['x']);
  });

  it('reports errors from the engine and can dismiss them', async () => {
    const { store } = await setup();
    store.patchDraft({ name: ' ' });
    await store.generate();
    expect(store.error()).toContain('name');
    store.dismissError();
    expect(store.error()).toBeNull();
  });

  it('generates XML, drops it when the draft changes, and saves it where the user chose', async () => {
    const { store } = await setup();
    await store.generate();
    expect(store.generated()?.xml).toContain('<DataCollectorSet>');
    store.patchDraft({ intervalSecs: 30 });
    expect(store.generated()).toBeNull();
    await store.generate();
    await store.saveXml();
    expect(store.generated()?.path).toBe('/x/IMT.xml');
    expect(store.notice()).toBe('Saved /x/IMT.xml');
  });

  it('never replaces an existing XML file without asking', async () => {
    const { store } = await setup();
    const dialogs = TestBed.inject(DialogService);
    await store.saveXml();
    store.patchDraft({ intervalSecs: 30 });
    // The same file again (the browser build's dialog does not ask): the app asks, and "Keep" saves nothing.
    let saving = store.saveXml();
    await vi.waitFor(() => expect(dialogs.stack()[0]?.options.title).toBe('Replace file?'));
    dialogs.stack()[0].ref.close(false);
    await saving;
    expect(store.generated()).toBeNull();
    expect(store.error()).toBeNull();
    saving = store.saveXml();
    await vi.waitFor(() => expect(dialogs.stack()).toHaveLength(1));
    dialogs.stack()[0].ref.close(true);
    await saving;
    expect(store.notice()).toBe('Saved /x/IMT.xml');
    expect(store.generated()?.xml).toContain('<DataCollectorSet>');
  });

  it('does nothing when the save dialog is cancelled', async () => {
    const { store } = await setup({}, null);
    await store.saveXml();
    expect(store.generated()).toBeNull();
  });

  it('installs and starts the collector, then controls it', async () => {
    const { store } = await setup();
    await store.installAndStart();
    expect(store.status()?.status).toBe('running');
    await store.control('stop');
    expect(store.status()?.status).toBe('stopped');
    await store.control('query');
    await store.control('delete');
    expect(store.status()).toBeNull();
    expect(store.notice()).toBe('Deleted IMT');
  });

  it('imports the output with live tail, or says there is nothing yet', async () => {
    const { store, tools, workspace } = await setup();
    const live = TestBed.inject(LiveTail);
    const start = vi.spyOn(live, 'start');
    await store.installAndStart();
    await store.importResults();
    expect(start).toHaveBeenCalledTimes(1);
    expect(live.enabled()).toBe(true);
    expect(store.notice()).toContain('with live tail');
    expect(workspace.summary()?.sources.length).toBeGreaterThan(0);
    store.setLiveTail(false);
    await store.importResults();
    expect(start).toHaveBeenCalledTimes(1);
    expect(store.notice()).toBe('Opened 1 file');
    const empty = vi.spyOn(tools, 'perfmonOutputs').mockResolvedValue([]);
    await store.importResults();
    expect(store.notice()).toContain('not written any files');
    empty.mockResolvedValue(['/a', '/b']);
    await store.importResults();
    expect(store.notice()).toContain('2 files');
  });

  it('does not start live tail when opening the results failed', async () => {
    const { store, tools, workspace } = await setup();
    await store.installAndStart();
    workspace.setError('boom');
    vi.spyOn(workspace, 'openPaths').mockResolvedValue();
    tools.calls.length = 0;
    await store.importResults();
    expect(tools.calls).toEqual([]);
  });

  it('manages sets only on Windows', async () => {
    const win = await setup();
    expect(win.store.canManage()).toBe(true);
    TestBed.resetTestingModule();
    const mac = await setup({ platform: 'macos' });
    expect(mac.store.canManage()).toBe(false);
  });

  it('manages sets on Windows even when the backend cannot restart elevated (already administrator)', async () => {
    TestBed.configureTestingModule({ providers: provideToolsTesting() });
    vi.spyOn(mockTools(), 'elevation').mockResolvedValue({ elevated: true, canRestartElevated: false, platform: 'windows' });
    await TestBed.inject(LifecycleService).init();
    expect(TestBed.inject(PerfmonStore).canManage()).toBe(true);
  });

  it('reports live tail that cannot start after importing', async () => {
    const { store } = await setup();
    await store.installAndStart();
    vi.spyOn(TestBed.inject(LiveTail), 'set').mockResolvedValue();
    await store.importResults();
    expect(store.error()).toBe('Live tail could not start');
  });
});

describe('PerfmonView', () => {
  it('walks the wizard: processes, presets, options, XML preview, copy', async () => {
    const { fixture, store } = await setup();
    const input = el<HTMLInputElement>(fixture, 'input[aria-label="Process name"]');
    typeInto(input, 'OrderService.exe');
    buttonByText(fixture, 'Add process').click();
    await settle(fixture);
    expect(all(fixture, '.chip').map((c) => c.textContent?.trim())).toEqual(['OrderService']);
    expect(input.value).toBe('');
    el<HTMLButtonElement>(fixture, 'button[aria-label="Remove process OrderService"]').click();
    await settle(fixture);
    expect(text(fixture)).toContain('All processes');
    typeInto(input, 'svc');
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }));
    await settle(fixture);
    expect(store.draft().processes).toEqual(['svc']);

    // Presets and counters with guidance.
    expect(text(fixture)).not.toContain('What to look for:');
    el<HTMLButtonElement>(fixture, 'button[aria-label="Show counters of CPU"]').click();
    await settle(fixture);
    expect(text(fixture)).toContain('What to look for:');
    const disk = el<HTMLInputElement>(fixture, 'input[aria-label="Preset Disk"]');
    disk.click();
    await settle(fixture);
    expect(store.draft().counters).toContain('\\PhysicalDisk(_Total)\\Avg. Disk sec/Read');
    const counter = all<HTMLInputElement>(fixture, '.counters input')[0];
    counter.click();
    await settle(fixture);
    expect(store.draft().counters).not.toContain('\\Process({process})\\% Processor Time');
    expect(el<HTMLInputElement>(fixture, 'input[aria-label="Preset CPU"]').indeterminate).toBe(true);
    el<HTMLButtonElement>(fixture, 'button[aria-label="Hide counters of CPU"]').click();
    await settle(fixture);
    expect(all(fixture, '.counters')).toHaveLength(0);

    // Options.
    typeInto(el(fixture, 'input[aria-label="Collector name"]'), 'Repro');
    typeInto(all<HTMLInputElement>(fixture, 'input[type=number]')[0], '30');
    const sel = el<HTMLSelectElement>(fixture, 'select');
    typeInto(sel, 'blg', 'change');
    await settle(fixture);
    typeInto(all<HTMLInputElement>(fixture, 'input[type=number]')[1], '256');
    typeInto(all<HTMLInputElement>(fixture, 'input[type=number]')[2], '10');
    typeInto(el(fixture, 'input.mono'), 'D:\\Perf\\');
    expect(store.draft()).toMatchObject({ name: 'Repro', intervalSecs: 30, format: 'blg', maxSizeMb: 256, durationMinutes: 10, outputDir: 'D:\\Perf\\' });

    buttonByText(fixture, 'Generate XML').click();
    await settle(fixture);
    const xml = el(fixture, 'pre.xml').textContent ?? '';
    expect(xml).toContain('<Name>Repro</Name>');
    expect(xml).toContain('Process(svc)');
    expect(all(fixture, '.cmds code')).toHaveLength(4);

    const write = vi.fn(async () => undefined);
    Object.defineProperty(navigator, 'clipboard', { value: { writeText: write }, configurable: true });
    buttonByText(fixture, 'Copy XML').click();
    await settle(fixture);
    expect(write).toHaveBeenCalledWith(xml);
    expect(text(fixture)).toContain('Copied');
    el<HTMLButtonElement>(fixture, 'button[aria-label^="Copy command logman import"]').click();
    await settle(fixture);
    expect(write).toHaveBeenCalledTimes(2);
    buttonByText(fixture, 'Save as…').click();
    await settle(fixture);
    expect(store.notice()).toBe('Saved /x/IMT.xml');
  });

  it('shows problems and disables generating until fixed', async () => {
    const { fixture, store } = await setup();
    store.patchDraft({ intervalSecs: 0 });
    await settle(fixture);
    expect(text(fixture)).toContain('The interval must be between');
    expect(buttonByText(fixture, 'Generate XML').disabled).toBe(true);
  });

  it('installs, shows the status card and drives start/stop/refresh/delete/import', async () => {
    const { fixture, store } = await setup();
    buttonByText(fixture, /Install/).click();
    await settle(fixture);
    expect(el(fixture, '[data-testid=collector-state]').textContent).toBe('running');
    buttonByText(fixture, 'Stop').click();
    await settle(fixture);
    expect(el(fixture, '[data-testid=collector-state]').textContent).toBe('stopped');
    buttonByText(fixture, 'Start').click();
    await settle(fixture);
    buttonByText(fixture, /Refresh/).click();
    await settle(fixture);
    el<HTMLInputElement>(fixture, '.card input[type=checkbox]').click();
    expect(store.liveTail()).toBe(false);
    buttonByText(fixture, 'Import results').click();
    await settle(fixture);
    expect(text(fixture)).toContain('Opened 1 file');
    buttonByText(fixture, 'Delete').click();
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('[data-testid=collector-status]')).toBeNull();
  });

  it('offers a restart as administrator when access is denied', async () => {
    const { fixture, tools } = await setup({ denied: true });
    buttonByText(fixture, /Install/).click();
    await settle(fixture);
    expect(text(fixture)).toContain('Access is denied');
    buttonByText(fixture, 'Restart as administrator').click();
    await settle(fixture);
    expect(tools.calls.some((c) => c.startsWith('restartElevated'))).toBe(true);
  });

  it('explains what is possible away from Windows', async () => {
    const { fixture } = await setup({ platform: 'macos' });
    expect(text(fixture)).toContain('only be created on Windows');
    expect(fixture.nativeElement.textContent).not.toContain('Install & start');
    buttonByText(fixture, 'Generate XML').click();
    await settle(fixture);
    expect(text(fixture)).toContain('Copy XML');
  });
});
