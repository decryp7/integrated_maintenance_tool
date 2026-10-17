import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import { Icon } from '../shared/icon';
import { fileName, percent, progressLine } from './import-format';
import { ImportProgressService } from './import-progress.service';

/** Shared view model of the progress widgets. */
function view(svc: ImportProgressService) {
  return computed(() => {
    const p = svc.progress();
    if (!p) return null;
    return { percent: percent(p), line: progressLine(p), file: fileName(p.current), path: p.current ?? '', counted: p.total > 0 };
  });
}

/** Import progress in the status bar: bar, files, bytes, ETA, current file, Cancel. */
@Component({
  selector: 'imt-import-status',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (vm(); as v) {
      <span class="wrap" data-testid="import-status">
        <span
          class="bar"
          role="progressbar"
          aria-label="Import progress"
          aria-valuemin="0"
          aria-valuemax="100"
          [attr.aria-valuenow]="v.counted ? v.percent : null"
          [attr.aria-valuetext]="v.line"
        ><span class="fill" [style.width.%]="v.percent"></span></span>
        <span>Importing {{ v.line }}</span>
        @if (v.file) { <span class="file" [title]="v.path">{{ v.file }}</span> }
        <button type="button" class="cancel" [disabled]="svc.cancelling()" (click)="svc.cancel()">{{ svc.cancelling() ? 'Cancelling…' : 'Cancel' }}</button>
      </span>
    }
  `,
  styles: `
    :host { display: contents; }
    .wrap { display: inline-flex; align-items: center; gap: 8px; min-width: 0; }
    .bar { width: 80px; height: 6px; border-radius: 3px; background: color-mix(in srgb, var(--accent-ink) 25%, transparent); overflow: hidden; flex: none; }
    .fill { display: block; height: 100%; background: var(--accent-ink); transition: width 0.2s; }
    .file { max-width: 180px; overflow: hidden; text-overflow: ellipsis; font-family: var(--font-mono); font-size: 11px; }
    .cancel { height: 18px; padding: 0 8px; border-radius: 9px; border: 1px solid currentColor; background: transparent; color: inherit; cursor: pointer; font-size: 11px; font-weight: 600; }
    .cancel:hover:not(:disabled) { background: color-mix(in srgb, var(--accent-ink) 18%, transparent); }
    @media (prefers-reduced-motion: reduce) { .fill { transition: none; } }
  `,
})
export class ImportStatus {
  protected readonly svc = inject(ImportProgressService);
  protected readonly vm = view(this.svc);
}

/** A non-blocking toast for long imports, with the same progress and Cancel. */
@Component({
  selector: 'imt-import-toast',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  template: `
    @if (svc.long()) {
      @if (vm(); as v) {
        <div class="toast" role="status" aria-label="Import in progress" data-testid="import-toast">
          <div class="head">
            <imt-icon name="refresh" [size]="14" />
            <strong>Importing…</strong>
            <span class="spacer"></span>
            <button type="button" class="btn" [disabled]="svc.cancelling()" (click)="svc.cancel()">{{ svc.cancelling() ? 'Cancelling…' : 'Cancel import' }}</button>
          </div>
          <div class="bar" aria-hidden="true"><span class="fill" [style.width.%]="v.percent"></span></div>
          <div class="line">{{ v.line }}</div>
          @if (v.file) { <div class="file" [title]="v.path">Reading {{ v.file }}</div> }
          <div class="faint small">You can keep working; files already read stay open if you cancel.</div>
        </div>
      }
    }
  `,
  styles: `
    .toast { position: fixed; left: 16px; bottom: 36px; z-index: 100; width: 360px; display: flex; flex-direction: column; gap: 6px; padding: 10px 12px; border-radius: 8px; background: var(--tooltip-bg); border: 1px solid var(--border-3); box-shadow: var(--shadow); color: var(--text); font-size: 12px; }
    .head { display: flex; align-items: center; gap: 8px; }
    .head imt-icon { color: var(--accent); }
    .spacer { flex: 1; }
    .bar { height: 6px; border-radius: 3px; background: var(--bg-3); overflow: hidden; }
    .fill { display: block; height: 100%; background: var(--accent); transition: width 0.2s; }
    .line { color: var(--text-2); }
    .file { font-family: var(--font-mono); font-size: 11px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .small { font-size: 11px; }
    @media (prefers-reduced-motion: reduce) { .fill { transition: none; } }
  `,
})
export class ImportToast {
  protected readonly svc = inject(ImportProgressService);
  protected readonly vm = view(this.svc);
}

/** Shown after a cancelled import: what was read, and how to read the rest. */
@Component({
  selector: 'imt-partial-import-banner',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  host: { style: 'display: contents' },
  template: `
    @if (svc.partial(); as p) {
      <div class="banner" role="status" data-testid="partial-import-banner">
        <imt-icon name="alert" [size]="14" />
        <span class="msg"><strong>Partially imported:</strong> {{ p.done }} of {{ p.total }} files were read before you cancelled. The rest are listed as not imported in the Explorer.</span>
        <button type="button" class="btn primary" (click)="svc.reimportAll()">Re-import all</button>
        <button type="button" class="btn ghost" (click)="svc.dismissPartial()">Dismiss</button>
      </div>
    }
  `,
  styles: `
    .banner { flex: none; display: flex; align-items: center; gap: 10px; padding: 6px 12px; font-size: 12px; border-bottom: 1px solid var(--border); background: var(--bg-3); color: var(--text); border-left: 3px solid var(--lvl-warn); }
    .banner imt-icon { color: var(--warn-text); }
    .msg { flex: 1; min-width: 0; }
  `,
})
export class PartialImportBanner {
  protected readonly svc = inject(ImportProgressService);
}
