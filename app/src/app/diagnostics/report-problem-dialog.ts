import { ChangeDetectionStrategy, Component, computed, inject, OnInit, signal } from '@angular/core';
import { ProblemReportPreview, ReportEntry } from '../core/api.types';
import { describeError } from '../core/logger';
import { DIALOG_REF } from '../ui/dialog.service';
import { OverwriteGuard } from '../ui/overwrite-guard';
import { ExportTarget } from './export-target';
import { SnapshotBackend } from './snapshot-backend';
import { localizeDecimal } from '../i18n/locale';

/** Name the save dialog suggests. */
export const REPORT_FILE_NAME = 'imt-problem-report.zip';
/** Longest description the engine keeps (bytes); the field stops there. */
export const MAX_DESCRIPTION = 10 * 1024;

/** Human-readable size. */
export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${localizeDecimal((n / 1024).toFixed(1))} KB`;
  return `${localizeDecimal((n / (1024 * 1024)).toFixed(1))} MB`;
}

/**
 * Help ▸ Report a problem: lists what the zip will contain (activity, logs, snapshots, steps to reproduce,
 * system info) for the user to review, takes a description, and saves the zip where the user chooses.
 * Nothing is sent anywhere.
 */
@Component({
  selector: 'imt-report-problem-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <p class="lead" i18n="@@diagnostics.reportDialog.lead">IMT puts what is needed to diagnose the problem into one zip file that you save and attach to your report. Nothing is sent anywhere.</p>
    <label class="field">
      <span i18n="@@diagnostics.reportDialog.whatHappened">What happened? What did you expect?</span>
      <textarea
        rows="4"
        [maxLength]="maxDescription"
        [value]="description()"
        (input)="description.set($any($event.target).value)"
        aria-describedby="report-description-hint"
        data-testid="report-description"
      ></textarea>
      <small id="report-description-hint" i18n="@@diagnostics.reportDialog.descriptionHint">Optional. Do not paste passwords or keys.</small>
    </label>
    @if (loading()) {
      <p class="muted" role="status" i18n="@@diagnostics.reportDialog.collecting">Collecting…</p>
    }
    @if (preview(); as p) {
      <section aria-labelledby="report-contents-title">
        <h3 id="report-contents-title" i18n="@@diagnostics.reportDialog.contents">Contents ({{ entries().length }} files, {{ total() }})</h3>
        <ul class="files" data-testid="report-contents">
          @for (e of entries(); track e.name) {
            <li><span class="mono name">{{ e.name }}</span><span class="size">{{ size(e) }}</span><span class="sum">{{ e.summary }}</span></li>
          }
        </ul>
      </section>
      <section aria-labelledby="report-steps-title">
        <h3 id="report-steps-title" i18n="@@diagnostics.reportDialog.steps">Steps to reproduce</h3>
        @if (p.steps.length) {
          <ol class="steps" data-testid="report-steps">
            @for (s of p.steps; track $index) {
              <li>{{ stripNumber(s) }}</li>
            }
          </ol>
        } @else {
          <p class="muted" data-testid="report-steps" i18n="@@diagnostics.reportDialog.noActivity">No recent activity was recorded.</p>
        }
      </section>
    }
    @if (error(); as e) {
      <div class="msg err" role="alert">{{ e }}</div>
    }
    @if (saved(); as s) {
      <div class="msg" role="status" data-testid="report-saved">{{ s }}</div>
    }
    <div class="foot">
      <button type="button" class="btn ghost" (click)="ref.close()">{{ saved() ? closeLabel : cancelLabel }}</button>
      <button type="button" class="btn primary" [disabled]="busy() || !preview()" (click)="save()" data-testid="report-save" i18n="@@diagnostics.reportDialog.save">Save report…</button>
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 10px; }
    .lead, .muted { margin: 0; color: var(--text-2); }
    .field { display: flex; flex-direction: column; gap: 4px; }
    .field small { color: var(--muted); }
    textarea { resize: vertical; font: inherit; }
    h3 { margin: 4px 0; font-size: 13px; }
    .files { list-style: none; margin: 0; padding: 0; max-height: 180px; overflow: auto; display: grid; grid-template-columns: minmax(0, max-content) max-content 1fr; gap: 2px 10px; font-size: 12px; }
    .files li { display: contents; }
    .name { overflow-wrap: anywhere; }
    .size { color: var(--text-2); text-align: end; }
    .sum { color: var(--text-2); }
    .steps { margin: 0; padding-inline-start: 22px; max-height: 160px; overflow: auto; font-size: 12px; }
    .msg { padding: 6px 10px; border: 1px solid var(--border-2); border-radius: 6px; background: var(--bg-3); overflow-wrap: anywhere; }
    .msg.err { border-color: var(--lvl-error); color: var(--danger-text); }
    .foot { display: flex; justify-content: flex-end; gap: 8px; }
  `,
})
export class ReportProblemDialog implements OnInit {
  protected readonly ref = inject(DIALOG_REF);
  private readonly backend = inject(SnapshotBackend);
  private readonly target = inject(ExportTarget);
  private readonly overwrite = inject(OverwriteGuard);
  protected readonly maxDescription = MAX_DESCRIPTION;
  protected readonly closeLabel = $localize`:Close the dialog|Button@@diagnostics.reportDialog.close:Close`;
  protected readonly cancelLabel = $localize`:@@diagnostics.reportDialog.cancel:Cancel`;

  readonly description = signal('');
  readonly preview = signal<ProblemReportPreview | null>(null);
  readonly loading = signal(false);
  readonly busy = signal(false);
  readonly error = signal<string | null>(null);
  readonly saved = signal<string | null>(null);

  /** The preview's files, with the description's size as typed. */
  protected readonly entries = computed<ReportEntry[]>(() => {
    const bytes = new TextEncoder().encode(this.description().trim()).length;
    return (this.preview()?.entries ?? []).map((e) => (e.name === 'description.txt' ? { ...e, bytes, summary: bytes ? $localize`:@@diagnostics.reportDialog.yourDescription:Your description` : $localize`:@@diagnostics.reportDialog.noDescription:No description` } : e));
  });
  protected readonly total = computed(() => formatBytes(this.entries().reduce((n, e) => n + e.bytes, 0)));

  ngOnInit(): void {
    void this.load();
  }

  async load(): Promise<void> {
    this.loading.set(true);
    this.error.set(null);
    try {
      this.preview.set(await this.backend.reportPreview(this.description()));
    } catch (e) {
      this.error.set($localize`:@@diagnostics.reportDialog.collectFailed:Could not collect the report: ${describeError(e)}:error:`);
    }
    this.loading.set(false);
  }

  /** Asks where to save, then writes the zip (asking before replacing a file). */
  async save(): Promise<void> {
    this.busy.set(true);
    this.error.set(null);
    this.saved.set(null);
    try {
      const path = await this.target.pickZip(REPORT_FILE_NAME, $localize`:@@diagnostics.reportDialog.saveTitle:Save problem report`);
      if (path) {
        const files = await this.overwrite.write(path, (replace) => this.backend.saveReport(path, this.description(), replace));
        if (files !== null) this.saved.set($localize`:@@diagnostics.reportDialog.saved:Saved ${files}:count: files to ${path}:path:. Attach it to your report.`);
      }
    } catch (e) {
      this.error.set($localize`:@@diagnostics.reportDialog.saveFailed:Could not save the report: ${describeError(e)}:error:`);
    }
    this.busy.set(false);
  }

  protected size(e: ReportEntry): string {
    return formatBytes(e.bytes);
  }

  /** The engine numbers its steps; the list numbers them again. */
  protected stripNumber(step: string): string {
    return step.replace(/^\d+\.\s*/, '');
  }
}
