import { TestBed } from '@angular/core/testing';
import { settle } from '../../../testing/helpers';
import { SettingsStore } from '../../settings/settings.store';
import { ThemeService } from './theme.service';
import { UiPrefsStore } from './ui-prefs.store';

/** A controllable `prefers-color-scheme: light` query. */
function fakeMedia(matches: boolean) {
  const listeners = new Set<(e: MediaQueryListEvent) => void>();
  const mql = {
    matches,
    addEventListener: (_: string, l: (e: MediaQueryListEvent) => void) => listeners.add(l),
    removeEventListener: (_: string, l: (e: MediaQueryListEvent) => void) => listeners.delete(l),
  };
  vi.spyOn(window, 'matchMedia').mockReturnValue(mql as unknown as MediaQueryList);
  return { change: (m: boolean) => listeners.forEach((l) => l({ matches: m } as MediaQueryListEvent)), listeners };
}

describe('ThemeService', () => {
  const root = document.documentElement;
  beforeEach(() => localStorage.clear());
  afterEach(() => {
    vi.restoreAllMocks();
    for (const a of [...root.attributes]) if (a.name.startsWith('data-')) root.removeAttribute(a.name);
    root.removeAttribute('style');
  });

  it('sets the tokens, the scheme, the density and every trait on <html>, and follows changes', async () => {
    fakeMedia(false);
    const theme = TestBed.inject(ThemeService);
    const prefs = TestBed.inject(UiPrefsStore);
    theme.start();
    await settle();
    expect(root.getAttribute('data-theme')).toBe('calm');
    expect(root.getAttribute('data-scheme')).toBe('dark');
    expect(root.getAttribute('data-density')).toBe('comfortable');
    expect(root.getAttribute('data-runs')).toBe('line');
    expect(root.getAttribute('data-lane-fill')).toBe('neutral');
    expect(root.style.getPropertyValue('--bg-1')).toBe('#17181b');
    expect(root.style.getPropertyValue('--row-h')).toBe('24px');
    expect(root.style.colorScheme).toBe('dark');

    prefs.setTheme('console');
    prefs.setScheme('light');
    TestBed.inject(SettingsStore).setDensity('compact');
    await settle();
    expect(root.getAttribute('data-theme')).toBe('console');
    expect(root.getAttribute('data-scheme')).toBe('light');
    expect(root.getAttribute('data-density')).toBe('compact');
    expect(root.getAttribute('data-runs')).toBe('pill');
    expect(root.getAttribute('data-surface')).toBe('cards');
    expect(root.style.getPropertyValue('--accent')).toBe('#5b47e8');
    expect(root.style.getPropertyValue('--row-h')).toBe('22px');
    expect(theme.tokens()['accent']).toBe('#5b47e8');

    prefs.setLanesBySource(false);
    await settle();
    expect(root.getAttribute('data-lane-fill')).toBe('neutral');
    expect(theme.traits().laneFill).toBe('neutral');

    // Precision: barcode lanes and bracket runs; the interface font is the system one unless the user keeps the theme's.
    prefs.setTheme('precision');
    prefs.setLanesBySource(null);
    await settle();
    expect(root.getAttribute('data-theme')).toBe('precision');
    expect(root.getAttribute('data-runs')).toBe('bracket');
    expect(root.getAttribute('data-lane-fill')).toBe('barcode');
    expect(root.style.getPropertyValue('--font-sans')).toContain('Segoe UI');
    expect(root.style.getPropertyValue('--font-mono')).toContain('IBM Plex Mono');
    prefs.setUiFont('theme');
    await settle();
    expect(root.style.getPropertyValue('--font-sans')).toContain('IBM Plex Sans');
  });

  it('follows the operating system live while the scheme is "system"', async () => {
    const media = fakeMedia(true);
    const theme = TestBed.inject(ThemeService);
    theme.start();
    await settle();
    expect(theme.scheme()).toBe('light');
    media.change(false);
    await settle();
    expect(theme.scheme()).toBe('dark');
    expect(root.getAttribute('data-scheme')).toBe('dark');
    // A fixed scheme ignores the system.
    TestBed.inject(UiPrefsStore).setScheme('light');
    media.change(false);
    expect(theme.scheme()).toBe('light');
    TestBed.resetTestingModule();
    expect(media.listeners.size).toBe(0);
  });

  it('starts once', async () => {
    fakeMedia(false);
    const theme = TestBed.inject(ThemeService);
    const set = vi.spyOn(root, 'setAttribute');
    theme.start();
    theme.start();
    await settle();
    expect(set.mock.calls.filter(([name]) => name === 'data-theme')).toHaveLength(1);
  });

  it('works without matchMedia (follows dark)', () => {
    const original = window.matchMedia;
    Object.defineProperty(window, 'matchMedia', { value: undefined, configurable: true });
    try {
      expect(TestBed.inject(ThemeService).scheme()).toBe('dark');
    } finally {
      Object.defineProperty(window, 'matchMedia', { value: original, configurable: true });
    }
  });
});
