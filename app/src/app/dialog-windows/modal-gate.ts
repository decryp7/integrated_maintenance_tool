import { computed, Injectable, signal } from '@angular/core';

/**
 * Whether a modal dialog window is open over this window (ADR 0050): this window must not run commands or
 * shortcuts meanwhile (the OS disables the window; this covers menus and keys that still reach the page).
 */
@Injectable({ providedIn: 'root' })
export class ModalGate {
  private readonly open = signal(0);
  readonly blocked = computed(() => this.open() > 0);
  enter(): void {
    this.open.update((n) => n + 1);
  }
  leave(): void {
    this.open.update((n) => Math.max(0, n - 1));
  }
}
