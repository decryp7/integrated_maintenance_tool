import { ChangeDetectionStrategy, Component, DestroyRef, effect, inject, OnInit, untracked } from '@angular/core';
import { CommandRegistry } from '../extensions/command-registry';
import { ErrorSnapshotService } from './error-snapshot.service';
import { SnapshotsStore } from './snapshots.store';
import { SnapshotInfo } from '../core/api.types';
import { REPORT_PROBLEM_COMMAND, SNAPSHOT_NOTICE } from './snapshot.types';
import { SETTINGS_DIAGNOSTICS_COMMAND } from '../settings/settings-dialog';
import { activeLocale } from '../i18n/locale';

/** Opens the Settings screen at its Diagnostics section (error snapshot switch and limits). */
export const SETTINGS_COMMAND = SETTINGS_DIAGNOSTICS_COMMAND;

/** Thumbnail grid of the saved error snapshots, with a large view of one and its context. */
@Component({
  selector: 'imt-error-snapshots-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="wrap">
      <div class="bar" role="toolbar" aria-label="Error snapshots actions" i18n-aria-label="@@diagnostics.snapshotsView.toolbar">
        <label class="all" i18n="@@diagnostics.snapshotsView.selectAll"><input type="checkbox" [checked]="store.allSelected()" [disabled]="!store.items().length" (change)="store.toggleAll()" /> Select all</label>
        <button type="button" class="btn" [disabled]="store.busy()" (click)="store.captureNow()" i18n="@@diagnostics.snapshotsView.captureNow">Capture now</button>
        <button type="button" class="btn primary" [disabled]="store.busy() || !store.items().length" (click)="store.exportSelected()" i18n="@@diagnostics.snapshotsView.export">Export diagnostics…</button>
        <button type="button" class="btn" [disabled]="store.busy() || !store.selected().length" (click)="store.remove(store.selected())" i18n="@@diagnostics.snapshotsView.deleteSelected">Delete selected</button>
        <span class="grow"></span>
        <button type="button" class="btn" (click)="store.openFolder()" i18n="@@diagnostics.snapshotsView.openFolder">Open folder</button>
        <button type="button" class="btn ghost" (click)="openSettings()" i18n="@@diagnostics.snapshotsView.settings">Settings…</button>
      </div>
      <p class="privacy" i18n="@@diagnostics.snapshotsView.privacy">{{ notice }} Snapshots may show log content; they leave this machine only when you export them.</p>
      @if (store.error(); as e) {
        <div class="msg err" role="alert">{{ e }}</div>
      }
      @if (store.info(); as i) {
        <div class="msg" role="status">{{ i }}</div>
      }
      <div class="body">
        <ul class="grid" aria-label="Error snapshots" i18n-aria-label="@@diagnostics.snapshotsView.list">
          @for (s of store.items(); track s.id) {
            <li class="card" [class.open]="store.open() === s.id" [attr.data-testid]="'snapshot-card'">
              <input type="checkbox" class="pick" [checked]="store.selected().includes(s.id)" (change)="store.toggle(s.id)" [attr.aria-label]="selectLabel(label(s))" />
              <button type="button" class="thumb" [attr.aria-label]="openLabel(label(s))" (click)="store.show(s.id)">
                @if (store.images()[s.id]; as src) {
                  <img [src]="src" alt="" loading="lazy" />
                } @else {
                  <span class="ph" i18n="@@diagnostics.snapshotsView.noPreview">No preview</span>
                }
              </button>
              <div class="meta">
                <span class="when">{{ when(s) }}</span>
                <span class="reason">{{ s.reason }}</span>
                <span class="text" [title]="s.message">{{ s.message }}</span>
              </div>
            </li>
          } @empty {
            <li class="empty">@if (store.loading()) {<ng-container i18n="@@diagnostics.snapshotsView.loading">Loading…</ng-container>} @else {<ng-container i18n="@@diagnostics.snapshotsView.empty">No error snapshots yet. IMT saves one when something goes wrong; press Ctrl/Cmd+Shift+F12 to take one yourself.</ng-container>}</li>
          }
        </ul>
        @if (store.current(); as c) {
          <section class="detail" aria-label="Snapshot details" i18n-aria-label="@@diagnostics.snapshotsView.details">
            <div class="dhead">
              <strong>{{ when(c) }}</strong>
              <span class="grow"></span>
              <button type="button" class="btn" (click)="store.remove([c.id])" i18n="Delete one snapshot|Button@@diagnostics.snapshotsView.delete">Delete</button>
              <button type="button" class="btn ghost" (click)="store.show(null)" i18n="Close the details|Button@@diagnostics.snapshotsView.close">Close</button>
            </div>
            @if (store.images()[c.id]; as src) {
              <img class="big" [src]="src" alt="Screenshot taken when the error happened" i18n-alt="@@diagnostics.snapshotsView.screenshotAlt" />
            }
            <dl>
              <dt i18n="@@diagnostics.snapshotsView.reason">Reason</dt><dd>{{ c.reason }}{{ c.manual ? manualSuffix : '' }}</dd>
              <dt i18n="@@diagnostics.snapshotsView.message">Message</dt><dd class="mono">{{ c.message }}</dd>
              @if (c.stack) { <dt i18n="@@diagnostics.snapshotsView.stack">Stack</dt><dd class="mono pre">{{ c.stack }}</dd> }
              @if (c.view) { <dt i18n="@@diagnostics.snapshotsView.activeView">Active view</dt><dd>{{ c.view }}</dd> }
              <dt i18n="@@diagnostics.snapshotsView.app">App</dt><dd i18n="@@diagnostics.snapshotsView.appValue">IMT {{ c.appVersion }} on {{ c.os }}</dd>
              <dt i18n="@@diagnostics.snapshotsView.id">Id</dt><dd class="mono">{{ c.id }}</dd>
            </dl>
            @if (c.steps.length) {
              <h4 id="snapshot-steps-title" i18n="@@diagnostics.snapshotsView.steps">Steps to reproduce</h4>
              <ol class="steps" aria-labelledby="snapshot-steps-title" data-testid="snapshot-steps">
                @for (step of c.steps; track $index) {
                  <li>{{ stripNumber(step) }}</li>
                }
              </ol>
            }
            <button type="button" class="btn report" (click)="report()" i18n="@@diagnostics.snapshotsView.report">Report a problem…</button>
          </section>
        }
      </div>
    </div>
  `,
  styles: `
    :host { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; background: var(--bg-1); }
    .wrap { flex: 1; min-height: 0; display: flex; flex-direction: column; gap: 8px; padding: 10px 14px; overflow: hidden; }
    .bar { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
    .grow { flex: 1; }
    .all { display: inline-flex; gap: 6px; align-items: center; color: var(--text-2); }
    .privacy { margin: 0; color: var(--text-2); font-size: var(--fs-sm); }
    .msg { padding: 6px 10px; border: 1px solid var(--border-2); border-radius: 6px; background: var(--bg-3); overflow-wrap: anywhere; }
    .msg.err { border-color: var(--lvl-error); color: var(--danger-text); }
    .body { flex: 1; min-height: 0; display: flex; gap: 12px; }
    .grid { list-style: none; margin: 0; padding: 0; flex: 1; min-width: 0; overflow: auto; display: grid; grid-template-columns: repeat(auto-fill, minmax(190px, 1fr)); gap: 10px; align-content: start; }
    .card { position: relative; border: 1px solid var(--border-2); border-radius: 8px; background: var(--bg-2); display: flex; flex-direction: column; overflow: hidden; }
    .card.open { border-color: var(--accent); }
    .pick { position: absolute; top: 6px; left: 6px; z-index: 1; }
    .thumb { all: unset; display: block; cursor: pointer; height: 110px; background: var(--bg-3); }
    .thumb:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -2px; }
    .thumb img { width: 100%; height: 100%; object-fit: cover; object-position: top left; display: block; }
    .ph { display: grid; place-items: center; height: 100%; color: var(--muted); font-size: var(--fs-sm); }
    .meta { display: flex; flex-direction: column; gap: 2px; padding: 6px 8px; font-size: var(--fs-sm); min-width: 0; }
    .when { color: var(--text); }
    .reason { color: var(--warn-text); }
    .text { color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .empty { grid-column: 1 / -1; color: var(--text-2); padding: 12px 0; }
    .detail { flex: 0 0 min(55%, 760px); min-width: 0; overflow: auto; border: 1px solid var(--border-2); border-radius: 8px; background: var(--bg-2); padding: 10px; display: flex; flex-direction: column; gap: 8px; }
    .dhead { display: flex; gap: 8px; align-items: center; }
    .big { width: 100%; height: auto; border: 1px solid var(--border-2); border-radius: 4px; }
    dl { margin: 0; display: grid; grid-template-columns: max-content 1fr; gap: 4px 12px; font-size: var(--fs-sm); }
    dt { color: var(--text-2); }
    dd { margin: 0; overflow-wrap: anywhere; }
    .pre { white-space: pre-wrap; max-height: 160px; overflow: auto; }
    h4 { margin: 4px 0 0; font-size: var(--fs-sm); }
    .steps { margin: 0; padding-left: 22px; font-size: var(--fs-sm); display: flex; flex-direction: column; gap: 2px; }
    .report { align-self: flex-start; }
  `,
})
export class ErrorSnapshotsView implements OnInit {
  protected openLabel(snapshot: string): string {
    return $localize`:@@diagnostics.snapshotsView.openOne:Open snapshot ${snapshot}:snapshot:`;
  }
  protected selectLabel(snapshot: string): string {
    return $localize`:@@diagnostics.snapshotsView.selectOne:Select snapshot ${snapshot}:snapshot:`;
  }
  protected readonly store = inject(SnapshotsStore);
  private readonly service = inject(ErrorSnapshotService);
  private readonly commands = inject(CommandRegistry);
  protected readonly notice = SNAPSHOT_NOTICE;
  protected readonly manualSuffix = $localize`:@@diagnostics.snapshotsView.manualSuffix: (manual)`;

  constructor() {
    // A new snapshot appears without reopening the view.
    let first = true;
    effect(() => {
      this.service.saved();
      if (first) return void (first = false);
      untracked(() => void this.store.refresh());
    });
    inject(DestroyRef).onDestroy(() => this.store.show(null));
  }

  ngOnInit(): void {
    void this.store.refresh();
  }

  protected openSettings(): void {
    void this.commands.run(SETTINGS_COMMAND);
  }

  protected report(): void {
    void this.commands.run(REPORT_PROBLEM_COMMAND);
  }

  /** The engine numbers its steps; the list numbers them again. */
  protected stripNumber(step: string): string {
    return step.replace(/^\d+\.\s*/, '');
  }

  protected when(s: SnapshotInfo): string {
    return new Date(s.createdAt).toLocaleString(activeLocale());
  }

  protected label(s: SnapshotInfo): string {
    return `${this.when(s)} ${s.reason}`;
  }
}
