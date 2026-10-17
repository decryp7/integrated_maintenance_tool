import { ChangeDetectionStrategy, Component, DestroyRef, effect, inject, OnInit, untracked } from '@angular/core';
import { CommandRegistry } from '../extensions/command-registry';
import { ErrorSnapshotService } from './error-snapshot.service';
import { SnapshotsStore } from './snapshots.store';
import { SnapshotInfo } from '../core/api.types';
import { REPORT_PROBLEM_COMMAND, SNAPSHOT_NOTICE } from './snapshot.types';
import { SETTINGS_DIAGNOSTICS_COMMAND } from '../settings/settings-dialog';

/** Opens the Settings screen at its Diagnostics section (error snapshot switch and limits). */
export const SETTINGS_COMMAND = SETTINGS_DIAGNOSTICS_COMMAND;

/** Thumbnail grid of the saved error snapshots, with a large view of one and its context. */
@Component({
  selector: 'imt-error-snapshots-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="wrap">
      <div class="bar" role="toolbar" aria-label="Error snapshots actions">
        <label class="all"><input type="checkbox" [checked]="store.allSelected()" [disabled]="!store.items().length" (change)="store.toggleAll()" /> Select all</label>
        <button type="button" class="btn" [disabled]="store.busy()" (click)="store.captureNow()">Capture now</button>
        <button type="button" class="btn primary" [disabled]="store.busy() || !store.items().length" (click)="store.exportSelected()">Export diagnostics…</button>
        <button type="button" class="btn" [disabled]="store.busy() || !store.selected().length" (click)="store.remove(store.selected())">Delete selected</button>
        <span class="grow"></span>
        <button type="button" class="btn" (click)="store.openFolder()">Open folder</button>
        <button type="button" class="btn ghost" (click)="openSettings()">Settings…</button>
      </div>
      <p class="privacy">{{ notice }} Snapshots may show log content; they leave this machine only when you export them.</p>
      @if (store.error(); as e) {
        <div class="msg err" role="alert">{{ e }}</div>
      }
      @if (store.info(); as i) {
        <div class="msg" role="status">{{ i }}</div>
      }
      <div class="body">
        <ul class="grid" aria-label="Error snapshots">
          @for (s of store.items(); track s.id) {
            <li class="card" [class.open]="store.open() === s.id" [attr.data-testid]="'snapshot-card'">
              <input type="checkbox" class="pick" [checked]="store.selected().includes(s.id)" (change)="store.toggle(s.id)" [attr.aria-label]="'Select snapshot ' + label(s)" />
              <button type="button" class="thumb" [attr.aria-label]="'Open snapshot ' + label(s)" (click)="store.show(s.id)">
                @if (store.images()[s.id]; as src) {
                  <img [src]="src" alt="" loading="lazy" />
                } @else {
                  <span class="ph">No preview</span>
                }
              </button>
              <div class="meta">
                <span class="when">{{ when(s) }}</span>
                <span class="reason">{{ s.reason }}</span>
                <span class="text" [title]="s.message">{{ s.message }}</span>
              </div>
            </li>
          } @empty {
            <li class="empty">{{ store.loading() ? 'Loading…' : 'No error snapshots yet. IMT saves one when something goes wrong; press Ctrl/Cmd+Shift+F12 to take one yourself.' }}</li>
          }
        </ul>
        @if (store.current(); as c) {
          <section class="detail" aria-label="Snapshot details">
            <div class="dhead">
              <strong>{{ when(c) }}</strong>
              <span class="grow"></span>
              <button type="button" class="btn" (click)="store.remove([c.id])">Delete</button>
              <button type="button" class="btn ghost" (click)="store.show(null)">Close</button>
            </div>
            @if (store.images()[c.id]; as src) {
              <img class="big" [src]="src" alt="Screenshot taken when the error happened" />
            }
            <dl>
              <dt>Reason</dt><dd>{{ c.reason }}{{ c.manual ? ' (manual)' : '' }}</dd>
              <dt>Message</dt><dd class="mono">{{ c.message }}</dd>
              @if (c.stack) { <dt>Stack</dt><dd class="mono pre">{{ c.stack }}</dd> }
              @if (c.view) { <dt>Active view</dt><dd>{{ c.view }}</dd> }
              <dt>App</dt><dd>IMT {{ c.appVersion }} on {{ c.os }}</dd>
              <dt>Id</dt><dd class="mono">{{ c.id }}</dd>
            </dl>
            @if (c.steps.length) {
              <h4 id="snapshot-steps-title">Steps to reproduce</h4>
              <ol class="steps" aria-labelledby="snapshot-steps-title" data-testid="snapshot-steps">
                @for (step of c.steps; track $index) {
                  <li>{{ stripNumber(step) }}</li>
                }
              </ol>
            }
            <button type="button" class="btn report" (click)="report()">Report a problem…</button>
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
    .privacy { margin: 0; color: var(--text-2); font-size: 12px; }
    .msg { padding: 6px 10px; border: 1px solid var(--border-2); border-radius: 6px; background: var(--bg-3); overflow-wrap: anywhere; }
    .msg.err { border-color: var(--lvl-error); color: var(--danger-text); }
    .body { flex: 1; min-height: 0; display: flex; gap: 12px; }
    .grid { list-style: none; margin: 0; padding: 0; flex: 1; min-width: 0; overflow: auto; display: grid; grid-template-columns: repeat(auto-fill, minmax(190px, 1fr)); gap: 10px; align-content: start; }
    .card { position: relative; border: 1px solid var(--border-2); border-radius: 8px; background: var(--bg-2); display: flex; flex-direction: column; overflow: hidden; }
    .card.open { border-color: var(--accent); }
    .pick { position: absolute; top: 6px; left: 6px; z-index: 1; }
    .thumb { all: unset; display: block; cursor: pointer; height: 110px; background: var(--bg-3); }
    .thumb:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
    .thumb img { width: 100%; height: 100%; object-fit: cover; object-position: top left; display: block; }
    .ph { display: grid; place-items: center; height: 100%; color: var(--muted); font-size: 12px; }
    .meta { display: flex; flex-direction: column; gap: 2px; padding: 6px 8px; font-size: 12px; min-width: 0; }
    .when { color: var(--text); }
    .reason { color: var(--warn-text); }
    .text { color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .empty { grid-column: 1 / -1; color: var(--text-2); padding: 12px 0; }
    .detail { flex: 0 0 min(55%, 760px); min-width: 0; overflow: auto; border: 1px solid var(--border-2); border-radius: 8px; background: var(--bg-2); padding: 10px; display: flex; flex-direction: column; gap: 8px; }
    .dhead { display: flex; gap: 8px; align-items: center; }
    .big { width: 100%; height: auto; border: 1px solid var(--border-2); border-radius: 4px; }
    dl { margin: 0; display: grid; grid-template-columns: max-content 1fr; gap: 4px 12px; font-size: 12px; }
    dt { color: var(--text-2); }
    dd { margin: 0; overflow-wrap: anywhere; }
    .pre { white-space: pre-wrap; max-height: 160px; overflow: auto; }
    h4 { margin: 4px 0 0; font-size: 12px; }
    .steps { margin: 0; padding-left: 22px; font-size: 12px; display: flex; flex-direction: column; gap: 2px; }
    .report { align-self: flex-start; }
  `,
})
export class ErrorSnapshotsView implements OnInit {
  protected readonly store = inject(SnapshotsStore);
  private readonly service = inject(ErrorSnapshotService);
  private readonly commands = inject(CommandRegistry);
  protected readonly notice = SNAPSHOT_NOTICE;

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
    return new Date(s.createdAt).toLocaleString();
  }

  protected label(s: SnapshotInfo): string {
    return `${this.when(s)} ${s.reason}`;
  }
}
