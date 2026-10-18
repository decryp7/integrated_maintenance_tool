import { signal } from '@angular/core';

/** The parts of the dock `TimelineViews` uses; panels "open" when shown. */
export class TimelineFakeDock {
  readonly openIds = signal<ReadonlySet<string>>(new Set(['timeline']));
  readonly calls: string[] = [];
  readonly titles = new Map<string, string>();
  show = (id: string) => this.add(id, `show:${id}`);
  showBeside = (id: string, ref: string, dir: string) => this.add(id, `beside:${id}:${ref}:${dir}`);
  close = (id: string) => {
    this.calls.push(`close:${id}`);
    this.openIds.update((s) => new Set([...s].filter((x) => x !== id)));
  };
  setTitle = (id: string, title: string) => void this.titles.set(id, title);
  isOpen = (id: string) => this.openIds().has(id);
  private add(id: string, call: string): void {
    this.calls.push(call);
    this.openIds.update((s) => new Set([...s, id]));
  }
}
