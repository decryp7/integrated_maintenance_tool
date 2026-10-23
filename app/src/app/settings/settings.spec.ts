import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, FakeDock, provideFakeDock, provideTinyBackend, settle, text } from '../../testing/helpers';
import { WorkspaceStore } from '../core/workspace.store';
import { HOME_AUTO_OPEN_KEY, HomePrefs } from '../home/home-prefs';
import { DialogService } from '../ui/dialog.service';
import { GeneralSettings } from './general-settings';
import { EngineSettingsStore } from './engine-settings.store';
import { SettingsDialogService } from './settings-dialog';
import { SettingsScreen } from './settings-screen';
import { DEFAULT_SETTINGS, LocalStorageSettingsStorage, sanitizeSettings, SETTINGS_KEY, SETTINGS_STORAGE, SettingsStore } from './settings.store';

describe('settings storage', () => {
  beforeEach(() => localStorage.clear());

  it('round-trips and validates', () => {
    const storage = new LocalStorageSettingsStorage();
    expect(storage.load()).toEqual({});
    storage.save({ dateFormat: 'iso', density: 'compact', sourceColors: 'instance' });
    expect(storage.load()).toEqual({ dateFormat: 'iso', density: 'compact', sourceColors: 'instance' });
    expect(sanitizeSettings({ dateFormat: 'weird' as never, density: 'compact' })).toEqual({ dateFormat: 'readable', density: 'compact', sourceColors: 'source' });
    expect(sanitizeSettings({ sourceColors: 'rainbow' as never }).sourceColors).toBe('source');
    expect(sanitizeSettings({ sourceColors: 'off' }).sourceColors).toBe('off');
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

describe('GeneralSettings', () => {
  beforeEach(() => localStorage.clear());

  async function setup() {
    const dock = new FakeDock();
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideFakeDock(dock)] });
    const fixture = TestBed.createComponent(GeneralSettings);
    const resets: number[] = [];
    fixture.componentInstance.layoutReset.subscribe(() => resets.push(1));
    await settle(fixture);
    return { fixture, dock, resets, store: TestBed.inject(WorkspaceStore), settings: TestBed.inject(SettingsStore) };
  }

  it('edits time zone, date format and lifecycle default', async () => {
    const { fixture, store, settings } = await setup();
    buttonByText(fixture, 'Local').click();
    expect(store.timezone()).toBe('local');
    buttonByText(fixture, 'ISO').click();
    expect(settings.dateFormat()).toBe('iso');
    el<HTMLInputElement>(fixture, 'input[type=checkbox]').click();
    expect(store.alwaysShowLifecycle()).toBe(false);
    const overview = TestBed.inject(HomePrefs);
    expect(overview.autoOpen()).toBe(true);
    el<HTMLInputElement>(fixture, '#home-auto-open').click();
    expect(overview.autoOpen()).toBe(false);
    expect(localStorage.getItem(HOME_AUTO_OPEN_KEY)).toBe('false');
    await settle(fixture);
    expect(all(fixture, '[aria-pressed=true]').map((b) => b.textContent?.trim())).toEqual(['Local', 'ISO']);
    // The look moved to Appearance.
    expect(text(fixture)).not.toContain('Theme');
    expect(text(fixture)).toContain('Always show start/stop markers');
    expect(text(fixture)).toContain('Home: show when files or a project open');
  });

  it('resets the layout and says so', async () => {
    const { fixture, dock, resets } = await setup();
    buttonByText(fixture, 'Reset layout').click();
    expect(dock.calls).toContain('reset');
    expect(resets).toEqual([1]);
  });
});

describe('SettingsDialogService', () => {
  it('opens the settings screen once', async () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const service = TestBed.inject(SettingsDialogService);
    await Promise.all([service.open(), service.open()]);
    const stack = TestBed.inject(DialogService).stack();
    expect(stack).toHaveLength(1);
    expect(stack[0]?.options.title).toBe('Settings');
    expect(stack[0]?.component).toBe(SettingsScreen);
  });

  it('asks the settings store before closing, so unapplied edits are not lost by Escape or the backdrop', async () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const store = TestBed.inject(EngineSettingsStore);
    const canClose = vi.spyOn(store, 'canClose').mockReturnValue(false);
    await TestBed.inject(SettingsDialogService).open();
    const dialogs = TestBed.inject(DialogService);
    dialogs.dismissTop();
    expect(canClose).toHaveBeenCalledOnce();
    expect(dialogs.stack()).toHaveLength(1);
    canClose.mockReturnValue(true);
    dialogs.dismissTop();
    expect(dialogs.stack()).toHaveLength(0);
  });
});
