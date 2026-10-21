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
    const cards = all<HTMLInputElement>(fixture, 'input[type=radio][name=imt-theme]');
    expect(cards.map((c) => c.value)).toEqual(['calm', 'console', 'precision']);
    expect(cards[0]?.checked).toBe(true);
    expect(text(fixture)).toContain('Calm');
    expect(text(fixture)).toContain('Console');
    expect(text(fixture)).toContain('Precision');
    cards[1]?.click();
    await settle(fixture);
    expect(prefs.theme()).toBe('console');
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

  it('colours lanes by source as the theme says until the user chooses, and can go back to the default', async () => {
    const { fixture, prefs, theme } = await setup();
    const toggle = () => el<HTMLInputElement>(fixture, '[data-testid=lanes-by-source] input');
    expect(toggle().checked).toBe(false);
    expect(text(fixture)).not.toContain("Use the theme's default");
    prefs.setTheme('console');
    await settle(fixture);
    expect(toggle().checked).toBe(true);
    toggle().click();
    await settle(fixture);
    expect(prefs.lanesBySource()).toBe(false);
    expect(theme.traits().laneFill).toBe('neutral');
    buttonByText(fixture, "Use the theme's default").click();
    await settle(fixture);
    expect(prefs.lanesBySource()).toBeNull();
    expect(theme.traits().laneFill).toBe('bySource');
  });
});
