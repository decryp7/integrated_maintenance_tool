import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, pluginInfo, settle, text } from '../../../testing/helpers';
import { WorkspaceStore } from '../../core/workspace.store';
import { PluginInfo } from '../../core/api.types';
import { mockTools, provideToolsTesting } from '../mock/test-support';
import { PluginHealthView } from './plugin-health-view';
import { eventName, PluginHealthStore } from './plugin-health.store';
import { ImtBackend } from '../../core/backend';
import { at } from '../../../testing/must';

const PLUGINS: PluginInfo[] = [
  pluginInfo({ id: 'a', name: 'Alpha', version: '1.0', contributes: ['importer'], status: 'active', failures: 0, lastError: null }),
  pluginInfo({ id: 'b', name: 'Beta', version: '2.0', contributes: ['hook'], status: 'quarantined', failures: 3, lastError: 'panic at x.rs:1' }),
  pluginInfo({ id: 'c', name: 'Gamma', version: '3.0', contributes: [], status: 'disabled', failures: 0, lastError: null }),
  pluginInfo({ id: 'd', name: 'Delta', version: '4.0', contributes: [], status: 'active', failures: 1, lastError: null }),
];

async function setup(plugins: PluginInfo[] = PLUGINS) {
  TestBed.configureTestingModule({ providers: provideToolsTesting() });
  const fixture = TestBed.createComponent(PluginHealthView);
  const workspace = TestBed.inject(WorkspaceStore);
  await settle(fixture);
  workspace.applySummary({ ...workspace.summary()!, plugins });
  await settle(fixture);
  return { fixture, store: TestBed.inject(PluginHealthStore), tools: mockTools() };
}

describe('PluginHealthView', () => {
  it('lists status badges, failures and the last error', async () => {
    const { fixture } = await setup();
    expect(all(fixture, '[data-testid=plugin-status]').map((e) => e.textContent)).toEqual(['active', 'quarantined', 'disabled', 'active']);
    const beta = at(all(fixture, '.plugin'), 1).textContent ?? '';
    expect(beta).toContain('3 failures');
    expect(beta).toContain('panic at x.rs:1');
    expect(at(all(fixture, '.plugin'), 0).textContent).not.toContain('failure');
    expect(all(fixture, '.plugin')[3]?.textContent).toContain('1 failure');
    expect(at(all(fixture, '.plugin'), 3).textContent).not.toContain('1 failures');
  });

  it('resets a quarantined plugin', async () => {
    const { fixture, tools, store } = await setup();
    expect(all(fixture, '.plugin')[1]?.textContent).toContain('Reset');
    expect(at(all(fixture, '.plugin'), 0).textContent).not.toContain('Reset');
    buttonByText(fixture, 'Reset').click();
    await settle(fixture);
    expect(tools.calls).toContain('pluginReset:b');
    expect(all(fixture, '[data-testid=plugin-status]')[1]?.textContent).toBe('active');
    expect(store.plugins()[1]?.failures).toBe(0);
    expect(fixture.nativeElement.textContent).not.toContain('3 failures');
  });

  it('disables and enables, then restarts to apply', async () => {
    const { fixture, tools } = await setup();
    expect(fixture.nativeElement.textContent).not.toContain('Restart to apply');
    at(all<HTMLButtonElement>(fixture, '.plugin'), 0).querySelector<HTMLButtonElement>('.actions .btn')!.click();
    await settle(fixture);
    expect(tools.calls).toContain('pluginSetEnabled:a:false');
    expect(text(fixture)).toContain('will be disabled');
    const gamma = at(all(fixture, '.plugin'), 2).querySelector<HTMLButtonElement>('.actions .btn')!;
    expect(gamma.textContent).toBe('Enable');
    gamma.click();
    await settle(fixture);
    expect(text(fixture)).toContain('will be enabled');
    buttonByText(fixture, 'Restart to apply').click();
    await settle(fixture);
    expect(tools.calls.some((c) => c.startsWith('restartApp:false'))).toBe(true);
  });

  it('shows failures and an empty state', async () => {
    const { fixture, store, tools } = await setup([]);
    expect(text(fixture)).toContain('No plugins are loaded');
    vi.spyOn(tools, 'pluginReset').mockRejectedValue(new Error('no config folder'));
    await store.reset('x');
    await settle(fixture);
    expect(text(fixture)).toContain('no config folder');
    el<HTMLButtonElement>(fixture, '.err .btn.ghost').click();
    await settle(fixture);
    expect(store.error()).toBeNull();
  });
});

describe('PluginHealthView (plugin v2)', () => {
  const V2: PluginInfo[] = [
    pluginInfo({
      id: 'net.decryptology.windows',
      name: 'Windows',
      provides: [{ id: 'net.decryptology.windows.events', version: '1.0.0' }],
      dependents: ['net.decryptology.web'],
      hung: true,
    }),
    pluginInfo({
      id: 'net.decryptology.web',
      name: 'Web',
      requires: [
        { id: 'net.decryptology.windows', req: '^0.2', optional: false },
        { id: 'net.decryptology.perfmon', req: '^1.0', optional: true },
      ],
      warnings: ['optional dependency net.decryptology.perfmon is not installed'],
    }),
    pluginInfo({ id: 'net.decryptology.otel', name: 'Otel', status: 'inactive', inactiveReason: 'needs net.decryptology.collector ^1.0 (not installed)' }),
  ];

  it('shows dependencies, services, dependents, warnings, hangs and why a plugin is inactive', async () => {
    const { fixture } = await setup(V2);
    const plugins = all(fixture, '.plugin');
    const win = at(plugins, 0);
    const web = at(plugins, 1);
    const otel = at(plugins, 2);
    expect(win.querySelector('[data-testid=plugin-hung]')!.textContent).toContain('not responding');
    expect(win.textContent).toContain('Provides');
    expect(win.textContent).toContain('net.decryptology.windows.events 1.0.0');
    expect(win.textContent).toContain('Needed by');
    expect(web.textContent).toContain('net.decryptology.windows ^0.2');
    expect(web.textContent).toContain('net.decryptology.perfmon ^1.0 (optional)');
    expect(web.querySelector('.warnings')!.textContent).toContain('perfmon is not installed');
    expect(otel.getAttribute('data-status')).toBe('inactive');
    expect(otel.querySelector('[data-testid=inactive-reason]')!.textContent).toContain('needs net.decryptology.collector');
  });

  it('shows event-bus counters per subscriber and refreshes them', async () => {
    const { fixture, store } = await setup(V2);
    await settle(fixture);
    const table = at(all(fixture, '.plugin'), 0).querySelector('table.events')!;
    expect(table.getAttribute('aria-label')).toBe('Events delivered to Windows');
    expect(table.textContent).toContain('RecordsAppended');
    const web = at(all(fixture, '.plugin'), 1).querySelector('table.events tbody tr')!;
    expect(web.classList).toContain('bad');
    const backend = TestBed.inject(ImtBackend);
    vi.spyOn(backend, 'pluginEvents').mockRejectedValueOnce('bus gone');
    buttonByText(fixture, 'Refresh event counters').click();
    await settle(fixture);
    expect(store.eventsError()).toBe('bus gone');
    expect(text(fixture)).toContain('Event counters unavailable');
    await store.loadEvents();
    expect(store.eventsError()).toBeNull();
    expect(eventName('a::b::C')).toBe('C');
    expect(eventName('')).toBe('');
  });
});
