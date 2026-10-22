import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, FakeDock, provideFakeDock, provideTinyBackend, settle, text, typeInto } from '../../testing/helpers';
import { LifecycleService } from '../lifecycle/lifecycle.service';
import { DIALOG_REF } from '../ui/dialog.service';
import { AUTO_APPLY_MS, EngineSettingsStore } from './engine-settings.store';
import { MockSettingsBackend } from './mock-settings-backend';
import { SettingsBackend } from './settings-backend';
import { SettingsFiles } from './settings-files';
import { SettingsScreen } from './settings-screen';
import { MockUpdatesBackend } from '../updates/mock-updates-backend';
import { UpdatesBackend } from '../updates/updates-backend';
import { at } from '../../testing/must';
import { MockVideoBackend } from '../video/mock/mock-video-backend';
import { VideoBackend } from '../video/video-backend';
import { BrowserRuntimeBackend } from '../browser/browser-runtime-backend';
import { MockBrowserRuntimeBackend } from '../browser/mock-browser-runtime-backend';

async function setup(backend = new MockSettingsBackend(), withRef = true) {
  const closed = vi.fn();
  const restart = vi.fn(async () => undefined);
  const files = { pickExport: vi.fn(async () => '/out/settings.json' as string | null), pickImport: vi.fn(async () => '/out/settings.json' as string | null), pickPath: vi.fn() };
  const dock = new FakeDock();
  TestBed.configureTestingModule({
    providers: [
      provideTinyBackend(),
      provideFakeDock(dock),
      { provide: SettingsBackend, useValue: backend },
      { provide: UpdatesBackend, useValue: new MockUpdatesBackend('', 1) },
      { provide: SettingsFiles, useValue: files },
      { provide: VideoBackend, useFactory: () => new MockVideoBackend() },
      { provide: BrowserRuntimeBackend, useFactory: () => new MockBrowserRuntimeBackend() },
      { provide: LifecycleService, useValue: { restartKeepingSession: restart } },
      ...(withRef ? [{ provide: DIALOG_REF, useValue: { close: closed, closed: Promise.resolve() } }] : []),
    ],
  });
  const fixture = TestBed.createComponent(SettingsScreen);
  await settle(fixture);
  return { fixture, closed, restart, files, backend, dock, store: TestBed.inject(EngineSettingsStore) };
}

const nav = (fixture: Parameters<typeof el>[0], label: string) => buttonByText(fixture, new RegExp(`^\\s*${label}`));
const escape = (fixture: Parameters<typeof el>[0]) => {
  const e = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
  el(fixture, 'input[type=search]').dispatchEvent(e);
  return e;
};

describe('SettingsScreen', () => {
  it('shows the browser for research under Web browser, below the generated fields, with the Browser to use field', async () => {
    const { fixture } = await setup();
    nav(fixture, 'Web browser').click();
    await settle(fixture);
    expect(text(fixture)).toContain('Browser to use');
    expect(el(fixture, 'imt-browser-runtime-settings')).toBeTruthy();
    expect(text(fixture)).toContain('Browser for research');
  });

  it('shows the FFmpeg runtime under Video', async () => {
    const { fixture } = await setup();
    nav(fixture, 'Video').click();
    await settle(fixture);
    expect(el(fixture, 'nav [aria-current=page]').textContent).toContain('Video');
    expect(el(fixture, 'imt-video-settings')).toBeTruthy();
    expect(text(fixture)).toContain('FFmpeg runtime');
  });

  it('groups the navigation: Application, Data, AI, Plugins, Diagnostics (I7)', async () => {
    const { fixture } = await setup();
    const groups = all(fixture, 'nav .group').map((g) => g.textContent?.trim());
    expect(groups).toEqual(['Application', 'Data', 'AI', 'Plugins', 'Diagnostics']);
    const items = all(fixture, 'nav .nav').map((b) => [...b.querySelectorAll('.name, .status')].map((x) => x.textContent?.trim()).join(' ') || b.textContent?.trim());
    expect(items).toEqual([
      'General',
      'Appearance',
      'Keyboard shortcuts',
      'Window & tray',
      'Web browser',
      'Updates',
      'Disk usage',
      'Performance',
      'Parsers',
      'Video',
      'Live tail',
      'Symbols',
      'AI assistant not installed',
      'Plugins',
      'Windows diagnostics',
      'Example plugin disabled',
      'Diagnostics',
    ]);
    expect(el(fixture, 'nav [aria-current=page]').textContent).toContain('General');
    expect(text(fixture)).toContain('Time zone');
    nav(fixture, 'Appearance').click();
    await settle(fixture);
    expect(el(fixture, 'nav [aria-current=page]').textContent).toContain('Appearance');
    expect(el(fixture, 'imt-appearance-settings')).toBeTruthy();
    expect(text(fixture)).toContain('Colour scheme');
  });

  it('renders a section from its schema, hiding advanced fields until asked', async () => {
    const { fixture } = await setup();
    nav(fixture, 'Diagnostics').click();
    await settle(fixture);
    expect(all(fixture, 'imt-setting-field')).toHaveLength(6);
    expect(text(fixture)).toContain('4 advanced settings hidden');
    buttonByText(fixture, 'Show advanced').click();
    await settle(fixture);
    expect(all(fixture, 'imt-setting-field')).toHaveLength(10);
    expect(el<HTMLInputElement>(fixture, '.adv input').checked).toBe(true);
    el<HTMLInputElement>(fixture, '.adv input').click();
    await settle(fixture);
    expect(all(fixture, 'imt-setting-field')).toHaveLength(6);
  });

  it('searches label, description and key across sections', async () => {
    const { fixture } = await setup();
    typeInto(el<HTMLInputElement>(fixture, 'input[type=search]'), 'timeout');
    await settle(fixture);
    expect(all(fixture, '.content h3').map((h) => h.textContent)).toEqual(['Windows diagnostics']);
    expect(all(fixture, 'imt-setting-field')).toHaveLength(2);
    typeInto(el<HTMLInputElement>(fixture, 'input[type=search]'), 'liveTailDebounce');
    await settle(fixture);
    expect(all(fixture, '.content h3').map((h) => h.textContent)).toEqual(['Live tail']);
    typeInto(el<HTMLInputElement>(fixture, 'input[type=search]'), 'zzz');
    await settle(fixture);
    expect(text(fixture)).toContain('No settings match “zzz”.');
    typeInto(el<HTMLInputElement>(fixture, 'input[type=search]'), 'cache');
    await settle(fixture);
    // A read-only (disabled plugin) hit is shown disabled.
    expect(el<HTMLInputElement>(fixture, 'imt-setting-field input').disabled).toBe(true);
    typeInto(el<HTMLInputElement>(fixture, 'input[type=search]'), 'log level');
    await settle(fixture);
    typeInto(el<HTMLSelectElement>(fixture, 'imt-setting-field select'), 'debug', 'change');
    await settle(fixture);
    el<HTMLButtonElement>(fixture, 'imt-setting-field button.reset').click();
    await settle(fixture);
    expect(text(fixture)).not.toContain('Not saved in');
  });

  it('searches the Parsers section and the parser library, and opens it from a hit', async () => {
    const { fixture, store } = await setup();
    const search = el<HTMLInputElement>(fixture, 'input[type=search]');
    typeInto(search, 'parsers');
    await settle(fixture);
    expect(all(fixture, '.content h3').map((h) => h.textContent)).toContain('Parsers');
    expect(text(fixture)).not.toContain('No settings match');
    typeInto(search, 'serilog clef');
    await vi.waitFor(async () => {
      await settle(fixture);
      expect(all(fixture, '[data-testid=settings-search-parser]').map((li) => li.textContent)).toEqual([expect.stringContaining('Serilog CLEF')]);
    });
    el<HTMLButtonElement>(fixture, '[data-testid=settings-search-open-parsers]').click();
    await settle(fixture);
    expect(store.query()).toBe('');
    expect(el(fixture, 'nav [aria-current=page]').textContent).toContain('Parsers');
    // Settings keeps a link: the library itself is the Parsers view (I3).
    expect(el(fixture, '[data-testid=settings-open-parsers-view]')).toBeTruthy();
    expect(fixture.nativeElement.querySelector('imt-parsers-settings')).toBeNull();
  });

  it('opens the keyboard shortcut editor from its navigation entry', async () => {
    const { fixture } = await setup();
    nav(fixture, 'Keyboard shortcuts').click();
    await settle(fixture);
    expect(el(fixture, 'nav [aria-current=page]').textContent).toContain('Keyboard shortcuts');
    expect(el(fixture, 'imt-keyboard-settings')).toBeTruthy();
  });

  it('applies a change by itself, shows the engine\'s errors inline and offers Undo (I1)', async () => {
    const { fixture, store } = await setup();
    // Fake timers only now: `settle` itself waits on setTimeout.
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    const settle = async (f: typeof fixture) => {
      await vi.advanceTimersByTimeAsync(0);
      TestBed.tick();
      await f.whenStable();
    };
    try {
      nav(fixture, 'Live tail').click();
      await settle(fixture);
      expect(all(fixture, 'button').some((b) => b.textContent?.trim() === 'Apply')).toBe(false);
      typeInto(el<HTMLInputElement>(fixture, 'imt-setting-field input'), '5');
      await vi.advanceTimersByTimeAsync(AUTO_APPLY_MS + 10);
      await settle(fixture);
      expect(el(fixture, 'imt-setting-field [role=alert]').textContent).toContain('Must be between 100 ms and 10 s.');
      expect(el(fixture, 'nav [aria-label="unsaved changes"]')).toBeTruthy();
      expect(text(fixture)).toContain('Not saved in 1 section');
      typeInto(el<HTMLInputElement>(fixture, 'imt-setting-field input'), '900');
      await settle(fixture);
      expect(all(fixture, 'imt-setting-field [role=alert]')).toHaveLength(0);
      await vi.advanceTimersByTimeAsync(AUTO_APPLY_MS + 10);
      await settle(fixture);
      expect(store.valueOf('workspace', 'liveTailDebounce')).toBe(900);
      expect(el(fixture, '[data-testid=settings-notice]').textContent).toContain('Live tail delay saved.');
      el<HTMLButtonElement>(fixture, '[data-testid=settings-undo]').click();
      await settle(fixture);
      expect(store.valueOf('workspace', 'liveTailDebounce')).toBe(750);
      expect(text(fixture)).toContain('Live tail restored.');
      expect(all(fixture, '[data-testid=settings-undo]')).toHaveLength(0);
      buttonByText(fixture, 'Dismiss').click();
      await settle(fixture);
      expect(text(fixture)).not.toContain('restored');
    } finally {
      vi.useRealTimers();
    }
  });

  it('the Live tail page holds the engine delay and the watch and detector options (I2)', async () => {
    const { fixture } = await setup();
    nav(fixture, 'Live tail').click();
    await settle(fixture);
    expect(el(fixture, 'imt-live-tail-settings')).toBeTruthy();
    expect(text(fixture)).toContain('Live tail delay');
    expect(text(fixture)).toContain('Poll every');
    expect(text(fixture)).toContain('Run detectors every');
  });

  it('shows the Updates page with the app version and a check button (I5)', async () => {
    const { fixture } = await setup();
    nav(fixture, 'Updates').click();
    await settle(fixture);
    expect(el(fixture, 'imt-updates-settings')).toBeTruthy();
    expect(el(fixture, '[data-testid=updates-facts]').textContent).toContain('Schedule');
    // AI runtime and model updates stay in the AI section; this page only says so.
    expect(el(fixture, '[data-testid=updates-ai-note]').textContent).toContain('AI section');
  });

  it('shows section-level engine errors', async () => {
    const backend = new MockSettingsBackend();
    const { fixture, store } = await setup(backend);
    nav(fixture, 'Live tail').click();
    store.edit('workspace', 'liveTailDebounce', 900);
    vi.spyOn(backend, 'set').mockRejectedValueOnce('the plugin refused');
    await store.apply();
    await settle(fixture);
    expect(all(fixture, '.content > .error').map((e) => e.textContent)).toEqual(['the plugin refused']);
  });

  it('resets a section and offers a restart when needed', async () => {
    const { fixture, restart } = await setup();
    nav(fixture, 'Diagnostics').click();
    await settle(fixture);
    const size = at(all<HTMLInputElement>(fixture, 'imt-setting-field input[type=number]'), 0);
    typeInto(size, '20');
    await settle(fixture);
    await TestBed.inject(EngineSettingsStore).apply();
    await settle(fixture);
    const banner = el(fixture, '.banner.restart');
    expect(banner.getAttribute('role')).toBe('status');
    expect(banner.textContent).toContain('take effect after IMT restarts');
    buttonByText(fixture, 'Restart now').click();
    expect(restart).toHaveBeenCalled();
    buttonByText(fixture, 'Reset section').click();
    await settle(fixture);
    expect(text(fixture)).toContain('Diagnostics reset to defaults.');
    expect(all<HTMLInputElement>(fixture, 'imt-setting-field input[type=number]')[0]?.value).toBe('10');
  });

  it('shows read-only and not-installed sections with their note', async () => {
    const { fixture } = await setup();
    nav(fixture, 'Example plugin').click();
    await settle(fixture);
    expect(el(fixture, '[role=note]').textContent).toContain('disabled');
    expect(buttonByText(fixture, 'Reset section').disabled).toBe(true);
    expect(all<HTMLInputElement>(fixture, 'imt-setting-field input').every((i) => i.disabled)).toBe(true);
    nav(fixture, 'AI assistant').click();
    await settle(fixture);
    expect(el(fixture, '[role=note]').textContent).toContain('not installed');
    expect(all(fixture, 'imt-setting-field')).toHaveLength(0);
    expect(text(fixture)).not.toContain('This section has no settings.');
  });

  it('says when a section has no settings', async () => {
    const backend = new MockSettingsBackend();
    const { fixture, store } = await setup(backend);
    const overview = store.overview()!;
    vi.spyOn(backend, 'sections').mockResolvedValueOnce({ ...overview, sections: [{ ...overview.sections.find((s) => s.id === 'diagnostics')!, fields: [], description: '' }] });
    await store.load();
    store.select('diagnostics');
    await settle(fixture);
    expect(text(fixture)).toContain('This section has no settings.');
  });

  it('guards edits the engine refused on Close and Escape', async () => {
    const { fixture, closed, store } = await setup();
    // Nothing edited: Close closes, Escape is left to the dialog.
    expect(escape(fixture).defaultPrevented).toBe(false);
    buttonByText(fixture, 'Close').click();
    expect(closed).toHaveBeenCalledTimes(1);
    store.edit('workspace', 'liveTailDebounce', 5);
    await store.apply();
    await settle(fixture);
    buttonByText(fixture, 'Close').click();
    await settle(fixture);
    expect(el(fixture, '[role=alertdialog]').textContent).toContain('not saved');
    buttonByText(fixture, 'Keep editing').click();
    await settle(fixture);
    expect(all(fixture, '[role=alertdialog]')).toHaveLength(0);
    expect(escape(fixture).defaultPrevented).toBe(true);
    await settle(fixture);
    buttonByText(fixture, 'Discard changes').click();
    await settle(fixture);
    expect(closed).toHaveBeenCalledTimes(2);
    expect(store.dirty()).toBe(false);
  });

  it('exports and imports through file dialogs', async () => {
    const { fixture, files, backend } = await setup();
    buttonByText(fixture, 'Export settings…').click();
    await settle(fixture);
    expect(backend.files.has('/out/settings.json')).toBe(true);
    expect(text(fixture)).toContain('Settings exported to /out/settings.json.');
    buttonByText(fixture, 'Import settings…').click();
    await settle(fixture);
    expect(text(fixture)).toContain('Imported');
    files.pickExport.mockResolvedValueOnce(null);
    files.pickImport.mockResolvedValueOnce(null);
    const calls = backend.calls.length;
    buttonByText(fixture, 'Export settings…').click();
    buttonByText(fixture, 'Import settings…').click();
    await settle(fixture);
    expect(backend.calls.length).toBe(calls);
    files.pickImport.mockResolvedValueOnce('/nope.json');
    buttonByText(fixture, 'Import settings…').click();
    await settle(fixture);
    expect(el(fixture, '.banner.err').textContent).toContain('no such file');
  });

  it('shows load failures with Retry', async () => {
    const backend = new MockSettingsBackend({ fail: true });
    const { fixture } = await setup(backend);
    expect(el(fixture, '.banner.err').textContent).toContain('Could not load the settings');
    buttonByText(fixture, 'Retry').click();
    await settle(fixture);
    expect(backend.calls.filter((c) => c === 'sections')).toHaveLength(2);
  });

  it('closes after the layout was reset, and works without a dialog', async () => {
    const { fixture, closed, dock } = await setup();
    buttonByText(fixture, 'Reset layout').click();
    expect(dock.calls).toContain('reset');
    expect(closed).toHaveBeenCalled();
    TestBed.resetTestingModule();
    const bare = await setup(new MockSettingsBackend(), false);
    buttonByText(bare.fixture, 'Close').click();
    expect(bare.closed).not.toHaveBeenCalled();
  });

  it('shows the loading state', async () => {
    const backend = new MockSettingsBackend();
    let release!: () => void;
    vi.spyOn(backend, 'sections').mockImplementationOnce(() => new Promise((r) => (release = () => r({ sections: [], restartPending: false, safeMode: false }))));
    const { fixture } = await setup(backend);
    expect(text(fixture)).toContain('Loading settings…');
    release();
    await settle(fixture);
    expect(text(fixture)).not.toContain('Loading settings…');
  });
});
