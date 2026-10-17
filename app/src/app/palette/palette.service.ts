import { Injectable, signal } from '@angular/core';

/** What the palette lists: every command, or only the views (View ▸ Other windows…). */
export type PaletteMode = 'commands' | 'views';

/** Open/closed state of the command palette. */
@Injectable({ providedIn: 'root' })
export class PaletteService {
  readonly isOpen = signal(false);
  readonly mode = signal<PaletteMode>('commands');
  open(mode: PaletteMode = 'commands'): void {
    this.mode.set(mode);
    this.isOpen.set(true);
  }
  close(): void {
    this.isOpen.set(false);
  }
}
