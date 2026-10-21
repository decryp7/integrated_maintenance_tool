import { computed, DestroyRef, DOCUMENT, effect, inject, Injectable, Injector, signal } from '@angular/core';
import { SettingsStore } from '../../settings/settings.store';
import { effectiveTraits, resolveTokens, traitAttributes, withUiFont } from '../../theme/theme-tokens';
import { themeById } from '../../theme/themes';
import { Density, Scheme, ThemeDef, ThemeTraits, TokenSet } from '../../theme/theme.types';
import { UiPrefsStore } from './ui-prefs.store';

const LIGHT_QUERY = '(prefers-color-scheme: light)';

/**
 * Applies the theme (ADR 0018) to the document: the token set of the chosen theme, scheme and density as CSS custom
 * properties on <html>, and `data-theme`, `data-scheme`, `data-density` and one `data-*` attribute per trait there.
 * Code that draws reads `traits()` and `tokens()` instead of the DOM. "System" follows the operating system live.
 */
@Injectable({ providedIn: 'root' })
export class ThemeService {
  private readonly prefs = inject(UiPrefsStore);
  private readonly settings = inject(SettingsStore);
  private readonly root = inject(DOCUMENT).documentElement;
  private readonly injector = inject(Injector);
  private readonly media = typeof matchMedia === 'function' ? matchMedia(LIGHT_QUERY) : null;
  private readonly systemScheme = signal<Scheme>(this.media?.matches ? 'light' : 'dark');
  private started = false;

  /** The theme in effect. */
  readonly theme = computed<ThemeDef>(() => themeById(this.prefs.theme()));
  /** The scheme in effect ("system" resolved). */
  readonly scheme = computed<Scheme>(() => {
    const mode = this.prefs.scheme();
    return mode === 'system' ? this.systemScheme() : mode;
  });
  readonly density = computed<Density>(() => this.settings.density());
  /** The theme's traits with the user's overrides ("Colour lanes by source"). */
  readonly traits = computed<ThemeTraits>(() => effectiveTraits(this.theme(), { lanesBySource: this.prefs.lanesBySource() }));
  /** The token values in effect, with the interface font the user chose. */
  readonly tokens = computed<TokenSet>(() => withUiFont(resolveTokens(this.theme(), this.scheme(), this.density()), this.prefs.uiFont()));

  constructor() {
    const media = this.media;
    if (media?.addEventListener) {
      const follow = (e: MediaQueryListEvent) => this.systemScheme.set(e.matches ? 'light' : 'dark');
      media.addEventListener('change', follow);
      inject(DestroyRef).onDestroy(() => media.removeEventListener('change', follow));
    }
  }

  /** Starts following the preferences; idempotent. */
  start(): void {
    if (this.started) return;
    this.started = true;
    effect(() => this.apply(this.tokens(), this.traits()), { injector: this.injector });
  }

  private apply(tokens: TokenSet, traits: ThemeTraits): void {
    // Every theme sets every token (`missingTokens` is a gate), so a new set fully replaces the previous one.
    const style = this.root.style;
    for (const [name, value] of Object.entries(tokens)) style.setProperty(`--${name}`, value);
    style.colorScheme = this.scheme();
    this.root.setAttribute('data-theme', this.theme().id);
    this.root.setAttribute('data-scheme', this.scheme());
    this.root.setAttribute('data-density', this.density());
    for (const [attr, value] of traitAttributes(traits)) this.root.setAttribute(attr, value);
  }
}
