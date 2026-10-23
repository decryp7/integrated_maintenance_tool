import { Injectable, signal } from '@angular/core';

/** Labels of the dialog windows this window has open: they join the window group's UI state sync. */
@Injectable({ providedIn: 'root' })
export class DialogLabels {
  readonly labels = signal<readonly string[]>([]);
  add(label: string): void {
    this.labels.update((l) => (l.includes(label) ? l : [...l, label]));
  }
  remove(label: string): void {
    this.labels.update((l) => l.filter((x) => x !== label));
  }
}
