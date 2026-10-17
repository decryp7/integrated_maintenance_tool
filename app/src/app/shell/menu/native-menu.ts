import { DestroyRef, effect, inject, Injectable, InjectionToken, Injector, untracked } from '@angular/core';
import { Logger } from '../../core/logger';
import { CommandRegistry } from '../../extensions/command-registry';
import { matchesKeybinding } from '../../extensions/keybinding';
import { listenHere } from '../../windowing/window-events';
import { AppMenu } from './app-menu.service';
import { MenuNode, TopMenu } from './menu-model';
import { MENU_EVENT, NativeMenuEntry, NativeMenuModel } from './native-menu.types';

/** The shell side of the native menu bar (Tauri on macOS); fakes in tests. */
export interface NativeMenuPort {
  /** Hands the menu model to the shell, which builds (or updates) the native menu bar from it. */
  setAppMenu(menu: NativeMenuModel): Promise<void>;
  /** Calls `handler` with the item id when a native menu item is chosen while this window has the focus. */
  onMenu(handler: (id: string) => void): Promise<() => void>;
}

class TauriNativeMenu implements NativeMenuPort {
  async setAppMenu(menu: NativeMenuModel): Promise<void> {
    const { invoke } = await import('../../core/activity/ipc');
    await invoke('set_app_menu', { menu });
  }

  onMenu(handler: (id: string) => void): Promise<() => void> {
    return listenHere(MENU_EVENT, (payload) => {
      if (typeof payload === 'string') handler(payload);
    });
  }
}

export const NATIVE_MENU_PORT = new InjectionToken<NativeMenuPort>('NATIVE_MENU_PORT', { providedIn: 'root', factory: () => new TauriNativeMenu() });

/**
 * Edit menu commands whose keys the standard macOS Edit items (Undo/Redo for text fields) already show: listed
 * without a shortcut so the menu bar does not offer the same key twice. The key still reaches the page first.
 */
const NO_NATIVE_SHORTCUT = new Set(['edit.undo', 'edit.redo']);

function entries(nodes: readonly MenuNode[]): NativeMenuEntry[] {
  return nodes.flatMap((n): NativeMenuEntry[] => {
    switch (n.kind) {
      case 'separator':
        return [{ kind: 'separator' }];
      case 'submenu':
        return [{ kind: 'submenu', label: n.label, items: entries(n.items) }];
      case 'group':
        return [{ kind: 'header', label: n.label }, ...entries(n.items)];
      default:
        return [
          {
            kind: 'item',
            id: n.id,
            label: n.label,
            keybinding: n.keybinding && !NO_NATIVE_SHORTCUT.has(n.id) ? n.keybinding : null,
            checked: n.checked ?? null,
            enabled: true,
          },
        ];
    }
  });
}

/** The wire form of the menus for the native menu bar. Pure. */
export function toNativeModel(menus: readonly TopMenu[]): NativeMenuModel {
  return { menus: menus.map((m) => ({ id: m.id, label: m.label, items: entries(m.items) })) };
}

/** A menu event this soon after a key press that ran the same command is that key press echoed by the menu bar. */
const ECHO_MS = 400;

/**
 * Mirrors the menus into the macOS native menu bar and runs what is chosen there. The model is sent only when it
 * changed (the shell diffs again and updates check marks in place); it is re-sent when the window gets the focus, as
 * the menu bar shows the focused window's menus.
 */
@Injectable({ providedIn: 'root' })
export class NativeMenuBridge {
  private readonly port = inject(NATIVE_MENU_PORT);
  private readonly app = inject(AppMenu);
  private readonly registry = inject(CommandRegistry);
  private readonly logger = inject(Logger);
  private readonly destroyRef = inject(DestroyRef);
  private readonly injector = inject(Injector);
  private started = false;
  private lastSent = '';
  private lastKey: { id: string; at: number } | null = null;

  /** Starts mirroring. Idempotent. */
  start(): void {
    if (this.started) return;
    this.started = true;
    effect(
      () => {
        const model = toNativeModel(this.app.menus());
        untracked(() => this.send(model));
      },
      { injector: this.injector },
    );
    const onFocus = () => {
      this.lastSent = '';
      this.send(toNativeModel(this.app.menus()));
    };
    // A key press that runs a command is remembered, so the native echo of the same shortcut (if the menu bar
    // also sees the key) does not run it twice.
    const onKey = (e: KeyboardEvent) => {
      const c = this.registry.available().find((x) => x.keybinding && matchesKeybinding(x.keybinding, e));
      if (c) this.lastKey = { id: c.id, at: Date.now() };
    };
    window.addEventListener('focus', onFocus);
    document.addEventListener('keydown', onKey, true);
    let unlisten: (() => void) | null = null;
    let destroyed = false;
    this.port.onMenu((id) => this.chosen(id)).then(
      (u) => (destroyed ? u() : (unlisten = u)),
      (e: unknown) => this.logger.warn('ui.menu', 'Native menu events are unavailable', { Error: String(e) }),
    );
    this.destroyRef.onDestroy(() => {
      destroyed = true;
      window.removeEventListener('focus', onFocus);
      document.removeEventListener('keydown', onKey, true);
      unlisten?.();
    });
  }

  /** Runs a native menu item (unless it echoes the key press that just ran it). */
  chosen(id: string): void {
    if (this.lastKey && this.lastKey.id === id && Date.now() - this.lastKey.at < ECHO_MS) return;
    this.app.run(id);
  }

  private send(model: NativeMenuModel): void {
    const json = JSON.stringify(model);
    if (json === this.lastSent) return;
    this.lastSent = json;
    this.port.setAppMenu(model).catch((e: unknown) => {
      this.lastSent = '';
      this.logger.warn('ui.menu', 'The native menu could not be updated', { Error: String(e) });
    });
  }
}
