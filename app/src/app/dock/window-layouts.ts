import { inject, Injectable } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { DockService } from './dock.service';
import { NAMED_LAYOUT_STORE, NamedLayouts } from './named-layouts';

/** Named window layouts (VS "Save/Apply Window Layout"): saving the current arrangement and applying a saved one. */
@Injectable({ providedIn: 'root' })
export class WindowLayouts {
  private readonly dock = inject(DockService);
  private readonly store = inject(WorkspaceStore);
  private readonly named = new NamedLayouts(inject(NAMED_LAYOUT_STORE));
  readonly list = this.named.list;

  /** Saves the current arrangement under `name`; returns the saved name, or null (empty name, no dock). */
  saveCurrent(name: string): string | null {
    const layout = this.dock.currentLayout();
    if (!layout) return null;
    const saved = this.named.save(name, layout);
    if (saved) this.store.setNotice($localize`:@@dock.windowLayouts.saved:Window layout "${saved}:name:" saved`);
    return saved;
  }

  apply(name: string): boolean {
    const layout = this.named.find(name);
    if (!layout) return false;
    const ok = this.dock.applyLayout(layout.saved);
    if (ok) this.store.setNotice($localize`:@@dock.windowLayouts.applied:Window layout "${layout.name}:name:" applied`);
    else this.store.setError($localize`:@@dock.windowLayouts.applyFailed:Window layout "${layout.name}:name:" could not be applied; the default layout is shown`);
    return ok;
  }

  /** Applies layout number `n` (Ctrl+Alt+n). */
  applyAt(n: number): boolean {
    const layout = this.named.at(n);
    return layout ? this.apply(layout.name) : false;
  }

  has(n: number): boolean {
    return !!this.named.at(n);
  }

  remove(name: string): void {
    this.named.remove(name);
  }

  rename(from: string, to: string): boolean {
    return this.named.rename(from, to);
  }

  move(name: string, delta: -1 | 1): void {
    this.named.move(name, delta);
  }
}
