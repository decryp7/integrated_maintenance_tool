import { ChangeDetectionStrategy, Component, inject, output } from '@angular/core';
import { DateFormat, TimeZoneMode } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { DockService } from '../dock/dock.service';
import { LanguageService } from '../i18n/language.service';
import { LanguageChoice } from '../i18n/locale';
import { OverviewPrefs } from '../overview/overview-prefs';
import { HelpHint } from '../ui/help-hint';
import { SettingsStore } from './settings.store';

/**
 * UI-only preferences (kept in the browser): language (applied after a restart), time zone, date format, lifecycle
 * markers, overview, layout. Everything but the language applies immediately. The look is in Appearance.
 */
@Component({
  selector: 'imt-general-settings',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [HelpHint],
  template: `
    <fieldset data-testid="language-setting">
      <legend i18n="Setting for the UI language@@settings.general.language">Language <imt-help label="Language" text="The language of menus, dialogs and messages. Log text and file contents are never translated. The new language is used after IMT restarts." i18n-label="@@settings.general.language.helpLabel" i18n-text="@@settings.general.language.help" /></legend>
      <select class="select lang" data-testid="language-select" aria-label="Language" i18n-aria-label="@@settings.general.language.selectLabel" (change)="language.choose($any($event.target).value)">
        @for (l of languages; track l.id) {
          <option [value]="l.id" [selected]="language.choice() === l.id" [attr.lang]="l.lang">{{ l.label }}</option>
        }
      </select>
      @if (language.reloadPending()) {
        <p class="pending" role="status" i18n="@@settings.general.language.pending">The new language is used after IMT restarts. <button class="link" type="button" (click)="language.reload()">Restart now</button></p>
      }
    </fieldset>
    <fieldset>
      <legend i18n="@@settings.general.timeZone">Time zone</legend>
      <div class="seg">
        @for (z of zones; track z.id) {
          <button class="btn" type="button" [class.active]="store.timezone() === z.id" [attr.aria-pressed]="store.timezone() === z.id" (click)="store.setTimezone(z.id)">{{ z.label }}</button>
        }
      </div>
    </fieldset>
    <fieldset>
      <legend i18n="@@settings.general.dateFormat">Date format <imt-help label="Date format" text="How times appear in the grid and the inspector. Readable: 14 Oct 10:41:22. ISO: 2026-10-14 10:41:22." i18n-label="@@settings.general.dateFormat.helpLabel" i18n-text="@@settings.general.dateFormat.help" /></legend>
      <div class="seg">
        @for (f of dateFormats; track f.id) {
          <button class="btn" type="button" [class.active]="settings.dateFormat() === f.id" [attr.aria-pressed]="settings.dateFormat() === f.id" (click)="settings.setDateFormat(f.id)">{{ f.label }}</button>
        }
      </div>
    </fieldset>
    <label class="row" i18n="@@settings.general.alwaysShowLifecycle">
      <input type="checkbox" [checked]="store.alwaysShowLifecycle()" (change)="store.setAlwaysShowLifecycle($any($event.target).checked)" />
      Always show start/stop markers
      <imt-help label="Start/stop markers" text="Keep process start and stop markers on the timeline even when level or text filters would hide them." i18n-label="@@settings.general.alwaysShowLifecycle.helpLabel" i18n-text="@@settings.general.alwaysShowLifecycle.help" />
    </label>
    <label class="row" i18n="@@settings.general.overviewAutoOpen">
      <input type="checkbox" data-testid="overview-auto-open" [checked]="overview.autoOpen()" (change)="overview.setAutoOpen($any($event.target).checked)" />
      Overview: show after every import
      <imt-help label="Overview after import" text="Open the Overview panel by itself each time files are opened. The same choice is in the Overview panel." i18n-label="@@settings.general.overviewAutoOpen.helpLabel" i18n-text="@@settings.general.overviewAutoOpen.help" />
    </label>
    <div>
      <button class="btn" type="button" (click)="dock.reset(); layoutReset.emit()" i18n="@@settings.general.resetLayout">Reset layout</button>
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 14px; }
    fieldset { border: 0; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
    legend { padding: 0 0 6px; font-size: var(--fs-xs); font-weight: 600; letter-spacing: 0.08em; text-transform: uppercase; color: var(--muted); display: flex; align-items: center; gap: 6px; }
    .seg { display: flex; gap: 4px; flex-wrap: wrap; }
    .seg .btn { text-transform: capitalize; }
    .row { display: flex; align-items: center; gap: 8px; color: var(--text-2); }
    .lang { align-self: flex-start; min-width: 200px; }
    .pending { margin: 0; color: var(--text-2); font-size: var(--fs-sm); }
    .link { border: 0; background: none; padding: 0; color: var(--text); cursor: pointer; text-decoration: underline; }
  `,
})
export class GeneralSettings {
  protected readonly store = inject(WorkspaceStore);
  protected readonly settings = inject(SettingsStore);
  protected readonly dock = inject(DockService);
  protected readonly overview = inject(OverviewPrefs);
  protected readonly language = inject(LanguageService);
  /** The layout was reset (the screen closes so the user sees it). */
  readonly layoutReset = output<void>();
  /** Language names are written in their own language (an endonym is never translated). */
  protected readonly languages: { id: LanguageChoice; label: string; lang: string | null }[] = [
    { id: 'system', label: $localize`:Language option, follow the operating system@@settings.general.language.system:System default`, lang: null },
    { id: 'en', label: 'English', lang: 'en' },
    { id: 'pseudo', label: $localize`:Language option, a generated test language with accented, longer text@@settings.general.language.pseudo:Pseudo`, lang: null },
  ];
  protected readonly zones: { id: TimeZoneMode; label: string }[] = [
    { id: 'utc', label: 'UTC' },
    { id: 'local', label: $localize`:Time zone option, the computer's time zone@@settings.general.timeZone.local:Local` },
  ];
  protected readonly dateFormats: { id: DateFormat; label: string }[] = [
    { id: 'readable', label: $localize`:Date format option@@settings.general.dateFormat.readable:Readable` },
    { id: 'iso', label: 'ISO' },
  ];
}
