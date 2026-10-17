import { ChangeDetectionStrategy, Component, inject, Injectable } from '@angular/core';
import { DateFormat, TimeZoneMode } from '../core/format';
import { ThemeMode, WorkspaceStore } from '../core/workspace.store';
import { DockService } from '../dock/dock.service';
import { DIALOG_REF, DialogService } from '../ui/dialog.service';
import { HelpHint } from '../ui/help-hint';
import { GridDensity, SettingsStore } from './settings.store';

/** Opens the settings dialog (gear in the activity bar, "Open settings" command). */
@Injectable({ providedIn: 'root' })
export class SettingsDialogService {
  private readonly dialogs = inject(DialogService);
  open(): void {
    this.dialogs.open(SettingsDialog, { title: 'Settings', width: 460 });
  }
}

@Component({
  selector: 'imt-settings-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [HelpHint],
  template: `
    <fieldset>
      <legend>Theme</legend>
      <div class="seg">
        @for (t of themes; track t) {
          <button class="btn" type="button" [class.active]="store.theme() === t" [attr.aria-pressed]="store.theme() === t" (click)="store.setTheme(t)">{{ t }}</button>
        }
      </div>
    </fieldset>
    <fieldset>
      <legend>Time zone</legend>
      <div class="seg">
        @for (z of zones; track z.id) {
          <button class="btn" type="button" [class.active]="store.timezone() === z.id" [attr.aria-pressed]="store.timezone() === z.id" (click)="store.setTimezone(z.id)">{{ z.label }}</button>
        }
      </div>
    </fieldset>
    <fieldset>
      <legend>Date format <imt-help label="Date format" text="How times appear in the grid and the inspector. Readable: 14 Oct 10:41:22. ISO: 2026-10-14 10:41:22." /></legend>
      <div class="seg">
        @for (f of dateFormats; track f.id) {
          <button class="btn" type="button" [class.active]="settings.dateFormat() === f.id" [attr.aria-pressed]="settings.dateFormat() === f.id" (click)="settings.setDateFormat(f.id)">{{ f.label }}</button>
        }
      </div>
    </fieldset>
    <fieldset>
      <legend>Grid density</legend>
      <div class="seg">
        @for (d of densities; track d) {
          <button class="btn" type="button" [class.active]="settings.density() === d" [attr.aria-pressed]="settings.density() === d" (click)="settings.setDensity(d)">{{ d }}</button>
        }
      </div>
    </fieldset>
    <label class="row">
      <input type="checkbox" [checked]="store.alwaysShowLifecycle()" (change)="store.setAlwaysShowLifecycle($any($event.target).checked)" />
      Always show start/stop markers
      <imt-help label="Start/stop markers" text="Keep process start and stop markers on the timeline even when level or text filters would hide them." />
    </label>
    <div class="footer">
      <button class="btn" type="button" (click)="dock.reset(); ref.close()">Reset layout</button>
      <span class="spacer"></span>
      <button class="btn primary" type="button" (click)="ref.close()">Done</button>
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 14px; }
    fieldset { border: 0; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
    legend { padding: 0 0 6px; font-size: 11px; font-weight: 600; letter-spacing: 0.08em; text-transform: uppercase; color: var(--muted); display: flex; align-items: center; gap: 6px; }
    .seg { display: flex; gap: 4px; flex-wrap: wrap; }
    .seg .btn { text-transform: capitalize; }
    .row { display: flex; align-items: center; gap: 8px; color: var(--text-2); }
    .footer { display: flex; gap: 8px; align-items: center; }
    .spacer { flex: 1; }
  `,
})
export class SettingsDialog {
  protected readonly ref = inject(DIALOG_REF);
  protected readonly store = inject(WorkspaceStore);
  protected readonly settings = inject(SettingsStore);
  protected readonly dock = inject(DockService);
  protected readonly themes: ThemeMode[] = ['system', 'dark', 'light'];
  protected readonly zones: { id: TimeZoneMode; label: string }[] = [
    { id: 'utc', label: 'UTC' },
    { id: 'local', label: 'Local' },
  ];
  protected readonly dateFormats: { id: DateFormat; label: string }[] = [
    { id: 'readable', label: 'Readable' },
    { id: 'iso', label: 'ISO' },
  ];
  protected readonly densities: GridDensity[] = ['comfortable', 'compact'];
}
