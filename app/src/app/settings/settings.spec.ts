import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, FakeDock, provideFakeDock, provideTinyBackend, settle, text } from '../../testing/helpers';
import { WorkspaceStore } from '../core/workspace.store';
import { DIALOG_REF, DialogService } from '../ui/dialog.service';
import { SettingsDialog, SettingsDialogService } from './settings-dialog';
import { DEFAULT_SETTINGS, LocalStorageSettingsStorage, sanitizeSettings, SETTINGS_KEY, SETTINGS_STORAGE, SettingsStore } from './settings.store';

describe('settings storage', () => {
  beforeEach(() => localStorage.clear());

  it('round-trips and validates', () => {
    const storage = new LocalStorageSettingsStorage();
    expect(storage.load()).toEqual({});
    storage.save({ dateFormat: 'iso', density: 'compact' });
    expect(storage.load()).toEqual({ dateFormat: 'iso', density: 'compact' });
    expect(sanitizeSettings({ dateFormat: 'weird' as never, density: 'compact' })).toEqual({ dateFormat: 'readable', density: 'compact' });
    expect(sanitizeSettings({})).toEqual(DEFAULT_SETTINGS);
  });

  it('ignores other versions, bad JSON and storage failures', () => {
    const storage = new LocalStorageSettingsStorage();
    localStorage.setItem(SETTINGS_KEY, JSON.stringify({ version: 7, settings: { density: 'compact' } }));
    expect(storage.load()).toEqual({});
    localStorage.setItem(SETTINGS_KEY, '{bad');
    expect(storage.load()).toEqual({});
    localStorage.setItem(SETTINGS_KEY, JSON.stringify({ version: 1, settings: 'x' }));
    expect(storage.load()).toEqual({});
    const set = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('quota');
    });
    expect(() => storage.save(DEFAULT_SETTINGS)).not.toThrow();
    set.mockRestore();
  });
});

describe('SettingsStore', () => {
  beforeEach(() => localStorage.clear());

  it('starts from defaults, persists changes and reloads them', () => {
    const store = TestBed.inject(SettingsStore);
    expect(TestBed.inject(SETTINGS_STORAGE)).toBeInstanceOf(LocalStorageSettingsStorage);
    expect(store.dateFormat()).toBe('readable');
    store.setDateFormat('iso');
    store.setDensity('compact');
    TestBed.resetTestingModule();
    const again = TestBed.inject(SettingsStore);
    expect(again.dateFormat()).toBe('iso');
    expect(again.density()).toBe('compact');
  });
});

describe('SettingsDialog', () => {
  beforeEach(() => localStorage.clear());

  async function setup() {
    const dock = new FakeDock();
    const closed = vi.fn();
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideFakeDock(dock), { provide: DIALOG_REF, useValue: { close: closed, closed: Promise.resolve() } }] });
    const fixture = TestBed.createComponent(SettingsDialog);
    await settle(fixture);
    return { fixture, dock, closed, store: TestBed.inject(WorkspaceStore), settings: TestBed.inject(SettingsStore) };
  }

  it('edits theme, time zone, date format, density and lifecycle default', async () => {
    const { fixture, store, settings } = await setup();
    buttonByText(fixture, 'light').click();
    expect(store.theme()).toBe('light');
    buttonByText(fixture, 'Local').click();
    expect(store.timezone()).toBe('local');
    buttonByText(fixture, 'ISO').click();
    expect(settings.dateFormat()).toBe('iso');
    buttonByText(fixture, 'compact').click();
    expect(settings.density()).toBe('compact');
    const box = el<HTMLInputElement>(fixture, 'input[type=checkbox]');
    box.click();
    expect(store.alwaysShowLifecycle()).toBe(false);
    await settle(fixture);
    expect(all(fixture, '[aria-pressed=true]').map((b) => b.textContent?.trim())).toEqual(['light', 'Local', 'ISO', 'compact']);
    expect(text(fixture)).toContain('Always show start/stop markers');
  });

  it('resets the layout and closes', async () => {
    const { fixture, dock, closed } = await setup();
    buttonByText(fixture, 'Reset layout').click();
    expect(dock.calls).toContain('reset');
    expect(closed).toHaveBeenCalled();
    buttonByText(fixture, 'Done').click();
  });

  it('is opened through the dialog service', () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    TestBed.inject(SettingsDialogService).open();
    expect(TestBed.inject(DialogService).stack()[0].options.title).toBe('Settings');
  });
});
