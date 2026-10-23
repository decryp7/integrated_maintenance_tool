import { ChangeDetectionStrategy, Component, computed, DestroyRef, effect, ElementRef, inject, viewChild } from '@angular/core';
import { TileCfg } from '../../core/api.types';
import { Icon } from '../../shared/icon';
import { DialogService } from '../../ui/dialog.service';
import { AddTileDialog } from './add-tile-dialog';
import { DashboardStore } from './dashboard.store';
import { DashboardTile } from './dashboard-tile';

/** Height of one grid row and the gap between cells, px. */
export const ROW_PX = 64;
export const GAP_PX = 8;

/** What a key does to a focused tile in edit mode: move, resize or remove; null for other keys. */
export function tileKeyAction(key: string, shift: boolean): { kind: 'move' | 'resize'; dx: number; dy: number } | { kind: 'remove' } | null {
  const d: Record<string, [number, number]> = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] };
  if (key === 'Delete' || key === 'Backspace') return { kind: 'remove' };
  const step = d[key];
  return step ? { kind: shift ? 'resize' : 'move', dx: step[0], dy: step[1] } : null;
}

interface Drag {
  id: string;
  mode: 'move' | 'resize';
  px: number;
  py: number;
  start: TileCfg;
  cellW: number;
}

/** The "Dashboards" view: a grid of tiles from the profile's `[[dashboard]]`, with an edit mode and a wall mode. */
@Component({
  selector: 'debrief-dashboard-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [DashboardTile, Icon],
  host: { '[class.wall]': 'ds.wall()', '(keydown.escape)': 'exitWall()' },
  template: `
    <div class="bar" role="toolbar" aria-label="Dashboard" i18n-aria-label="@@share.dashboardView.toolbar">
      @if (ds.editing()) {
        <label class="title-edit"><ng-container i18n="@@share.dashboardView.title">Title</ng-container>&ngsp;<input class="input" [value]="ds.current()?.title ?? ''" (input)="ds.setTitle($any($event.target).value)" /></label>
        <button type="button" class="btn" (click)="addTile()"><debrief-icon name="plus" [size]="13" />&ngsp;<ng-container i18n="@@share.dashboardView.addTile">Add tile</ng-container></button>
        <span class="spacer"></span>
        <span class="hint faint" i18n="@@share.dashboardView.hint">Drag tiles, or focus one: arrows move, Shift+arrows resize, Delete removes.</span>
        <button type="button" class="btn" (click)="ds.cancel()" i18n="@@share.dashboardView.cancel">Cancel</button>
        <button type="button" class="btn primary" [disabled]="ds.saving()" (click)="ds.save()">{{ saveLabel() }}</button>
      } @else {
        @if (ds.dashboards().length) {
          <label class="pick"><ng-container i18n="@@share.dashboardView.pick">Dashboard</ng-container>
            <select class="select" [value]="ds.current()?.id ?? ''" (change)="ds.select($any($event.target).value)">
              @for (d of ds.dashboards(); track d.id) {<option [value]="d.id">{{ d.title }}</option>}
            </select>
          </label>
        }
        <span class="spacer"></span>
        <button type="button" class="btn" (click)="ds.newDashboard()"><debrief-icon name="plus" [size]="13" />&ngsp;<ng-container i18n="@@share.dashboardView.new">New dashboard</ng-container></button>
        @if (ds.current()) {
          <button type="button" class="btn" (click)="ds.startEdit()" i18n="@@share.dashboardView.edit">Edit</button>
          <button type="button" class="btn" [attr.aria-pressed]="ds.wall()" (click)="toggleWall()">{{ wallLabel() }}</button>
        }
      }
    </div>
    @if (ds.error(); as e) {<p class="err" role="alert" i18n="@@share.dashboardView.saveFailed">Could not save the dashboard: {{ e }}</p>}
    <div class="sr-only" aria-live="polite" data-testid="dashboard-announce">{{ ds.announcement() }}</div>
    @if (ds.current(); as d) {
      @if (ds.wall()) {<h2 class="wall-title">{{ d.title }}</h2>}
      <div class="scroll" tabindex="0" role="region" [attr.aria-label]="d.title">
        <div class="grid" #grid [class.editing]="ds.editing()" [style.grid-template-columns]="'repeat(' + d.columns + ', minmax(0, 1fr))'">
          @for (t of d.tile; track t.id) {
            <section
              class="tile"
              [attr.data-tile]="t.id"
              [attr.data-size]="t.w + 'x' + t.h"
              [attr.data-pos]="t.x + ',' + t.y"
              [style.grid-column]="t.x + 1 + ' / span ' + t.w"
              [style.grid-row]="t.y + 1 + ' / span ' + t.h"
              [style.--rows]="t.h"
              [attr.tabindex]="ds.editing() ? 0 : null"
              [attr.aria-label]="label(t)"
              (keydown)="onKey($event, t)"
              (pointerdown)="startDrag($event, t, 'move')"
            >
              <h3 class="tile-title">{{ t.title || t.id }}@if (t.window) {<span class="faint win" i18n="@@share.dashboardView.window"> · last {{ t.window }}</span>}</h3>
              <debrief-dashboard-tile [tile]="t" />
              @if (ds.editing()) {
                <button type="button" class="icon-btn remove" [attr.aria-label]="removeLabel(t)" (pointerdown)="$event.stopPropagation()" (click)="ds.remove(t.id)"><debrief-icon name="close" [size]="12" /></button>
                <span class="resizer" aria-hidden="true" (pointerdown)="startDrag($event, t, 'resize')"></span>
              }
            </section>
          } @empty {
            <p class="empty faint" i18n="@@share.dashboardView.empty">{ds.editing(), select, true {This dashboard has no tiles yet. Use “Add tile”.} other {This dashboard has no tiles yet.}}</p>
          }
        </div>
      </div>
    } @else {
      <div class="none">
        <p i18n="@@share.dashboardView.none">No dashboards in this profile yet.</p>
        <p class="faint" i18n="@@share.dashboardView.noneHelp">A dashboard puts charts, counts and top values of your events side by side. It is saved in the profile as a <code>[[dashboard]]</code> section.</p>
        <button type="button" class="btn primary" (click)="ds.newDashboard()"><debrief-icon name="plus" [size]="13" />&ngsp;<ng-container i18n="@@share.dashboardView.create">Create a dashboard</ng-container></button>
      </div>
    }
  `,
  styles: `
    :host { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; background: var(--bg-1); container-type: inline-size; }
    :host(.wall) { position: fixed; inset: 0; z-index: 1000; --tile-value-size: 44px; }
    .bar { display: flex; align-items: center; gap: 8px; padding: 8px 12px; border-bottom: 1px solid var(--border); flex-wrap: wrap; }
    .pick, .title-edit { display: flex; align-items: center; gap: 6px; font-size: var(--fs-sm); color: var(--muted); }
    .spacer { flex: 1; }
    .hint { font-size: var(--fs-xs); }
    .err { margin: 6px 12px 0; color: var(--danger-text); }
    .wall-title { margin: 12px 16px 0; font-size: var(--fs-xl); }
    .scroll { flex: 1; min-height: 0; overflow: auto; padding: 12px; }
    .grid { display: grid; grid-auto-rows: ${ROW_PX}px; gap: ${GAP_PX}px; }
    .tile { position: relative; display: flex; flex-direction: column; min-width: 0; min-height: 0; padding: 8px 10px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-2); overflow: hidden; }
    .editing .tile { cursor: grab; border-style: dashed; touch-action: none; }
    .tile:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 1px; }
    .tile-title { margin: 0 0 4px; font-size: var(--fs-sm); font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    :host(.wall) .tile-title { font-size: var(--fs-lg); }
    .win { font-weight: 400; }
    .remove { position: absolute; top: 4px; right: 4px; }
    .resizer { position: absolute; right: 0; bottom: 0; width: 14px; height: 14px; cursor: nwse-resize; border-right: 2px solid var(--muted); border-bottom: 2px solid var(--muted); }
    .empty, .none { padding: 24px; text-align: center; }
    .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); white-space: nowrap; }
    @container (max-width: 640px) {
      .grid { grid-template-columns: minmax(0, 1fr) !important; }
      .tile { grid-column: 1 / -1 !important; grid-row: auto / span var(--rows, 3) !important; }
    }
  `,
})
export class DashboardView {
  protected readonly ds = inject(DashboardStore);
  private readonly dialogs = inject(DialogService);
  private readonly host = inject<ElementRef<HTMLElement>>(ElementRef);
  private readonly grid = viewChild<ElementRef<HTMLElement>>('grid');
  private drag: Drag | null = null;

  constructor() {
    const onMove = (e: PointerEvent) => this.onPointerMove(e);
    const onUp = () => (this.drag = null);
    // Leaving full screen (Escape in native full screen) leaves wall mode, but only once this view was
    // actually full screen: a request the platform refuses or drops keeps the full-viewport fallback.
    let entered = false;
    const onFullscreen = () => {
      if (document.fullscreenElement === this.host.nativeElement) entered = true;
      else if (entered) {
        entered = false;
        if (this.ds.wall()) this.ds.wall.set(false);
      }
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    document.addEventListener('fullscreenchange', onFullscreen);
    inject(DestroyRef).onDestroy(() => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      document.removeEventListener('fullscreenchange', onFullscreen);
      this.ds.wall.set(false);
    });
    effect(() => {
      if (!this.ds.wall()) return;
      // Full screen when the platform allows it; the fixed full-viewport layout is the fallback.
      const el = this.host.nativeElement;
      if (typeof el.requestFullscreen === 'function' && !document.fullscreenElement) el.requestFullscreen().catch(() => undefined);
    });
  }

  protected readonly saveLabel = computed(() => (this.ds.saving() ? $localize`:@@share.dashboardView.saving:Saving…` : $localize`:@@share.dashboardView.save:Save`));
  protected readonly wallLabel = computed(() => (this.ds.wall() ? $localize`:@@share.dashboardView.exitWall:Exit wall mode` : $localize`:@@share.dashboardView.wall:Wall mode`));

  protected removeLabel(t: TileCfg): string {
    return $localize`:@@share.dashboardView.removeTile:Remove tile ${t.title || t.id}:title:`;
  }

  protected label(t: TileCfg): string {
    return $localize`:@@share.dashboardView.tileLabel:${t.title || t.id}:title:, ${t.type}:type: tile, column ${t.x + 1}:column:, row ${t.y + 1}:row:, ${t.w}:w: by ${t.h}:h:`;
  }

  protected async addTile(): Promise<void> {
    const tile = await this.dialogs.open<TileCfg>(AddTileDialog, { title: $localize`:@@share.addTile.dialogTitle:Add tile`, width: 520 }).closed;
    if (tile) this.ds.add(tile);
  }

  protected toggleWall(): void {
    if (this.ds.wall()) this.exitWall();
    else this.ds.wall.set(true);
  }

  protected exitWall(): void {
    if (!this.ds.wall()) return;
    this.ds.wall.set(false);
    if (document.fullscreenElement && typeof document.exitFullscreen === 'function') void document.exitFullscreen().catch(() => undefined);
  }

  protected onKey(e: KeyboardEvent, t: TileCfg): void {
    if (!this.ds.editing() || e.target !== e.currentTarget) return;
    const action = tileKeyAction(e.key, e.shiftKey);
    if (!action) return;
    e.preventDefault();
    if (action.kind === 'remove') this.ds.remove(t.id);
    else if (action.kind === 'move') this.ds.move(t.id, action.dx, action.dy);
    else this.ds.resize(t.id, action.dx, action.dy);
  }

  protected startDrag(e: PointerEvent, t: TileCfg, mode: 'move' | 'resize'): void {
    const grid = this.grid()?.nativeElement;
    const d = this.ds.current();
    if (!this.ds.editing() || !grid || !d || e.button !== 0) return;
    if (mode === 'move' && (e.target as HTMLElement).closest('button, input, select, a, .resizer')) return;
    e.preventDefault();
    e.stopPropagation();
    const cellW = (grid.clientWidth + GAP_PX) / d.columns;
    this.drag = { id: t.id, mode, px: e.clientX, py: e.clientY, start: { ...t }, cellW: cellW || 1 };
  }

  private onPointerMove(e: PointerEvent): void {
    const drag = this.drag;
    if (!drag) return;
    const dx = Math.round((e.clientX - drag.px) / drag.cellW);
    const dy = Math.round((e.clientY - drag.py) / (ROW_PX + GAP_PX));
    const s = drag.start;
    if (drag.mode === 'move') this.ds.place(drag.id, s.x + dx, s.y + dy);
    else this.ds.setSize(drag.id, s.w + dx, s.h + dy);
  }
}
