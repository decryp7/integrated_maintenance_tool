import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, settle, text } from '../../../testing/helpers';
import { WorkspaceStore } from '../../core/workspace.store';
import { PluginInfo } from '../../core/api.types';
import { mockTools, provideToolsTesting } from '../mock/test-support';
import { PluginHealthView } from './plugin-health-view';
import { PluginHealthStore } from './plugin-health.store';

const PLUGINS: PluginInfo[] = [
  { id: 'a', name: 'Alpha', version: '1.0', contributes: ['importer'], status: 'active', failures: 0, lastError: null },
  { id: 'b', name: 'Beta', version: '2.0', contributes: ['hook'], status: 'quarantined', failures: 3, lastError: 'panic at x.rs:1' },
  { id: 'c', name: 'Gamma', version: '3.0', contributes: [], status: 'disabled', failures: 0, lastError: null },
  { id: 'd', name: 'Delta', version: '4.0', contributes: [], status: 'active', failures: 1, lastError: null },
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
    const beta = all(fixture, '.plugin')[1].textContent ?? '';
    expect(beta).toContain('3 failures');
    expect(beta).toContain('panic at x.rs:1');
    expect(all(fixture, '.plugin')[0].textContent).not.toContain('failure');
    expect(all(fixture, '.plugin')[3].textContent).toContain('1 failure');
    expect(all(fixture, '.plugin')[3].textContent).not.toContain('1 failures');
  });

  it('resets a quarantined plugin', async () => {
    const { fixture, tools, store } = await setup();
    expect(all(fixture, '.plugin')[1].textContent).toContain('Reset');
    expect(all(fixture, '.plugin')[0].textContent).not.toContain('Reset');
    buttonByText(fixture, 'Reset').click();
    await settle(fixture);
    expect(tools.calls).toContain('pluginReset:b');
    expect(all(fixture, '[data-testid=plugin-status]')[1].textContent).toBe('active');
    expect(store.plugins()[1].failures).toBe(0);
    expect(fixture.nativeElement.textContent).not.toContain('3 failures');
  });

  it('disables and enables, then restarts to apply', async () => {
    const { fixture, tools } = await setup();
    expect(fixture.nativeElement.textContent).not.toContain('Restart to apply');
    all<HTMLButtonElement>(fixture, '.plugin')[0].querySelector<HTMLButtonElement>('.actions .btn')!.click();
    await settle(fixture);
    expect(tools.calls).toContain('pluginSetEnabled:a:false');
    expect(text(fixture)).toContain('will be disabled');
    const gamma = all(fixture, '.plugin')[2].querySelector<HTMLButtonElement>('.actions .btn')!;
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
