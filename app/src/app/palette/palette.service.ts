import { Injectable, signal } from '@angular/core';

/** Open/closed state of the command palette. */
@Injectable({ providedIn: 'root' })
export class PaletteService {
  readonly isOpen = signal(false);
  open(): void {
    this.isOpen.set(true);
  }
  close(): void {
    this.isOpen.set(false);
  }
}
