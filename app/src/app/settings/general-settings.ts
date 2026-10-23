import { ChangeDetectionStrategy, Component, inject, output } from '@angular/core';
import { DateFormat, TimeZoneMode } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { DockService } from '../dock/dock.service';
import { LanguageService } from '../i18n/language.service';
import { LanguageChoice } from '../i18n/locale';
import { HomePrefs } from '../home/home-prefs';
import { Switch } from '../shared/switch';
import { DebriefSelect, SelectOption } from './themed-select';
import { THEMED_CONTROLS } from './themed-controls';
import { HelpHint } from '../ui/help-hint';
import { SettingsStore } from './settings.store';

/**
 * UI-only preferences (kept in the browser): language (applied after a restart), time zone, date format, lifecycle
 * markers, Home, layout. Everything but the language applies immediately. The look is in Appearance.
 */
@Component({
  selector: 'debrief-general-settings',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [HelpHint, DebriefSelect, Switch],
  template: `
    <fieldset data-testid="language-setting">
      <legend i18n="Setting for the UI language@@settings.general.language">Language <debrief-help label="Language" text="The language of menus, dialogs and messages. Log text and file contents are never translated. The new language is used after Debrief restarts." i18n-label="@@settings.general.language.helpLabel" i18n-text="@@settings.general.language.help" /></legend>
      <debrief-select class="lang" data-testid="language-select" ariaLabel="Language" i18n-ariaLabel="@@settings.general.language.selectLabel" [options]="languages" [value]="language.choice()" (valueChange)="language.choose($any($event))" />
      @if (language.reloadPending()) {
        <p class="pending" role="status" i18n="@@settings.general.language.pending">The new language is used after Debrief restarts. <button class="link" type="button" (click)="language.reload()">Restart now</button></p>
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
      <legend i18n="@@settings.general.dateFormat">Date format <debrief-help label="Date format" text="How times appear in the grid and the inspector. Readable: 14 Oct 10:41:22. ISO: 2026-10-14 10:41:22." i18n-label="@@settings.general.dateFormat.helpLabel" i18n-text="@@settings.general.dateFormat.help" /></legend>
      <div class="seg">
        @for (f of dateFormats; track f.id) {
          <button class="btn" type="button" [class.active]="settings.dateFormat() === f.id" [attr.aria-pressed]="settings.dateFormat() === f.id" (click)="settings.setDateFormat(f.id)">{{ f.label }}</button>
        }
      </div>
    </fieldset>
    <div class="row">
      <debrief-switch [checked]="store.alwaysShowLifecycle()" (checkedChange)="store.setAlwaysShowLifecycle($event)"><ng-container i18n="@@settings.general.alwaysShowLifecycle">Always show start/stop markers</ng-container></debrief-switch>
      <debrief-help label="Start/stop markers" text="Keep process start and stop markers on the timeline even when level or text filters would hide them." i18n-label="@@settings.general.alwaysShowLifecycle.helpLabel" i18n-text="@@settings.general.alwaysShowLifecycle.help" />
    </div>
    <div class="row">
      <debrief-switch inputId="home-auto-open" [checked]="home.autoOpen()" (checkedChange)="home.setAutoOpen($event)"><ng-container i18n="@@settings.general.homeAutoOpen">Home: show when files or a project open</ng-container></debrief-switch>
      <debrief-help label="Home after import" text="Open Home as a document tab by itself each time files or a project are opened. The same choice is in Home." i18n-label="@@settings.general.homeAutoOpen.helpLabel" i18n-text="@@settings.general.homeAutoOpen.help" />
    </div>
    <div>
      <button class="btn" type="button" (click)="dock.reset(); layoutReset.emit()" i18n="@@settings.general.resetLayout">Reset layout</button>
    </div>
  `,
  styles: [
    THEMED_CONTROLS,
    `
    :host { display: flex; flex-direction: column; gap: 14px; }
    fieldset { border: 0; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
    legend { padding: 0 0 6px; font-size: var(--fs-sm); font-weight: 600; color: var(--muted); display: flex; align-items: center; gap: 6px; }
    .seg { display: flex; gap: 4px; flex-wrap: wrap; }
    .seg .btn { text-transform: capitalize; }
    .row { display: flex; align-items: center; gap: 8px; color: var(--text-2); }
    .lang { align-self: flex-start; min-width: 200px; }
    .pending { margin: 0; color: var(--text-2); font-size: var(--fs-sm); }
    .link { border: 0; background: none; padding: 0; color: var(--text); cursor: pointer; text-decoration: underline; }
  `,
  ],
})
export class GeneralSettings {
  protected readonly store = inject(WorkspaceStore);
  protected readonly settings = inject(SettingsStore);
  protected readonly dock = inject(DockService);
  protected readonly home = inject(HomePrefs);
  protected readonly language = inject(LanguageService);
  /** The layout was reset (the screen closes so the user sees it). */
  readonly layoutReset = output<void>();
  /** Language names are written in their own language (an endonym is never translated). */
  protected readonly languages: (SelectOption<LanguageChoice> & { lang: string | null })[] = [
    { value: 'system', label: $localize`:Language option, follow the operating system@@settings.general.language.system:System default`, lang: null },
    { value: 'en', label: 'English', lang: 'en' },
    { value: 'pseudo', label: $localize`:Language option, a generated test language with accented, longer text@@settings.general.language.pseudo:Pseudo`, lang: null },
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
