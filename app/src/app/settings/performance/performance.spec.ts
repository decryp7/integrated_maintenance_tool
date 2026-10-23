import { Injector } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, text } from '../../../testing/helpers';
import { AiBackend } from '../../ai/ai-backend';
import { LifecycleBackend } from '../../core/tools-backend';
import { COMMAND_CONTEXT } from '../../extensions/command-registry';
import { NoticeActions, actionLabel } from '../../notifications/notice-actions';
import { Notice } from '../../notifications/notifications.types';
import { PluginMemory } from '../../plugins-panel/plugin-memory';
import { EngineSettingsStore } from '../engine-settings.store';
import { MockSettingsBackend } from '../mock-settings-backend';
import { SettingsBackend } from '../settings-backend';
import { SettingsDialogService } from '../settings-dialog';
import { ServiceHealthStore } from '../../health/service-health.store';
import { capField, raiseMemoryCap, retryProcess } from './memory-actions';
import { MockPerformanceBackend } from './mock-performance';
import { PerformanceBackend, TauriPerformanceBackend } from './performance-backend';
import { PerformanceExtras } from './performance-extras';
import { PluginMemoryStore } from './plugin-memory.store';

function setup() {
  const perf = new MockPerformanceBackend();
  const settings = new MockSettingsBackend();
  const showView = vi.fn();
  const open = vi.fn();
  const pluginReset = vi.fn(async () => undefined);
  const aiReset = vi.fn(async () => ({}));
  TestBed.configureTestingModule({
    providers: [
      provideTinyBackend(),
      { provide: PerformanceBackend, useValue: perf },
      { provide: SettingsBackend, useValue: settings },
      { provide: SettingsDialogService, useValue: { open } },
      { provide: COMMAND_CONTEXT, useValue: { showView } },
      { provide: LifecycleBackend, useValue: { pluginReset } },
      { provide: AiBackend, useValue: { reset: aiReset } },
    ],
  });
  return { perf, settings, showView, open, pluginReset, aiReset };
}

describe('Settings ▸ Performance', () => {
  it('Suggest settings shows the before/after table and applies it through the section', async () => {
    const { settings } = setup();
    const store = TestBed.inject(EngineSettingsStore);
    await store.load();
    const set = vi.spyOn(settings, 'set');
    const fixture = TestBed.createComponent(PerformanceExtras);
    await settle(fixture);
    el(fixture, '[data-testid=perf-suggest]').click();
    await settle(fixture);
    expect(text(fixture)).toContain('16 GB of memory');
    const rows = all(fixture, 'tbody tr').map((r) => [...r.children].map((c) => c.textContent?.trim()).join(' | '));
    expect(rows[0]).toBe('Database worker memory cap | 2048 MB | 3072 MB');
    expect(rows).toHaveLength(4);
    el(fixture, '[data-testid=perf-apply]').click();
    await settle(fixture);
    expect(set).toHaveBeenCalledWith('performance', expect.objectContaining({ storeCapMb: 3072, queryThreads: 3 }));
    expect(all(fixture, '[data-testid=perf-proposal]')).toHaveLength(0);
  });

  it('presets propose their values and say when nothing changes', async () => {
    setup();
    const fixture = TestBed.createComponent(PerformanceExtras);
    await settle(fixture);
    el(fixture, '[data-testid=perf-preset-low]').click();
    await settle(fixture);
    expect(text(fixture)).toContain('Give freed memory back at once');
    expect(text(fixture)).toMatch(/Off\s*On/);
    el(fixture, '[data-testid=perf-preset-standard]').click();
    await settle(fixture);
    expect(el(fixture, '[data-testid=perf-nothing]')).toBeTruthy();
  });

  it('shows the engine refusing a proposal', async () => {
    const { perf } = setup();
    vi.spyOn(perf, 'suggest').mockRejectedValue(new Error('no memory information'));
    const fixture = TestBed.createComponent(PerformanceExtras);
    await settle(fixture);
    el(fixture, '[data-testid=perf-suggest]').click();
    await settle(fixture);
    expect(el(fixture, '[role=alert]').textContent).toContain('no memory information');
  });

  it('"Raise the cap…" opens the worker caps in Settings with the suggested value, unapplied', async () => {
    const { open, showView } = setup();
    const injector = TestBed.inject(Injector);
    const store = TestBed.inject(EngineSettingsStore);
    await raiseMemoryCap(injector, { kind: 'storeWorker' }, 3328);
    expect(open).toHaveBeenCalledWith('performance');
    expect(store.valueOf('performance', 'storeCapMb')).toBe(3328);
    expect(store.dirty()).toBe(true);
    expect(capField({ kind: 'ai' })).toBe('aiWorkerMemoryMb');
    // A plugin's cap is its own, in the Plugins view.
    await raiseMemoryCap(injector, { kind: 'plugin', pluginId: 'net.decryptology.windows' }, 1536);
    expect(showView).toHaveBeenCalledWith('plugins');
    expect(TestBed.inject(PluginMemoryStore).requested()).toEqual({ pluginId: 'net.decryptology.windows', mb: 1536 });
  });

  it('"Retry" lets the stopped process start again', async () => {
    const { pluginReset, aiReset } = setup();
    const injector = TestBed.inject(Injector);
    const health = TestBed.inject(ServiceHealthStore);
    const retry = vi.spyOn(health, 'repair').mockResolvedValue();
    // A store that works needs no repair: it starts with its next use.
    health.apply({ revision: 4, services: [] });
    await retryProcess(injector, { kind: 'storeWorker' });
    expect(retry).not.toHaveBeenCalled();
    health.apply({ revision: 5, services: [{ service: 'store', label: 'the log store', state: 'down', cause: { message: 'x', hint: null, detail: null }, repair: 'restart' }] });
    await retryProcess(injector, { kind: 'storeWorker' });
    await retryProcess(injector, { kind: 'ai' });
    await retryProcess(injector, { kind: 'plugin', pluginId: 'p' });
    expect(retry).toHaveBeenCalledWith('store');
    expect(aiReset).toHaveBeenCalled();
    expect(pluginReset).toHaveBeenCalledWith('p');
  });

  it('memory messages run their actions from the notification centre', async () => {
    const { pluginReset } = setup();
    const notice: Notice = {
      id: -1,
      source: 'performance',
      severity: 'warning',
      title: "Plugin 'X' was stopped",
      body: '',
      at: 0,
      firstAt: 0,
      count: 1,
      read: false,
      primary: { type: 'raiseMemoryCap', process: { kind: 'plugin', pluginId: 'x' }, suggestedMb: 768 },
      secondary: { type: 'retryProcess', process: { kind: 'plugin', pluginId: 'x' } },
      tertiary: null,
      link: { window: null, jobId: null, recordId: null, pluginId: 'x', path: null },
      cause: 'fixture',
      scope: 'app',
    };
    expect(actionLabel(notice.primary!)).toBe('Raise the cap…');
    expect(actionLabel(notice.secondary!)).toBe('Retry');
    const actions = TestBed.inject(NoticeActions);
    expect(await actions.run(notice, notice.secondary!)).toBeNull();
    expect(pluginReset).toHaveBeenCalledWith('x');
    expect(await actions.run(notice, notice.primary!)).toBeNull();
    expect(TestBed.inject(PluginMemoryStore).requested()?.mb).toBe(768);
  });

  it("a plugin's memory shows its peak and cap, and its own cap is set, refused or removed", async () => {
    setup();
    const store = TestBed.inject(PluginMemoryStore);
    await store.load();
    const fixture = TestBed.createComponent(PluginMemory);
    fixture.componentRef.setInput('pluginId', 'net.decryptology.windows');
    fixture.componentRef.setInput('pluginName', 'Windows diagnostics');
    await settle(fixture);
    expect(text(fixture)).toContain('Peak 420 MB of a 1024 MB cap');
    const input = el<HTMLInputElement>(fixture, '[data-testid=plugin-memory-input]');
    input.value = '10';
    input.dispatchEvent(new Event('input'));
    await settle(fixture);
    input.dispatchEvent(new Event('blur'));
    await settle(fixture);
    expect(el(fixture, '[role=alert]').textContent).toContain('64 to 4096 MB');
    input.value = '768';
    input.dispatchEvent(new Event('input'));
    await settle(fixture);
    input.dispatchEvent(new Event('blur'));
    await settle(fixture);
    expect(store.plugin('net.decryptology.windows')?.overrideMb).toBe(768);
    expect(text(fixture)).toContain('of a 768 MB cap');
    // Leaving the field again without a change sends nothing.
    input.dispatchEvent(new Event('blur'));
    buttonByText(fixture, /Use the default/).click();
    await settle(fixture);
    expect(store.plugin('net.decryptology.windows')?.overrideMb).toBeNull();
  });

  it('a suggested plugin cap is filled in, not applied', async () => {
    setup();
    const store = TestBed.inject(PluginMemoryStore);
    await store.load();
    store.request('net.decryptology.registry', 1024);
    const fixture = TestBed.createComponent(PluginMemory);
    fixture.componentRef.setInput('pluginId', 'net.decryptology.registry');
    await settle(fixture);
    expect(el<HTMLInputElement>(fixture, '[data-testid=plugin-memory-input]').value).toBe('1024');
    expect(store.plugin('net.decryptology.registry')?.overrideMb).toBe(512);
  });

  it('the Tauri backend sends the commands of the contract', async () => {
    const call = vi.fn(async () => undefined as never);
    const b = new TauriPerformanceBackend(call);
    await b.suggest();
    await b.preset('lowMemory');
    await b.memory();
    await b.setPluginMemory('p', null);
    expect(call.mock.calls).toEqual([
      ['performance_suggest'],
      ['performance_preset', { preset: 'lowMemory' }],
      ['performance_memory'],
      ['performance_set_plugin_memory', { pluginId: 'p', mb: null }],
    ]);
  });
});
