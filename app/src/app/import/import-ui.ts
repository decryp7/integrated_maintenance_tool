import { formatCount } from '../shared/format/units';
import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import { Icon } from '../shared/icon';
import { detailText, fileName, importStatusText, percent, progressLine } from './import-format';
import { ImportProgressService } from './import-progress.service';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { JOBS_VIEW } from '../jobs/job-viewer';
import { WelcomeService } from '../welcome/welcome';

/** Shared view model of the progress widgets. */
function view(svc: ImportProgressService) {
  return computed(() => {
    const p = svc.progress();
    if (!p) return null;
    // Finding files has no total: the bar is indeterminate (animated) while the number grows.
    const counted = p.phase !== 'finding' && p.phase !== 'finishing' && p.total > 0;
    const indeterminate = p.phase === 'finding' || p.phase === 'finishing' || (p.phase === 'importing' && p.total <= 0);
    return {
      percent: percent(p),
      line: counted ? progressLine(p) : importStatusText(p),
      status: importStatusText(p),
      file: fileName(p.current),
      path: p.current ?? '',
      detail: detailText(p),
      counted,
      indeterminate,
    };
  });
}

/**
 * The import's one progress surface (over a workspace): bar, "Scanning for files…" until the engine counted them, then
 * files, bytes, ETA and the current file, and Cancel. The text opens the import's job in Jobs.
 */
@Component({
  selector: 'debrief-import-status',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (vm(); as v) {
      <span class="wrap" data-testid="import-status">
        <span
          class="bar"
          [class.indeterminate]="v.indeterminate"
          role="progressbar"
          aria-label="Import progress"
          i18n-aria-label="@@import.ui.progress"
          aria-valuemin="0"
          aria-valuemax="100"
          [attr.aria-valuenow]="v.counted ? v.percent : null"
          [attr.aria-valuetext]="v.line"
        ><span class="fill" [style.width.%]="v.indeterminate ? 100 : v.percent"></span></span>
        <button type="button" class="open-job" data-testid="import-status-open" [title]="openJobTitle" (click)="ctx.showView(jobsView)">
          <span>{{ v.status }}</span>
          @if (v.file) { <span class="file" [title]="v.path">{{ v.file }}</span> }
        </button>
        <button type="button" class="cancel" [disabled]="svc.cancelling()" (click)="svc.cancel()">{{ svc.cancelling() ? cancellingLabel : cancelLabel }}</button>
      </span>
    }
  `,
  styles: `
    :host { display: contents; }
    /* Never shrinks: in a crowded status bar the Cancel button would otherwise spill over its neighbours. */
    .wrap { display: inline-flex; align-items: center; gap: 8px; flex: none; }
    .bar { width: 80px; height: 6px; border-radius: 3px; background: var(--border-2); overflow: hidden; flex: none; }
    .fill { display: block; height: 100%; background: var(--accent); transition: width 0.2s; }
    .open-job { all: unset; display: inline-flex; align-items: center; gap: 8px; cursor: pointer; }
    .open-job:hover { text-decoration: underline; }
    .open-job:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 1px; }
    .file { max-width: 180px; overflow: hidden; text-overflow: ellipsis; font-family: var(--font-mono); font-size: var(--fs-xs); }
    .cancel { height: 18px; padding: 0 8px; border-radius: 9px; border: 1px solid currentColor; background: transparent; color: inherit; cursor: pointer; font-size: var(--fs-xs); font-weight: 600; }
    .cancel:hover:not(:disabled) { background: var(--bg-hover); }
    .bar.indeterminate .fill { background: repeating-linear-gradient(90deg, var(--accent) 0 12px, var(--bg-3) 12px 24px); background-size: 48px 100%; animation: import-slide 0.8s linear infinite; }
    @keyframes import-slide { to { background-position: 48px 0; } }
    @media (prefers-reduced-motion: reduce) { .fill { transition: none; } .bar.indeterminate .fill { animation: none; } }
  `,
})
export class ImportStatus {
  protected readonly svc = inject(ImportProgressService);
  protected readonly ctx = inject(COMMAND_CONTEXT);
  protected readonly jobsView = JOBS_VIEW;
  protected readonly vm = view(this.svc);
  protected readonly openJobTitle = $localize`:@@import.ui.openJob:Show the import in Jobs`;
  protected readonly cancellingLabel = $localize`:@@import.ui.cancelling:Cancelling…`;
  protected readonly cancelLabel = $localize`:Cancel the import|Button@@import.ui.cancel:Cancel`;
}

/**
 * A card for long imports on the welcome page, where there is nothing else to look at; over a workspace the status
 * bar is the only progress surface (the card would cover the Explorer and the grid).
 */
@Component({
  selector: 'debrief-import-toast',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  template: `
    @if (svc.long() && welcome.visible()) {
      @if (vm(); as v) {
        <div class="toast" data-debrief-overlay role="status" aria-label="Import in progress" i18n-aria-label="@@import.ui.inProgress" data-testid="import-toast">
          <div class="head">
            <debrief-icon name="refresh" [size]="14" />
            <strong i18n="@@import.ui.heading">Importing…</strong>
            <span class="spacer"></span>
            <button type="button" class="btn" [disabled]="svc.cancelling()" (click)="svc.cancel()">{{ svc.cancelling() ? cancellingLabel : cancelImportLabel }}</button>
          </div>
          <div class="bar" [class.indeterminate]="v.indeterminate" aria-hidden="true"><span class="fill" [style.width.%]="v.indeterminate ? 100 : v.percent"></span></div>
          <div class="line" data-testid="import-line">{{ v.line }}</div>
          @if (v.detail) { <div class="file" [title]="v.path">{{ v.detail }}</div> }
          <div class="faint small" i18n="@@import.ui.keepWorking">You can keep working; files already read stay open if you cancel.</div>
        </div>
      }
    }
  `,
  styles: `
    .toast { position: fixed; left: 16px; bottom: 36px; z-index: 100; width: 360px; display: flex; flex-direction: column; gap: 6px; padding: 10px 12px; border-radius: 8px; background: var(--tooltip-bg); border: 1px solid var(--border-3); box-shadow: var(--shadow); color: var(--text); font-size: var(--fs-sm); }
    .head { display: flex; align-items: center; gap: 8px; }
    .head debrief-icon { color: var(--accent-text); }
    .spacer { flex: 1; }
    .bar { height: 6px; border-radius: 3px; background: var(--bg-3); overflow: hidden; }
    .fill { display: block; height: 100%; background: var(--accent); transition: width 0.2s; }
    .line { color: var(--text-2); }
    .file { font-family: var(--font-mono); font-size: var(--fs-xs); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .small { font-size: var(--fs-xs); }
    .bar.indeterminate .fill { background: repeating-linear-gradient(90deg, var(--accent) 0 12px, var(--bg-3) 12px 24px); background-size: 48px 100%; animation: import-slide 0.8s linear infinite; }
    @keyframes import-slide { to { background-position: 48px 0; } }
    @media (prefers-reduced-motion: reduce) { .fill { transition: none; } .bar.indeterminate .fill { animation: none; } }
  `,
})
export class ImportToast {
  protected readonly svc = inject(ImportProgressService);
  protected readonly welcome = inject(WelcomeService);
  protected readonly vm = view(this.svc);
  protected readonly cancellingLabel = $localize`:@@import.ui.cancelling:Cancelling…`;
  protected readonly cancelImportLabel = $localize`:@@import.ui.cancelImport:Cancel import`;
}

/** Shown after a cancelled import: what was read, and how to read the rest. */
@Component({
  selector: 'debrief-partial-import-banner',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  host: { style: 'display: contents' },
  template: `
    @if (svc.partial(); as p) {
      <div class="banner" role="status" data-testid="partial-import-banner">
        <debrief-icon name="alert" [size]="14" />
        <span class="msg" i18n="@@import.ui.partial"><strong>Partially imported:</strong> {{ fmt(p.done) }} of {{ fmt(p.total) }} files were read before you cancelled. The rest are listed as not imported in the Explorer.</span>
        <button type="button" class="btn primary" (click)="svc.reimportAll()" i18n="@@import.ui.reimportAll">Re-import all</button>
        <button type="button" class="btn ghost" (click)="svc.dismissPartial()" i18n="@@import.ui.dismiss">Dismiss</button>
      </div>
    }
  `,
  styles: `
    .banner { flex: none; display: flex; align-items: center; gap: 10px; padding: 6px 12px; font-size: var(--fs-sm); border-bottom: 1px solid var(--border); background: var(--bg-3); color: var(--text); border-left: 3px solid var(--lvl-warn); }
    .banner debrief-icon { color: var(--warn-text); }
    .msg { flex: 1; min-width: 0; }
  `,
})
export class PartialImportBanner {
  protected readonly fmt = formatCount;
  protected readonly svc = inject(ImportProgressService);
}
