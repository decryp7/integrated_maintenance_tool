import { Injectable, signal } from '@angular/core';

export type HeapTab = 'types' | 'diff' | 'growth' | 'dominators' | 'instances';

export const HEAP_TABS: { id: HeapTab; label: string }[] = [
  { id: 'types', label: 'Types' },
  { id: 'diff', label: 'Compare snapshots' },
  { id: 'growth', label: 'Growth' },
  { id: 'dominators', label: 'Dominator tree' },
  { id: 'instances', label: 'Instances and paths' },
];

/** Selection shared by the memory view's tabs (provided by the view). */
@Injectable()
export class HeapSelection {
  readonly tab = signal<HeapTab>('types');
  /** Type whose instances are listed. */
  readonly typeName = signal<string | null>(null);
  /** Object whose paths to GC roots are shown. */
  readonly object = signal<number | null>(null);

  showInstances(typeName: string): void {
    this.typeName.set(typeName);
    this.object.set(null);
    this.tab.set('instances');
  }

  showPaths(object: number, typeName?: string): void {
    if (typeName) this.typeName.set(typeName);
    this.object.set(object);
    this.tab.set('instances');
  }
}

/** Sparkline points (polyline in a w × h box) for a series; flat series sit in the middle. */
export function sparkline(values: readonly number[], w = 80, h = 20): string {
  if (!values.length) return '';
  const min = Math.min(...values);
  const max = Math.max(...values);
  const step = values.length > 1 ? w / (values.length - 1) : 0;
  return values.map((v, i) => `${(i * step).toFixed(1)},${(max === min ? h / 2 : h - 1 - ((v - min) / (max - min)) * (h - 2)).toFixed(1)}`).join(' ');
}
