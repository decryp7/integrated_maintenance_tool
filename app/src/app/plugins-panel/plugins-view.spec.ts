import { TestBed } from '@angular/core/testing';
import { all, provideTinyBackend, settle, text } from '../../testing/helpers';
import { WorkspaceStore } from '../core/workspace.store';
import { contributionLabel, PluginsView } from './plugins-view';

describe('PluginsView', () => {
  it('labels transform contributions by kind', () => {
    expect(contributionLabel('transform:redact')).toBe('transform');
    expect(contributionLabel('importer')).toBe('importer');
  });

  it('lists plugins with contribution badges, built-ins and help', async () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const fixture = TestBed.createComponent(PluginsView);
    await settle(fixture);
    expect(all(fixture, '.plugin strong').map((e) => e.textContent)).toEqual(['Windows registry importer', 'CSV and CLEF exporters']);
    expect(all(fixture, '.plugin .badge').map((e) => e.textContent)).toEqual(['importer', 'exporter']);
    expect(text(fixture)).toContain('v0.1.0');
    expect(all(fixture, '.builtin .badge').map((e) => e.textContent)).toEqual(['json', 'text', 'csv', 'csv', 'clef']);
    expect(text(fixture)).toContain('About plugins');
  });

  it('says so when no plugins are installed', async () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const fixture = TestBed.createComponent(PluginsView);
    const store = TestBed.inject(WorkspaceStore);
    await settle(fixture);
    store.applySummary({ ...store.summary()!, plugins: [], exporters: [] });
    await settle(fixture);
    expect(text(fixture)).toContain('No plugins are installed');
    expect(text(fixture)).toContain('none');
  });
});
