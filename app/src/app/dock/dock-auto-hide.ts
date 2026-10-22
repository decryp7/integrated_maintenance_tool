import { inject, Injectable } from '@angular/core';
import { AutoHideEvent, AutoHideItem, autoHideReduce } from './auto-hide';
import { DockContext, PANEL, rectOf } from './dock-context';
import { nearestSide } from './dock-geometry';
import { DockPersistence } from './dock-persistence';
import { DockPlacement } from './dock-placement';

/**
 * Auto-hidden tool windows (VS's "Auto Hide"): collapsing a docked tool window into a tab on a side bar, sliding it
 * out and back in, and docking it back where it was ("Pin").
 */
@Injectable({ providedIn: 'root' })
export class DockAutoHide {
  private readonly ctx = inject(DockContext);
  private readonly placement = inject(DockPlacement);
  private readonly persistence = inject(DockPersistence);

  /** Auto-hidden tool windows and which one is slid out. */
  readonly state = this.ctx.autoHide.asReadonly();

  /** Collapses a docked tool window into a tab on the nearest side bar ("Auto hide"). */
  unpin(id: string): void {
    const api = this.ctx.api;
    const panel = api?.getPanel(id);
    if (!api || !panel || this.ctx.kindOf(id) !== 'tool' || panel.group.api.location.type !== 'grid') return;
    const rect = rectOf(panel.group.element);
    const root = this.ctx.rootRect();
    const side = rect && root ? nearestSide(rect, root) : 'right';
    const item: AutoHideItem = {
      id,
      title: this.ctx.views.find(id)?.title ?? id,
      side,
      slot: this.placement.captureSlot(id) ?? { referencePanel: null, direction: 'within' },
      size: rect ? (side === 'bottom' ? rect.height : rect.width) : 320,
    };
    api.removePanel(panel);
    this.ctx.autoHide.update((s) => autoHideReduce(s, { type: 'unpin', item }));
    this.ctx.refreshOpen();
    this.persistence.schedule();
  }

  /** Docks an auto-hidden tool window back where it was ("Pin"). */
  pin(id: string): void {
    const item = this.ctx.autoHide().items.find((i) => i.id === id);
    if (!item) return;
    this.ctx.autoHide.update((s) => autoHideReduce(s, { type: 'pin', id }));
    this.placement.restoreSlot(id, item.slot);
    this.ctx.api?.getPanel(id)?.api.setActive();
    this.ctx.refreshOpen();
    this.persistence.schedule();
  }

  /** Closes an auto-hidden tool window (it is no longer on a side bar, and not docked either). */
  remove(id: string): void {
    this.ctx.autoHide.update((s) => autoHideReduce(s, { type: 'pin', id }));
    this.persistence.schedule();
  }

  event(event: AutoHideEvent): void {
    this.ctx.autoHide.update((s) => autoHideReduce(s, event));
    if (event.type === 'resize') this.persistence.schedule();
  }

  /** Toggles auto-hide of the active tool window (or of the slid-out one). */
  toggleActive(): void {
    const open = this.ctx.autoHide().open;
    if (open) this.pin(open);
    else if (this.ctx.api?.activePanel) this.unpin(this.ctx.api.activePanel.id);
  }

  /** Auto-hides every docked tool window and returns their ids. */
  unpinAll(): string[] {
    const ids: string[] = [];
    for (const p of [...(this.ctx.api?.panels ?? [])]) {
      if (this.ctx.kindOf(p.id) !== 'tool' || p.group.api.location.type !== 'grid') continue;
      this.unpin(p.id);
      ids.push(p.id);
    }
    return ids;
  }

  /** Pins these tool windows back, those that are still auto-hidden (one the user pinned or closed since is left alone). */
  pinIds(ids: readonly string[]): void {
    for (const id of ids) if (this.ctx.autoHide().items.some((i) => i.id === id)) this.pin(id);
  }

  pinAll(): void {
    for (const i of [...this.ctx.autoHide().items]) this.pin(i.id);
  }

  /**
   * Slides the auto-hidden inspector out (without taking focus). Nothing when it is docked, popped out or closed, or
   * when a tool window is docked on its edge: sliding over it would hide what the user is working with there.
   */
  revealInspector(): void {
    const api = this.ctx.api;
    if (!api) return;
    const s = this.ctx.autoHide();
    const item = s.items.find((i) => i.id === PANEL.inspector);
    if (!item || s.open === PANEL.inspector) return;
    if (item.side !== 'left' && this.placement.toolGroupFor(api, item.side)) return;
    this.event({ type: 'activate', id: PANEL.inspector });
  }
}
