import { ChangeDetectionStrategy, Component, computed, effect, inject, signal, untracked } from '@angular/core';
import { ExporterInfo, Query, RedactionSummary } from '../core/api.types';
import { DebriefBackend } from '../core/backend';
import { isUnsupported } from '../core/cmd-error';
import { formatCount } from '../core/format';
import { Platform } from '../core/platform';
import { EMPTY_QUERY, WorkspaceStore } from '../core/workspace.store';
import { DIALOG_REF } from '../ui/dialog.service';
import { HelpHint } from '../ui/help-hint';
import { exporterForPath } from './exporters';
import { RedactionService } from '../share/redaction/redaction.service';
import { RedactionSummaryView } from '../share/redaction/redaction-summary';
import { ShareBackend } from '../share/share-backend';
import { OverwriteGuard } from '../ui/overwrite-guard';
import { JobFollow } from '../jobs/job-follow';
import { DebriefSelect } from '../settings/themed-select';

/** What to export: the events matching the filters, the same without the time range, or everything. */
export type ExportScope = 'filter' | 'filterAnyTime' | 'all';

/** Builds the query for a scope from the current effective query. */
export function exportQuery(current: Query, scope: ExportScope): Query {
  switch (scope) {
    case 'all':
      return { ...EMPTY_QUERY, groupBy: current.groupBy };
    case 'filterAnyTime':
      return { ...current, from: null, to: null };
    default:
      return current;
  }
}

@Component({
  selector: 'debrief-export-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [HelpHint, RedactionSummaryView, DebriefSelect],
  template: `
    <div class="field">
      <span class="lbl" i18n="@@export.dialog.format">Format</span>
      <debrief-select id="exp-format" data-testid="exp-format" ariaLabel="Format" i18n-ariaLabel="@@export.dialog.format" [options]="formatOptions()" [value]="format()" (valueChange)="format.set($any($event))" />
    </div>

    <fieldset class="field">
      <legend><ng-container i18n="@@export.dialog.whichEvents">Which events</ng-container> <debrief-help label="Which events" i18n-label="@@export.dialog.whichEventsHelpLabel" i18n-text="@@export.dialog.whichEventsHelpText" text="'Current filter' exports exactly what the grid shows. 'All events' ignores every filter and the time range." /></legend>
      <label class="opt" i18n="@@export.dialog.scopeFilter"><input type="radio" name="scope" value="filter" [checked]="scope() === 'filter'" (change)="scope.set('filter')" /> Current filter</label>
      @if (hasRange()) {
        <label class="opt" i18n="@@export.dialog.scopeAnyTime"><input type="radio" name="scope" value="filterAnyTime" [checked]="scope() === 'filterAnyTime'" (change)="scope.set('filterAnyTime')" /> Current filter, any time (ignore the zoomed range)</label>
      }
      <label class="opt" i18n="@@export.dialog.scopeAll"><input type="radio" name="scope" value="all" [checked]="scope() === 'all'" (change)="scope.set('all')" /> All events</label>
    </fieldset>

    <fieldset class="field">
      <legend i18n="@@export.dialog.order">Order</legend>
      <label class="opt" i18n="@@export.dialog.oldestFirst"><input type="radio" name="order" value="asc" [checked]="!descending()" (change)="descending.set(false)" /> Oldest first</label>
      <label class="opt" i18n="@@export.dialog.newestFirst"><input type="radio" name="order" value="desc" [checked]="descending()" (change)="descending.set(true)" /> Newest first</label>
    </fieldset>

    <fieldset class="field">
      <legend><ng-container i18n="@@export.dialog.privacy">Privacy</ng-container> <debrief-help label="Redaction" i18n-label="@@export.dialog.redactionHelpLabel" i18n-text="@@export.dialog.redactionHelpText" text="Masks secrets and personal data (e-mail and IP addresses, tokens, passwords…) with the profile's [redact] rules. Hashes use a new salt for every export." /></legend>
      <label class="opt" i18n="@@export.dialog.redact"><input type="checkbox" [checked]="redact()" (change)="chooseRedact($any($event.target).checked)" /> Redact with the profile's rules</label>
      @if (redact()) {
        <debrief-redaction-summary [summary]="redaction()" [unavailable]="redactionUnavailable()" />
      }
    </fieldset>

    <p class="count" role="status" aria-live="polite">
      @if (count() === null) { <ng-container i18n="@@export.dialog.counting">Counting events…</ng-container> } @else { <ng-container i18n="@@export.dialog.willExport"><strong>{{ fmt(count()!) }}</strong> events will be exported.</ng-container> }
    </p>
    @if (error(); as e) { <p class="err" role="alert">{{ e }}</p> }

    <div class="footer">
      <button class="btn" type="button" (click)="ref.close()" i18n="@@export.dialog.cancel">Cancel</button>
      <button class="btn primary" type="button" [disabled]="busy() || !count()" (click)="export()">{{ busy() ? exportingLabel : exportLabel }}</button>
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 12px; }
    .field { display: flex; flex-direction: column; gap: 6px; border: 0; margin: 0; padding: 0; min-width: 0; }
    label, legend, .lbl { font-size: var(--fs-sm); color: var(--muted); display: flex; align-items: center; gap: 6px; padding: 0; }
    /* Stacked options stay 24px apart (WCAG 2.5.8 spacing for the 13px radios). */
    .opt { color: var(--text-2); font-size: var(--fs-md); min-height: 24px; }
    .count { margin: 0; color: var(--text-2); }
    .err { margin: 0; color: var(--danger-text); }
    .footer { display: flex; justify-content: flex-end; gap: 8px; }
  `,
})
export class ExportDialog {
  static readonly dialogId = 'export';
  protected readonly ref = inject(DIALOG_REF);
  private readonly store = inject(WorkspaceStore);
  private readonly backend = inject(DebriefBackend);
  private readonly platform = inject(Platform);
  private readonly share = inject(ShareBackend);
  private readonly overwrite = inject(OverwriteGuard);
  private readonly follow = inject(JobFollow);
  private readonly redactionService = inject(RedactionService);

  protected readonly exporters = computed<ExporterInfo[]>(() => this.store.summary()?.exporters ?? []);
  protected readonly formatOptions = computed(() => this.exporters().map((e) => ({ value: e.id, label: `${e.name} (.${e.extension})` })));
  protected readonly format = signal(this.exporters()[0]?.id ?? '');
  protected readonly scope = signal<ExportScope>('filter');
  protected readonly descending = signal(this.store.descending());
  protected readonly count = signal<number | null>(null);
  protected readonly busy = signal(false);
  protected readonly error = signal<string | null>(null);
  protected readonly fmt = formatCount;
  protected readonly exportingLabel = $localize`:@@export.dialog.exporting:Exporting…`;
  protected readonly exportLabel = $localize`:Start the export|Button@@export.dialog.export:Export…`;
  protected readonly hasRange = computed(() => this.store.hasRange());
  /** Redact the export (defaults to the profile's `[redact] on_export`). */
  protected readonly redact = signal(false);
  /** The user ticked or unticked Redact: the profile default (loaded asynchronously) no longer applies. */
  private redactChosen = false;
  protected readonly redaction = signal<RedactionSummary | null>(null);
  /** The engine cannot preview the redaction (yet): the export is still redacted. */
  protected readonly redactionUnavailable = signal(false);
  private redactionSeq = 0;

  private readonly query = computed(() => exportQuery(this.store.effectiveQuery(), this.scope()));
  private seq = 0;

  constructor() {
    effect(() => {
      const query = this.query();
      untracked(() => void this.recount(query));
    });
    void this.redactionService.onExportDefault().then((on) => this.redactChosen || this.redact.set(on));
    effect(() => {
      const query = this.query();
      if (!this.redact()) return;
      untracked(() => void this.previewRedaction(query));
    });
  }

  private async previewRedaction(query: Query): Promise<void> {
    const mine = ++this.redactionSeq;
    this.redaction.set(null);
    try {
      const summary = await this.share.redactionSummary({ query, samples: 2 });
      if (mine === this.redactionSeq) this.redaction.set(summary);
    } catch (e) {
      if (mine !== this.redactionSeq) return;
      if (isUnsupported(e)) this.redactionUnavailable.set(true);
      else this.error.set(e instanceof Error ? e.message : String(e));
    }
  }

  private async recount(query: Query): Promise<void> {
    const mine = ++this.seq;
    this.count.set(null);
    try {
      const page = await this.backend.queryPage({ query, offset: 0, limit: 1, descending: false, anchor: null });
      if (mine === this.seq) this.count.set(page.total);
    } catch (e) {
      if (mine === this.seq) this.error.set(String(e));
    }
  }

  protected chooseRedact(on: boolean): void {
    this.redactChosen = true;
    this.redact.set(on);
  }

  protected async export(): Promise<void> {
    const exporters = this.exporters();
    const chosen = exporters.find((e) => e.id === this.format()) ?? exporters[0];
    if (!chosen) return;
    this.busy.set(true);
    this.error.set(null);
    try {
      const path = await this.platform.pickExportPath(`events.${chosen.extension}`, [chosen, ...exporters.filter((e) => e !== chosen)]);
      if (!path) return;
      const format = exporterForPath(path, exporters)?.id ?? chosen.id;
      const job = await this.overwrite.write(path, (overwrite) =>
        this.backend.exportEvents({ query: this.query(), format, path, descending: this.descending(), redact: this.redact(), overwrite }),
      );
      if (job === null) return;
      // The export runs as a job: its message says how many events went out, with "Show in folder".
      const file = path.split(/[\\/]/).pop() ?? path;
      this.follow.started($localize`:@@export.dialog.started:Exporting to ${file}:file: — running in Jobs`);
      this.ref.close();
    } catch (e) {
      this.error.set(e instanceof Error ? e.message : String(e));
    } finally {
      this.busy.set(false);
    }
  }
}
