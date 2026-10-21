import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import { ThemeService } from '../core/workspace/theme.service';
import { UiPrefsStore } from '../core/workspace/ui-prefs.store';
import { SOURCE_COLOR_LABELS } from '../sources/source-colors';
import { resolveTokens } from '../theme/theme-tokens';
import { BUILT_IN_THEMES } from '../theme/themes';
import { Density, DENSITIES, SCHEME_MODES, SchemeMode, ThemeId, UI_FONTS, UiFont } from '../theme/theme.types';
import { Switch } from '../shared/switch';
import { HelpHint } from '../ui/help-hint';
import { SettingsStore, SOURCE_COLOR_MODES } from './settings.store';

/** Swatches of a theme's preview card, in the scheme in effect. */
const PREVIEW = ['bg-0', 'bg-1', 'accent', 'src-1', 'src-2', 'src-3', 'mk-fatal'] as const;

/**
 * Settings ▸ Appearance (ADR 0018, 0028): theme, colour scheme, interface font, density, lane colouring and source colours. UI preferences of
 * this user, kept in the browser, applied at once in every window.
 */
@Component({
  selector: 'imt-appearance-settings',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Switch, HelpHint],
  template: `
    <fieldset data-testid="theme-setting">
      <legend i18n="@@settings.appearance.theme">Theme</legend>
      <div class="themes" role="radiogroup" aria-label="Theme" i18n-aria-label="@@settings.appearance.theme.aria">
        @for (t of themes(); track t.id) {
          <label class="theme" [class.on]="prefs.theme() === t.id" [attr.data-theme-card]="t.id">
            <input type="radio" name="imt-theme" [value]="t.id" [checked]="prefs.theme() === t.id" (change)="prefs.setTheme(t.id)" />
            <span class="preview" aria-hidden="true" [style.background]="t.swatches[0]" [style.border-color]="t.swatches[1]">
              <span class="panel" [style.background]="t.swatches[1]">
                <span class="bar" [style.background]="t.swatches[2]"></span>
                <span class="dots">
                  @for (c of t.swatches.slice(3); track $index) {
                    <span class="dot" [style.background]="c"></span>
                  }
                </span>
              </span>
            </span>
            <span class="name">{{ t.name }}</span>
            <span class="about">{{ t.about }}</span>
          </label>
        }
      </div>
    </fieldset>
    <fieldset>
      <legend i18n="@@settings.appearance.scheme">Colour scheme</legend>
      <div class="seg" data-testid="scheme-setting">
        @for (s of schemes; track s) {
          <button class="btn" type="button" [class.active]="prefs.scheme() === s" [attr.aria-pressed]="prefs.scheme() === s" (click)="prefs.setScheme(s)">{{ schemeLabels[s] }}</button>
        }
      </div>
    </fieldset>
    <fieldset>
      <legend i18n="@@settings.appearance.uiFont">Interface font <imt-help label="Interface font" text="The font of menus, panels and dialogs. System uses your operating system's font, like other desktop apps; theme uses the theme's own. Log text is always monospaced." i18n-label="@@settings.appearance.uiFont.helpLabel" i18n-text="@@settings.appearance.uiFont.help" /></legend>
      <div class="seg" data-testid="ui-font-setting">
        @for (f of uiFonts; track f) {
          <button class="btn" type="button" [class.active]="prefs.uiFont() === f" [attr.aria-pressed]="prefs.uiFont() === f" (click)="prefs.setUiFont(f)">{{ uiFontLabels[f] }}</button>
        }
      </div>
    </fieldset>
    <fieldset>
      <legend i18n="@@settings.appearance.density">Density <imt-help label="Density" text="Row and lane heights. Compact fits more events and lanes on the screen; comfortable is easier to read and to click." i18n-label="@@settings.appearance.density.helpLabel" i18n-text="@@settings.appearance.density.help" /></legend>
      <div class="seg" data-testid="density-setting">
        @for (d of densities; track d) {
          <button class="btn" type="button" [class.active]="settings.density() === d" [attr.aria-pressed]="settings.density() === d" (click)="settings.setDensity(d)">{{ densityLabels[d] }}</button>
        }
      </div>
    </fieldset>
    <fieldset>
      <legend i18n="@@settings.appearance.timeline">Timeline</legend>
      <div class="row">
        <imt-switch data-testid="lanes-by-source" [checked]="bySource()" (checkedChange)="prefs.setLanesBySource($event)" i18n="@@settings.appearance.lanesBySource">Colour lanes by source</imt-switch>
        <imt-help label="Colour lanes by source" text="Draw each lane's volume in the colour of its source instead of a neutral grey. Each theme has its own default." i18n-label="@@settings.appearance.lanesBySource.helpLabel" i18n-text="@@settings.appearance.lanesBySource.help" />
        @if (prefs.lanesBySource() !== null) {
          <button class="link" type="button" (click)="prefs.setLanesBySource(null)" i18n="@@settings.appearance.lanesBySource.reset">Use the theme's default</button>
        }
      </div>
    </fieldset>
    <fieldset>
      <legend i18n="@@settings.general.sourceColors">Source colours <imt-help label="Source colours" text="A coloured stripe at the start of each log line, with matching dots in the Explorer, the inspector and the timeline lanes, tells which file or instance the line came from. Hover a stripe or dot for its name." i18n-label="@@settings.general.sourceColors.helpLabel" i18n-text="@@settings.general.sourceColors.help" /></legend>
      <div class="seg" data-testid="source-colors">
        @for (m of sourceColorModes; track m) {
          <button class="btn" type="button" [class.active]="settings.sourceColors() === m" [attr.aria-pressed]="settings.sourceColors() === m" (click)="settings.setSourceColors(m)">{{ sourceColorLabels[m] }}</button>
        }
      </div>
    </fieldset>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: var(--s-4); }
    fieldset { border: 0; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
    legend { padding: 0 0 6px; font-size: var(--fs-xs); font-weight: 600; letter-spacing: 0.08em; text-transform: uppercase; color: var(--muted); display: flex; align-items: center; gap: 6px; }
    .seg { display: flex; gap: var(--s-1); flex-wrap: wrap; }
    .seg .btn { text-transform: capitalize; }
    .row { display: flex; align-items: center; gap: var(--s-2); color: var(--text-2); flex-wrap: wrap; }
    .link { border: 0; background: none; padding: 0; color: var(--accent-text); cursor: pointer; text-decoration: underline; font-size: var(--fs-sm); }
    .themes { display: grid; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); gap: var(--s-3); }
    .theme { position: relative; display: grid; grid-template-rows: auto auto auto; gap: var(--s-1); padding: var(--s-2); border: 1px solid var(--border-2); border-radius: var(--r-3); background: var(--bg-3); cursor: pointer; transition: border-color var(--dur-fast) var(--ease), box-shadow var(--dur-fast) var(--ease); }
    .theme:hover { border-color: var(--border-3); }
    .theme.on { border-color: var(--accent); box-shadow: 0 0 0 1px var(--accent); }
    .theme:has(input:focus-visible) { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
    .theme input { position: absolute; opacity: 0; width: 1px; height: 1px; margin: 0; }
    .preview { display: block; height: 64px; padding: 8px; border-radius: var(--r-2); border: 1px solid; }
    .panel { display: flex; flex-direction: column; justify-content: space-between; height: 100%; padding: 6px; border-radius: var(--r-1); }
    .bar { display: block; width: 45%; height: 6px; border-radius: 3px; }
    .dots { display: flex; gap: 4px; }
    .dot { width: 10px; height: 10px; border-radius: 50%; }
    .name { font-weight: 600; color: var(--text); }
    .about { font-size: var(--fs-xs); color: var(--muted); }
  `,
})
export class AppearanceSettings {
  protected readonly prefs = inject(UiPrefsStore);
  protected readonly settings = inject(SettingsStore);
  private readonly theme = inject(ThemeService);

  /** The built-in themes with their preview colours in the scheme in effect. */
  protected readonly themes = computed(() =>
    BUILT_IN_THEMES.map((t) => {
      const tokens = resolveTokens(t, this.theme.scheme(), this.theme.density());
      return { id: t.id, name: THEME_NAMES[t.id], about: THEME_ABOUT[t.id], swatches: PREVIEW.map((n) => tokens[n] ?? '') };
    }),
  );
  /** "Colour lanes by source" as it is in effect (the user's choice or the theme's default). */
  protected readonly bySource = computed(() => this.theme.traits().laneFill === 'bySource');

  protected readonly schemes: readonly SchemeMode[] = SCHEME_MODES;
  protected readonly schemeLabels: Record<SchemeMode, string> = {
    system: $localize`:Colour scheme option (capitalised by CSS)@@settings.appearance.scheme.system:system`,
    light: $localize`:Colour scheme option (capitalised by CSS)@@settings.appearance.scheme.light:light`,
    dark: $localize`:Colour scheme option (capitalised by CSS)@@settings.appearance.scheme.dark:dark`,
  };
  protected readonly uiFonts: readonly UiFont[] = UI_FONTS;
  protected readonly uiFontLabels: Record<UiFont, string> = {
    system: $localize`:Interface font option (capitalised by CSS)@@settings.appearance.uiFont.system:system`,
    theme: $localize`:Interface font option (capitalised by CSS)@@settings.appearance.uiFont.theme:theme`,
  };
  protected readonly densities: readonly Density[] = DENSITIES;
  protected readonly densityLabels: Record<Density, string> = {
    comfortable: $localize`:Density option (capitalised by CSS)@@settings.general.density.comfortable:comfortable`,
    compact: $localize`:Density option (capitalised by CSS)@@settings.general.density.compact:compact`,
  };
  protected readonly sourceColorModes = SOURCE_COLOR_MODES;
  protected readonly sourceColorLabels = SOURCE_COLOR_LABELS;
}

/** Localised theme names. */
export const THEME_NAMES: Record<ThemeId, string> = {
  calm: $localize`:Theme name@@settings.appearance.theme.calm:Calm`,
  console: $localize`:Theme name@@settings.appearance.theme.console:Console`,
  precision: $localize`:Theme name@@settings.appearance.theme.precision:Precision`,
};

const THEME_ABOUT: Record<ThemeId, string> = {
  calm: $localize`:Theme description@@settings.appearance.theme.calm.about:Quiet and neutral; only the data has colour.`,
  console: $localize`:Theme description@@settings.appearance.theme.console.about:Soft cards and richer colour; problems stand out.`,
  precision: $localize`:Theme description@@settings.appearance.theme.precision.about:Paper and ink, hairlines and mono figures; the densest view.`,
};
