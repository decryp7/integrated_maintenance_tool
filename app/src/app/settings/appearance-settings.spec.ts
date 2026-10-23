import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, text } from '../../testing/helpers';
import { ThemeService } from '../core/workspace/theme.service';
import { UiPrefsStore } from '../core/workspace/ui-prefs.store';
import { AppearanceSettings } from './appearance-settings';
import { SettingsStore } from './settings.store';

describe('AppearanceSettings', () => {
  beforeEach(() => localStorage.clear());

  async function setup() {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const fixture = TestBed.createComponent(AppearanceSettings);
    await settle(fixture);
    return { fixture, prefs: TestBed.inject(UiPrefsStore), settings: TestBed.inject(SettingsStore), theme: TestBed.inject(ThemeService) };
  }

  it('offers every built-in theme as a radio card with a preview in the scheme in effect', async () => {
    const { fixture, prefs, theme } = await setup();
    const cards = all<HTMLInputElement>(fixture, 'input[type=radio][name=debrief-theme]');
    expect(cards.map((c) => c.closest('label')?.getAttribute('data-theme-card'))).toEqual(['system', 'calm', 'console', 'precision', 'contrast']);
    expect(cards[0]?.checked).toBe(true);
    expect(prefs.theme()).toBeNull();
    expect(text(fixture)).toContain('Follow system');
    expect(text(fixture)).toContain('Calm');
    expect(text(fixture)).toContain('Console');
    expect(text(fixture)).toContain('Precision');
    cards[2]?.click();
    await settle(fixture);
    expect(prefs.theme()).toBe('console');
    cards[0]?.click();
    await settle(fixture);
    expect(prefs.theme()).toBeNull();
    cards[2]?.click();
    await settle(fixture);
    expect(el(fixture, '[data-theme-card=console]').classList).toContain('on');
    // The preview shows the theme's own colours, for the scheme in effect.
    const panel = (id: string) => el<HTMLElement>(fixture, `[data-theme-card=${id}] .panel`).style.background;
    prefs.setScheme('light');
    await settle(fixture);
    expect(theme.scheme()).toBe('light');
    expect(panel('calm')).toBe('rgb(255, 255, 255)');
    prefs.setScheme('dark');
    await settle(fixture);
    expect(panel('calm')).toBe('rgb(23, 24, 27)');
  });

  it('offers the system interface font by default and the theme font as the alternative', async () => {
    const { fixture, prefs, theme } = await setup();
    const buttons = all<HTMLButtonElement>(fixture, '[data-testid=ui-font-setting] button');
    expect(buttons.map((b) => b.textContent?.trim())).toEqual(['system', 'theme']);
    expect(prefs.uiFont()).toBe('system');
    expect(buttons[0]?.getAttribute('aria-pressed')).toBe('true');
    expect(theme.tokens()['font-sans']).toContain('Segoe UI');
    prefs.setTheme('precision');
    buttons[1]?.click();
    await settle(fixture);
    expect(prefs.uiFont()).toBe('theme');
    expect(buttons[1]?.getAttribute('aria-pressed')).toBe('true');
    expect(theme.tokens()['font-sans']).toContain('IBM Plex Sans');
  });

  it('sets the colour scheme, the density and the source colours', async () => {
    const { fixture, prefs, settings } = await setup();
    buttonByText(fixture, 'light').click();
    expect(prefs.scheme()).toBe('light');
    buttonByText(fixture, 'compact').click();
    expect(settings.density()).toBe('compact');
    all<HTMLButtonElement>(fixture, '[data-testid=source-colors] button')[2]?.click();
    expect(settings.sourceColors()).toBe('off');
    await settle(fixture);
    expect(all(fixture, '[aria-pressed=true]').map((b) => b.textContent?.trim())).toEqual(['light', 'system', 'compact', 'Off']);
  });

  it('Source colours: Off also draws the lanes neutral; the other choices leave lane colour to the theme (I6)', async () => {
    const { fixture, prefs, theme, settings } = await setup();
    prefs.setTheme('console');
    await settle(fixture);
    expect(text(fixture)).not.toContain('Colour lanes by source');
    expect(theme.traits().laneFill).toBe('bySource');
    all<HTMLButtonElement>(fixture, '[data-testid=source-colors] button')[2]?.click();
    await settle(fixture);
    expect(settings.sourceColors()).toBe('off');
    expect(theme.traits().laneFill).toBe('neutral');
    all<HTMLButtonElement>(fixture, '[data-testid=source-colors] button')[1]?.click();
    await settle(fixture);
    expect(theme.traits().laneFill).toBe('bySource');
  });
});
