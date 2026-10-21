import { ChangeDetectionStrategy, Component, computed, effect, inject, signal, untracked } from '@angular/core';
import { BundleContent, BundleEstimate, BundleOptions } from '../../core/api.types';
import { formatCount } from '../../core/format';
import { Platform } from '../../core/platform';
import { WorkspaceStore } from '../../core/workspace.store';
import { DIALOG_FORM_STYLES } from '../../context/dialog-styles';
import { UiStateRegistry } from '../../lifecycle/ui-state';
import { ProjectService, UNTITLED } from '../../projects/project.service';
import { baseName, errorMessage } from '../../tools/tool-errors';
import { DIALOG_REF } from '../../ui/dialog.service';
import { RedactionService } from '../redaction/redaction.service';
import { ReportStore } from '../report/report.store';
import { ShareBackend } from '../share-backend';
import { OverwriteGuard } from '../../ui/overwrite-guard';
import { JobFollow } from '../../jobs/job-follow';
import { formatBytes } from '../../shared/format/units';

const ESTIMATE_MS = 200;

/** A name usable as a file name. */
export function bundleFileName(name: string): string {
  return `${name.replace(/[\\/:*?"<>|]+/g, '_').trim() || 'bundle'}.imtz`;
}

/** Options and a live size estimate for a shareable `.imtz` bundle, then the save dialog. */
@Component({
  selector: 'imt-bundle-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <label class="lbl"><ng-container i18n="@@share.bundleDialog.name">Name</ng-container>&ngsp;<input class="input" [value]="name()" (input)="name.set($any($event.target).value)" aria-label="Bundle name" i18n-aria-label="@@share.bundleDialog.nameAria" /></label>
    <fieldset class="group">
      <legend i18n="@@share.bundleDialog.logs">Logs</legend>
      <label class="check"><input type="radio" name="content" value="raw" [checked]="content() === 'raw'" (change)="content.set('raw')" />&ngsp;<ng-container i18n="@@share.bundleDialog.contentRaw">The opened files as they are</ng-container></label>
      <label class="check"><input type="radio" name="content" value="events" [checked]="content() === 'events'" (change)="content.set('events')" />&ngsp;<ng-container i18n="@@share.bundleDialog.contentEvents">Only the events matching the current filter (CLEF)</ng-container></label>
    </fieldset>
    <label class="check"><input type="checkbox" [checked]="redact()" (change)="chooseRedact($any($event.target).checked)" />&ngsp;<ng-container i18n="@@share.bundleDialog.redact">Redact with the profile's rules</ng-container></label>
    <label class="check"><input type="checkbox" [disabled]="!reportCount()" [checked]="includeReports() && reportCount() > 0" (change)="includeReports.set($any($event.target).checked)" />&ngsp;<ng-container i18n="@@share.bundleDialog.includeReports">Include reports ({{ reportCount() }})</ng-container></label>
    @if (redact() && content() === 'raw') {
      <p class="status" i18n="@@share.bundleDialog.binaryNote">Binary files and archives cannot be redacted and are left out; bundle the filtered events to keep them.</p>
    }
    <div class="status" role="status" aria-live="polite">
      @if (estimate(); as e) {
        <ng-container i18n="@@share.bundleDialog.estimate">About <strong>{{ bytes(e.bytes) }}</strong> before compression · {{ fmt(e.files) }} files · {{ fmt(e.events) }} events</ng-container>
        @if (e.skipped.length) {
          <ul class="skipped">
            @for (s of e.skipped; track s.path) { <li><span class="mono">{{ base(s.path) }}</span>: {{ s.reason }}</li> }
          </ul>
        }
      } @else {
        <ng-container i18n="@@share.bundleDialog.estimating">Estimating…</ng-container>
      }
    </div>
    @if (error(); as e) { <p class="err" role="alert">{{ e }}</p> }
    <div class="foot">
      <button class="btn" type="button" (click)="ref.close()" i18n="@@share.bundleDialog.cancel">Cancel</button>
      <button class="btn primary" type="button" [disabled]="busy() || !name().trim()" (click)="save()">{{ saveLabel() }}</button>
    </div>
  `,
  styles: `
    ${DIALOG_FORM_STYLES}
    .group { border: 1px solid var(--border); border-radius: 6px; padding: 8px 10px; margin: 0; display: flex; flex-direction: column; gap: 6px; }
    legend { font-size: var(--fs-xs); color: var(--muted); padding: 0 4px; }
    .skipped { margin: 4px 0 0; padding-inline-start: 18px; }
  `,
})
export class BundleDialog {
  protected readonly ref = inject(DIALOG_REF);
  private readonly share = inject(ShareBackend);
  private readonly store = inject(WorkspaceStore);
  private readonly registry = inject(UiStateRegistry);
  private readonly reports = inject(ReportStore);
  private readonly platform = inject(Platform);
  private readonly overwrite = inject(OverwriteGuard);
  private readonly follow = inject(JobFollow);
  private readonly projects = inject(ProjectService);
  private readonly redaction = inject(RedactionService);

  protected readonly name = signal(this.projects.name() === UNTITLED ? 'IMT bundle' : this.projects.name());
  protected readonly content = signal<BundleContent>('raw');
  protected readonly redact = signal(false);
  /** The user ticked or unticked Redact: the profile default (loaded asynchronously) no longer applies. */
  private redactChosen = false;
  protected readonly includeReports = signal(true);
  protected readonly estimate = signal<BundleEstimate | null>(null);
  protected readonly busy = signal(false);
  protected readonly error = signal<string | null>(null);
  protected readonly saveLabel = computed(() => (this.busy() ? $localize`:@@share.bundleDialog.writing:Writing…` : $localize`:@@share.bundleDialog.save:Save bundle…`));
  protected readonly reportCount = computed(() => this.reports.reports().length);
  protected readonly fmt = formatCount;
  protected readonly bytes = formatBytes;
  protected readonly base = baseName;

  private readonly options = computed<BundleOptions>(() => ({
    name: this.name().trim(),
    content: this.content(),
    query: this.store.effectiveQuery(),
    redact: this.redact(),
    ui: null,
    reports: this.includeReports() ? this.reports.reports() : [],
  }));
  private timer: ReturnType<typeof setTimeout> | undefined;
  private seq = 0;

  constructor() {
    void this.redaction.onExportDefault().then((on) => this.redactChosen || this.redact.set(on));
    effect(() => {
      const o = this.options();
      untracked(() => {
        clearTimeout(this.timer);
        const mine = ++this.seq;
        this.estimate.set(null);
        this.timer = setTimeout(() => void this.runEstimate(mine, o), ESTIMATE_MS);
      });
    });
  }

  private async runEstimate(mine: number, o: BundleOptions): Promise<void> {
    try {
      const e = await this.share.estimateBundle(o);
      if (mine === this.seq) this.estimate.set(e);
    } catch (e) {
      if (mine === this.seq) this.error.set(errorMessage(e));
    }
  }

  protected chooseRedact(on: boolean): void {
    this.redactChosen = true;
    this.redact.set(on);
  }

  protected async save(): Promise<void> {
    this.busy.set(true);
    this.error.set(null);
    try {
      const options = { ...this.options(), ui: this.registry.serialize() };
      const path = await this.platform.pickSavePath($localize`:@@share.bundleDialog.pickTitle:Share bundle`, bundleFileName(options.name), [
        { name: $localize`:@@share.bundle.filterName:IMT bundle`, extensions: ['imtz'] },
      ]);
      if (!path) return;
      const job = await this.overwrite.write(path, (overwrite) => this.share.createBundle(options, path, overwrite));
      if (job === null) return;
      // Written as a job: its message ("Bundle written, redacted (5 values)") has "Show in folder".
      this.follow.started($localize`:@@share.bundleDialog.started:Writing the bundle ${baseName(path)}:file: — running in Jobs`);
      this.ref.close();
    } catch (e) {
      this.error.set(errorMessage(e));
    } finally {
      this.busy.set(false);
    }
  }
}
