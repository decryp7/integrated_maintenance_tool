import { inject, Injectable, Injector } from '@angular/core';
import { CommandRegistry } from '../extensions/command-registry';
import { isMac, matchesKeybinding } from '../extensions/keybinding';
import { DockService } from './dock.service';

/** The physical key of a binding's last part, for layouts where Alt changes `KeyboardEvent.key` (macOS Option). */
const CODES: Record<string, string> = { '-': 'Minus', enter: 'Enter', '1': 'Digit1', '2': 'Digit2', '3': 'Digit3', '4': 'Digit4' };

/**
 * Like `matchesKeybinding`, by physical key: on macOS Option+1 types "¡", so `key` never matches "Ctrl+Alt+1".
 * Only used when the binding has Alt.
 */
export function matchesByCode(binding: string, e: KeyboardEvent, mac = isMac()): boolean {
  const parts = binding.split('+').map((p) => p.trim().toLowerCase());
  const key = parts.pop() ?? '';
  if (!parts.includes('alt')) return false;
  const code = CODES[key] ?? (key.length === 1 && /[a-z]/.test(key) ? `Key${key.toUpperCase()}` : null);
  if (!code || e.code !== code) return false;
  const mod = parts.includes('mod');
  return (
    e.altKey &&
    e.ctrlKey === (parts.includes('ctrl') || (mod && !mac)) &&
    e.metaKey === (parts.includes('meta') || (mod && mac)) &&
    e.shiftKey === parts.includes('shift')
  );
}

/**
 * Window-level keys of the dock: the Ctrl+Tab navigator (held Ctrl, Tab cycles, releasing Ctrl switches; Ctrl+Tab
 * on macOS too, like VS), and the dock commands whose Alt bindings `KeyboardEvent.key` misses on macOS.
 */
@Injectable({ providedIn: 'root' })
export class DockKeys {
  private readonly dock = inject(DockService);
  private readonly injector = inject(Injector);

  listen(target: Document | HTMLElement): () => void {
    const down = (e: Event) => this.keydown(e as KeyboardEvent);
    const up = (e: Event) => this.keyup(e as KeyboardEvent);
    target.addEventListener('keydown', down, true);
    target.addEventListener('keyup', up, true);
    return () => {
      target.removeEventListener('keydown', down, true);
      target.removeEventListener('keyup', up, true);
    };
  }

  keydown(e: KeyboardEvent): void {
    if (e.key === 'Tab' && e.ctrlKey && !e.altKey && !e.metaKey) {
      e.preventDefault();
      e.stopPropagation();
      this.dock.switch(e.shiftKey);
      return;
    }
    if (this.dock.switcher()) {
      const step: Record<string, number> = { ArrowDown: 1, ArrowRight: 1, ArrowUp: -1, ArrowLeft: -1 };
      if (e.key === 'Escape') this.dock.cancelSwitch();
      else if (e.key === 'Enter') this.dock.commitSwitch();
      else if (step[e.key]) this.dock.switch((step[e.key] ?? 0) < 0);
      else return;
      e.preventDefault();
      e.stopPropagation();
      return;
    }
    if (!e.altKey) return;
    const registry = this.injector.get(CommandRegistry);
    const command = registry.available().find((c) => c.id.startsWith('window.') && c.keybinding && !matchesKeybinding(c.keybinding, e) && matchesByCode(c.keybinding, e));
    if (!command) return;
    e.preventDefault();
    e.stopPropagation();
    void registry.run(command.id);
  }

  keyup(e: KeyboardEvent): void {
    if (e.key === 'Control' && this.dock.switcher()) {
      e.preventDefault();
      this.dock.commitSwitch();
    }
  }
}
