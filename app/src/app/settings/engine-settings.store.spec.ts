import { TestBed } from '@angular/core/testing';
import { LifecycleService } from '../lifecycle/lifecycle.service';
import { EngineSettingsStore } from './engine-settings.store';
import { MockSettingsBackend } from './mock-settings-backend';
import { SettingsBackend } from './settings-backend';
import { GENERAL_ID } from './settings-model';

function setup(backend = new MockSettingsBackend()) {
  const restart = vi.fn(async () => undefined);
  TestBed.configureTestingModule({
    providers: [
      { provide: SettingsBackend, useValue: backend },
      { provide: LifecycleService, useValue: { restartKeepingSession: restart } },
    ],
  });
  return { store: TestBed.inject(EngineSettingsStore), backend, restart };
}

describe('EngineSettingsStore', () => {
  it('loads sections and keeps edits as drafts until applied', async () => {
    const { store } = setup();
    expect(store.selected()).toBe(GENERAL_ID);
    await store.load();
    expect(store.sections().map((s) => s.id)).toContain('diagnostics');
    store.select('workspace');
    expect(store.current()?.title).toBe('Live tail');
    store.edit('workspace', 'liveTailDebounce', 900);
    expect(store.dirty()).toBe(true);
    expect(store.dirtySections()).toEqual(['workspace']);
    expect(store.valueOf('workspace', 'liveTailDebounce')).toBe(900);
    // Editing back to the applied value is not a change.
    store.edit('workspace', 'liveTailDebounce', 750);
    expect(store.dirty()).toBe(false);
    store.edit('workspace', 'liveTailDebounce', 900);
    expect(await store.apply()).toBe(true);
    expect(store.dirty()).toBe(false);
    expect(store.valueOf('workspace', 'liveTailDebounce')).toBe(900);
    expect(store.notice()).toBe('Settings applied.');
    expect(store.restartPending()).toBe(false);
    // Reloading keeps unapplied edits.
    store.edit('workspace', 'liveTailDebounce', 1000);
    await store.load();
    expect(store.valueOf('workspace', 'liveTailDebounce')).toBe(1000);
    // A plain edit applies by itself, so it never blocks closing; only an edit the engine refused asks first.
    expect(store.canClose()).toBe(true);
    store.edit('workspace', 'liveTailDebounce', 5);
    expect(await store.apply()).toBe(false);
    expect(store.rejectedSections()).toEqual(['workspace']);
    expect(store.canClose()).toBe(false);
    expect(store.confirmDiscard()).toBe(true);
    store.keepEditing();
    expect(store.confirmDiscard()).toBe(false);
    store.canClose();
    store.discard();
    expect(store.dirty() || store.confirmDiscard()).toBe(false);
    expect(store.canClose()).toBe(true);
  });

  it('shows the engine errors and selects the first section with errors', async () => {
    const { store } = setup();
    await store.load();
    store.select('diagnostics');
    store.edit('workspace', 'liveTailDebounce', 5);
    store.edit('diagnostics', 'logLevel', 'debug');
    expect(await store.apply()).toBe(false);
    expect(store.selected()).toBe('workspace');
    expect(store.errorFor('workspace', 'liveTailDebounce')).toBe('Must be between 100 ms and 10 s.');
    expect(store.errorFor('workspace', 'other')).toBeNull();
    // The valid section was applied, the invalid one is still a draft.
    expect(store.dirtySections()).toEqual(['workspace']);
    expect(store.sections().find((s) => s.id === 'diagnostics')?.values['logLevel']).toBe('debug');
    // Editing the field clears its error.
    store.edit('workspace', 'liveTailDebounce', 800);
    expect(store.errorFor('workspace', 'liveTailDebounce')).toBeNull();
    expect(store.errors()).toEqual({});
  });

  it('turns a rejected apply into a section error', async () => {
    const backend = new MockSettingsBackend();
    const { store } = setup(backend);
    await store.load();
    store.edit('workspace', 'liveTailDebounce', 900);
    vi.spyOn(backend, 'set').mockRejectedValueOnce('plugin "x" (apply_settings) crashed: boom');
    expect(await store.apply()).toBe(false);
    expect(store.errors()['workspace']).toEqual([{ key: '', message: 'plugin "x" (apply_settings) crashed: boom' }]);
  });

  it('drops drafts of sections that disappeared', async () => {
    const { store } = setup();
    await store.load();
    store.edit('gone', 'x', 1);
    expect(store.dirty()).toBe(true);
    expect(await store.apply()).toBe(true);
    expect(store.dirty()).toBe(false);
  });

  it('resets a field (as an edit) and a section (in the engine)', async () => {
    const { store } = setup();
    await store.load();
    store.edit('diagnostics', 'logMaxFileMb', 20);
    await store.apply();
    expect(store.restartPending()).toBe(true);
    store.resetField('diagnostics', 'logMaxFileMb');
    expect(store.valueOf('diagnostics', 'logMaxFileMb')).toBe(10);
    expect(store.dirty()).toBe(true);
    store.resetField('diagnostics', 'nope');
    store.edit('diagnostics', 'logKeepFiles', 7);
    await store.resetSection('diagnostics');
    expect(store.dirty()).toBe(false);
    expect(store.notice()).toBe('Diagnostics reset to defaults.');
    expect(store.valueOf('diagnostics', 'logKeepFiles')).toBe(5);
    await store.resetSection('ai');
    expect(store.actionError()).toContain('not installed');
    store.dismissNotice();
    expect(store.actionError()).toBeNull();
  });

  it('reports load failures for Retry', async () => {
    const { store } = setup(new MockSettingsBackend({ fail: true }));
    await store.load();
    expect(store.loadError()).toContain('unavailable');
    expect(store.sections()).toEqual([]);
    expect(store.current()).toBeNull();
    store.edit('x', 'y', 1);
    expect(store.valueOf('x', 'y')).toBe(1);
  });

  it('exports, imports and restarts', async () => {
    const { store, backend, restart } = setup();
    await store.load();
    await store.exportTo('/s.json');
    expect(store.notice()).toBe('Settings exported to /s.json.');
    backend.files.set('/in.json', JSON.stringify({ sections: { workspace: { liveTailDebounce: 400 }, diagnostics: { logKeepFiles: 0 }, 'x.y': {} } }));
    await store.importFrom('/in.json');
    expect(store.notice()).toBe('Imported 1 section; kept 1 for later (x.y); rejected diagnostics.');
    expect(store.errorFor('diagnostics', 'logKeepFiles')).toBe('Must be between 1 and 50.');
    expect(store.valueOf('workspace', 'liveTailDebounce')).toBe(400);
    backend.files.set('/two.json', JSON.stringify({ sections: { workspace: {}, diagnostics: {}, '': {} } }));
    vi.spyOn(backend, 'importFrom').mockResolvedValueOnce({ applied: ['workspace', 'diagnostics'], kept: [], rejected: [{ section: '', errors: [{ key: '', message: 'bad id' }] }], restartRequired: true });
    await store.importFrom('/two.json');
    expect(store.notice()).toBe('Imported 2 sections; rejected invalid.');
    expect(store.restartPending()).toBe(true);
    await store.importFrom('/missing.json');
    expect(store.actionError()).toContain('no such file');
    await store.restart();
    expect(restart).toHaveBeenCalled();
    store.setQuery('log');
    store.setShowAdvanced(true);
    expect([store.query(), store.showAdvanced()]).toEqual(['log', true]);
    store.select('workspace');
    expect(store.query()).toBe('');
  });
});
