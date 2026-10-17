import { Injectable, signal } from '@angular/core';

export interface MenuEntry {
  label: string;
  run: () => void;
  disabled?: boolean;
}

export interface OpenMenu {
  x: number;
  y: number;
  items: readonly MenuEntry[];
  /** Element to focus again when the menu closes without running an item. */
  returnFocus: HTMLElement | null;
}

/** Opens the one context menu the app shows at a time; rendered by `imt-context-menu-host`. */
@Injectable({ providedIn: 'root' })
export class ContextMenuService {
  readonly menu = signal<OpenMenu | null>(null);

  open(x: number, y: number, items: readonly MenuEntry[], returnFocus: HTMLElement | null = document.activeElement as HTMLElement | null): void {
    if (items.length) this.menu.set({ x, y, items, returnFocus });
  }

  /** Closes the menu; `restoreFocus` is false when an item ran (its action decides where focus goes). */
  close(restoreFocus = true): void {
    const menu = this.menu();
    this.menu.set(null);
    if (restoreFocus) menu?.returnFocus?.focus?.();
  }
}
