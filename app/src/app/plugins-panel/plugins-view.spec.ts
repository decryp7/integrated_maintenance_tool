import { TestBed } from '@angular/core/testing';
import { all, el, settle, text } from '../../testing/helpers';
import { provideToolsTesting } from '../tools/mock/test-support';
import { WorkspaceStore } from '../core/workspace.store';
import { contributionLabel, PluginsView } from './plugins-view';
import { PluginsTabs } from './plugins-tabs';
import { PluginHealthView } from '../tools/health/plugin-health-view';

describe('PluginsView', () => {
  it('labels transform contributions by kind', () => {
    expect(contributionLabel('transform:redact')).toBe('transform');
    expect(contributionLabel('importer')).toBe('importer');
  });

  it('lists plugins with contribution badges, built-ins and help', async () => {
    TestBed.configureTestingModule({ providers: provideToolsTesting() });
    const fixture = TestBed.createComponent(PluginsView);
    await settle(fixture);
    expect(all(fixture, 'tr.plugin .name').map((e) => e.textContent)).toEqual(['Windows registry importer', 'Windows event logs', 'Web server logs', 'OpenTelemetry']);
    expect(all(fixture, 'tr.plugin .ver').map((e) => e.textContent)).toContain('0.1.0');
    // One table, no per-row forms; the capability identifiers are in the side panel only.
    expect(all(fixture, 'table.plugins')).toHaveLength(1);
    expect(all(fixture, 'tr.plugin input, tr.plugin .badge')).toHaveLength(0);
    expect(text(fixture)).not.toContain('importer:registry');
    expect(all(fixture, '.builtin .badge').map((e) => e.textContent)).toEqual(['json', 'text', 'csv', 'csv', 'clef']);
    expect(text(fixture)).toContain('About plugins');
    // Third-party packages have their own section.
    expect(text(fixture)).toContain('Installed from packages');
    expect(all(fixture, 'imt-installed-packages .package strong').map((e) => e.textContent)).toEqual(['SAP log importer']);
  });

  it('has Installed and Health tabs in one view (I4), opened on a chosen one and moved with the arrow keys', async () => {
    TestBed.configureTestingModule({ providers: provideToolsTesting() });
    const fixture = TestBed.createComponent(PluginsView);
    await settle(fixture);
    const tabs = () => all<HTMLButtonElement>(fixture, '[role=tab]');
    expect(tabs().map((t) => t.textContent?.trim())).toEqual(['Installed', 'Health']);
    expect(tabs()[0]?.getAttribute('aria-selected')).toBe('true');
    expect(fixture.nativeElement.querySelector('imt-plugin-health-view')).toBeNull();
    tabs()[0]!.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true }));
    await settle(fixture);
    expect(TestBed.inject(PluginsTabs).tab()).toBe('health');
    expect(fixture.nativeElement.querySelector('imt-plugin-health-view')).not.toBeNull();
    expect(fixture.debugElement.query((d) => d.componentInstance instanceof PluginHealthView)).not.toBeNull();
    tabs()[1]!.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', bubbles: true, cancelable: true }));
    tabs()[1]!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
    await settle(fixture);
    expect(TestBed.inject(PluginsTabs).tab()).toBe('installed');
    TestBed.inject(PluginsTabs).show('health');
    await settle(fixture);
    expect(tabs()[1]?.getAttribute('aria-selected')).toBe('true');
  });

  it('opens a side panel on a row click with the capabilities, the memory cap and the permissions, and closes it again', async () => {
    TestBed.configureTestingModule({ providers: provideToolsTesting() });
    const fixture = TestBed.createComponent(PluginsView);
    await settle(fixture);
    const rows = () => all<HTMLElement>(fixture, 'tr.plugin');
    const windows = rows().find((r) => r.textContent?.includes('Windows event logs'))!;
    expect(windows.querySelector('[data-testid=plugin-group]')).not.toBeNull();
    expect(Array.from(windows.querySelectorAll('[data-testid=plugin-group]')).map((g) => g.getAttribute('aria-label'))).toContain('1 importer');
    windows.click();
    await settle(fixture);
    const detail = el(fixture, '[data-testid=plugin-detail]');
    expect(detail.textContent).toContain('importer:evtx');
    expect(detail.textContent).toContain('net.decryptology.windows');
    expect(detail.querySelector('[data-testid=plugin-memory-input]')).not.toBeNull();
    expect(detail.querySelector('[data-testid=plugin-permissions]')).not.toBeNull();
    expect(windows.getAttribute('aria-selected')).toBe('true');
    el<HTMLButtonElement>(fixture, '[data-testid=plugin-detail-close]').click();
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('[data-testid=plugin-detail]')).toBeNull();
  });

  it('disables a plugin from its row menu', async () => {
    TestBed.configureTestingModule({ providers: provideToolsTesting() });
    const fixture = TestBed.createComponent(PluginsView);
    await settle(fixture);
    all<HTMLButtonElement>(fixture, '[data-testid=plugin-more]')[0]!.click();
    await settle(fixture);
    const disable = document.body.querySelector<HTMLButtonElement>('[data-testid=plugin-disable]')!;
    expect(disable).not.toBeNull();
    disable.click();
    await settle(fixture);
    expect(all(fixture, '[data-testid=plugin-status]')[0]!.textContent).toContain('Stops after restart');
  });

  it('says so when no plugins are installed', async () => {
    TestBed.configureTestingModule({ providers: provideToolsTesting() });
    const fixture = TestBed.createComponent(PluginsView);
    const store = TestBed.inject(WorkspaceStore);
    await settle(fixture);
    store.applySummary({ ...store.summary()!, plugins: [], exporters: [] });
    await settle(fixture);
    expect(text(fixture)).toContain('No plugins installed');
    expect(text(fixture)).toContain('none');
  });
});
