import { ChangeDetectionStrategy, Component, computed, effect, inject, signal, untracked } from '@angular/core';
import { ExportTheme, RedactionPreview, Report, ReportFormat } from '../../core/api.types';
import { Platform } from '../../core/platform';
import { WorkspaceStore } from '../../core/workspace.store';
import { ProfileEdits } from '../../profile/profile-edits';
import { errorMessage } from '../../tools/tool-errors';
import { DIALOG_REF, DialogService } from '../../ui/dialog.service';
import { ShareBackend } from '../share-backend';
import { fileStem } from '../visuals/visual-export.service';
import { ReportStore } from './report.store';
import { reportEventIds, reportText } from './report-text';
import { OverwriteGuard } from '../../ui/overwrite-guard';
import { JobFollow } from '../../jobs/job-follow';

/**
 * Exports the active report as self-contained HTML or PDF. Redaction (the profile's `[redact]` rules,
 * on by default when the profile says `on_export = true`) is applied by the engine at export; the dialog
 * previews what the rules match in the report's own text first.
 */
@Component({
  selector: 'imt-report-export-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="field">
      <label for="rep-format" i18n="@@share.reportExport.format">Format</label>
      <select id="rep-format" class="select" [value]="format()" (change)="format.set($any($event.target).value)">
        <option value="html" i18n="@@share.reportExport.formatHtml">HTML (self-contained web page)</option>
        <option value="pdf">PDF</option>
      </select>
    </div>
    <fieldset class="field">
      <legend i18n="@@share.reportExport.colours">Colours</legend>
      <label class="opt" i18n="@@share.reportExport.light"><input type="radio" name="rep-theme" value="light" [checked]="theme() === 'light'" (change)="theme.set('light')" /> Light</label>
      <label class="opt" i18n="@@share.reportExport.dark"><input type="radio" name="rep-theme" value="dark" [checked]="theme() === 'dark'" (change)="theme.set('dark')" /> Dark</label>
    </fieldset>
    <label class="opt" i18n="@@share.reportExport.redact"><input type="checkbox" [checked]="redact()" (change)="setRedact($any($event.target).checked)" /> Redact with the profile's rules</label>
    @if (redact()) {
      <div class="preview" role="status" aria-live="polite" aria-label="Redaction preview" i18n-aria-label="@@share.reportExport.previewAria">
        @if (preview(); as p) {
          @if (p.error) {
            <span class="err" i18n="@@share.reportExport.rulesInvalid">The redaction rules do not compile: {{ p.error }}</span>
          } @else {
            <span>{{ matchText(p) }}</span>
            @if (events()) { <span class="faint" i18n="@@share.reportExport.eventsRedacted"> {events(), plural, =1 {The {{ events() }} referenced event are redacted when exported.} other {The {{ events() }} referenced events are redacted when exported.}}</span> }
          }
        } @else {
          <span class="faint" i18n="@@share.reportExport.checking">Checking what will be redacted…</span>
        }
      </div>
    }
    @if (error(); as e) { <p class="err" role="alert">{{ e }}</p> }
    <div class="footer">
      <button class="btn" type="button" (click)="ref.close()" i18n="@@share.reportExport.cancel">Cancel</button>
      <button class="btn primary" type="button" [disabled]="busy() || !report()" (click)="export()">{{ exportLabel() }}</button>
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 12px; }
    .field { display: flex; flex-direction: column; gap: 6px; border: 0; margin: 0; padding: 0; min-width: 0; }
    label, legend { font-size: var(--fs-sm); color: var(--muted); display: flex; align-items: center; gap: 6px; padding: 0; }
    /* Stacked options stay 24px apart (WCAG 2.5.8 spacing for the 13px radios and checkbox). */
    .opt { color: var(--text-2); font-size: var(--fs-md); min-height: 24px; }
    .preview { font-size: var(--fs-sm); color: var(--text-2); padding: 8px; border: 1px dashed var(--border-3); border-radius: 6px; }
    .err { margin: 0; color: var(--danger-text); }
    .footer { display: flex; justify-content: flex-end; gap: 8px; }
  `,
})
export class ReportExportDialog {
  protected readonly ref = inject(DIALOG_REF);
  private readonly reports = inject(ReportStore);
  private readonly backend = inject(ShareBackend);
  private readonly platform = inject(Platform);
  private readonly overwrite = inject(OverwriteGuard);
  private readonly follow = inject(JobFollow);
  private readonly store = inject(WorkspaceStore);
  private readonly profile = inject(ProfileEdits);

  protected readonly report = computed<Report | null>(() => this.reports.active());
  protected readonly format = signal<ReportFormat>('html');
  protected readonly theme = signal<ExportTheme>(document.documentElement.getAttribute('data-scheme') === 'dark' ? 'dark' : 'light');
  protected readonly redact = signal(false);
  protected readonly preview = signal<RedactionPreview | null>(null);
  protected readonly busy = signal(false);
  protected readonly error = signal<string | null>(null);
  protected readonly exportLabel = computed(() => (this.busy() ? $localize`:@@share.reportExport.exporting:Exporting…` : $localize`:@@share.reportExport.export:Export…`));
  protected readonly events = computed(() => {
    const r = this.report();
    return r ? reportEventIds(r).length : 0;
  });

  constructor() {
    void this.profile
      .current()
      .then(({ model }) => {
        // The profile's default applies unless the user already chose.
        if (!this.touched) this.redact.set((model['redact'] as { on_export?: unknown } | undefined)?.on_export === true);
      })
      .catch(() => undefined);
    let seq = 0;
    effect(() => {
      const on = this.redact();
      const r = this.report();
      untracked(() => {
        const mine = ++seq;
        this.preview.set(null);
        if (!on || !r) return;
        this.backend
          .previewRedaction({ sample: reportText(r) })
          .then((p) => mine === seq && this.preview.set(p))
          .catch((e) => mine === seq && this.preview.set({ before: '', after: '', hits: [], error: errorMessage(e) }));
      });
    });
  }

  private touched = false;

  protected setRedact(on: boolean): void {
    this.touched = true;
    this.redact.set(on);
  }

  protected matched(p: RedactionPreview): number {
    return p.hits.reduce((n, h) => n + h.count, 0);
  }

  protected matchText(p: RedactionPreview): string {
    return this.matched(p)
      ? $localize`:@@share.reportExport.matched:In the report text: ${this.summary(p)}:summary:.`
      : $localize`:@@share.reportExport.noMatch:Nothing in the report text matches the rules.`;
  }

  protected summary(p: RedactionPreview): string {
    return p.hits
      .filter((h) => h.count > 0)
      .map((h) => `${h.count} × ${h.rule}`)
      .join(', ');
  }

  protected async export(): Promise<void> {
    const report = this.report();
    if (!report) return;
    this.busy.set(true);
    this.error.set(null);
    try {
      const format = this.format();
      const name = `${fileStem(report.title || 'report')}.${format}`;
      const path = await this.platform.pickSavePath($localize`:@@share.reportExport.pickTitle:Export report`, name, [
        { name: format === 'pdf' ? $localize`:@@share.reportExport.filterPdf:PDF document` : $localize`:@@share.reportExport.filterWeb:Web page`, extensions: [format] },
      ]);
      if (!path) return;
      const job = await this.overwrite.write(path, (overwrite) => this.backend.exportReport({ report, format, path, redact: this.redact(), theme: this.theme(), overwrite }));
      if (job === null) return;
      // Written as a job: its message says what was redacted, with "Show in folder".
      const file = path.split(/[\\/]/).pop() ?? path;
      this.follow.started($localize`:@@share.reportExport.started:Exporting the report to ${file}:file: — running in Jobs`);
      this.ref.close();
    } catch (e) {
      this.error.set(errorMessage(e));
    } finally {
      this.busy.set(false);
    }
  }
}

/** Opens the export dialog for the active report. */
export function openReportExport(dialogs: DialogService): Promise<unknown> {
  return dialogs.open(ReportExportDialog, { title: $localize`:@@share.reportExport.dialogTitle:Export report`, width: 520 }).closed;
}
