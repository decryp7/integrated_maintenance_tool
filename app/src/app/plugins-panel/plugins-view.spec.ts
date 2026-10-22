import { TestBed } from '@angular/core/testing';
import { all, settle, text } from '../../testing/helpers';
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
    expect(all(fixture, '.plugin strong').map((e) => e.textContent)).toEqual(['Windows registry importer', 'Windows event logs', 'Web server logs', 'OpenTelemetry']);
    expect(all(fixture, '.plugin .badge').map((e) => e.textContent)).toContain('importer:registry');
    expect(text(fixture)).toContain('v0.1.0');
    expect(all(fixture, '.builtin .badge').map((e) => e.textContent)).toEqual(['json', 'text', 'csv', 'csv', 'clef']);
    expect(text(fixture)).toContain('About plugins');
    // Third-party packages have their own section.
    expect(text(fixture)).toContain('Installed from packages');
    expect(all(fixture, 'debrief-installed-packages .package strong').map((e) => e.textContent)).toEqual(['SAP log importer']);
  });

  it('has Installed and Health tabs in one view (I4), opened on a chosen one and moved with the arrow keys', async () => {
    TestBed.configureTestingModule({ providers: provideToolsTesting() });
    const fixture = TestBed.createComponent(PluginsView);
    await settle(fixture);
    const tabs = () => all<HTMLButtonElement>(fixture, '[role=tab]');
    expect(tabs().map((t) => t.textContent?.trim())).toEqual(['Installed', 'Health']);
    expect(tabs()[0]?.getAttribute('aria-selected')).toBe('true');
    expect(fixture.nativeElement.querySelector('debrief-plugin-health-view')).toBeNull();
    tabs()[0]!.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true }));
    await settle(fixture);
    expect(TestBed.inject(PluginsTabs).tab()).toBe('health');
    expect(fixture.nativeElement.querySelector('debrief-plugin-health-view')).not.toBeNull();
    expect(fixture.debugElement.query((d) => d.componentInstance instanceof PluginHealthView)).not.toBeNull();
    tabs()[1]!.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', bubbles: true, cancelable: true }));
    tabs()[1]!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
    await settle(fixture);
    expect(TestBed.inject(PluginsTabs).tab()).toBe('installed');
    TestBed.inject(PluginsTabs).show('health');
    await settle(fixture);
    expect(tabs()[1]?.getAttribute('aria-selected')).toBe('true');
  });

  it('says so when no plugins are installed', async () => {
    TestBed.configureTestingModule({ providers: provideToolsTesting() });
    const fixture = TestBed.createComponent(PluginsView);
    const store = TestBed.inject(WorkspaceStore);
    await settle(fixture);
    store.applySummary({ ...store.summary()!, plugins: [], exporters: [] });
    await settle(fixture);
    expect(text(fixture)).toContain('No plugins are installed');
    expect(text(fixture)).toContain('none');
  });
});
