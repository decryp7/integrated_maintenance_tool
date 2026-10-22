import { Injectable, signal } from '@angular/core';

/** The tabs of the Plugins view. */
export type PluginsTab = 'installed' | 'health';

/** Which tab the Plugins view shows; a command can open the view on a given one. */
@Injectable({ providedIn: 'root' })
export class PluginsTabs {
  readonly tab = signal<PluginsTab>('installed');

  show(tab: PluginsTab): void {
    this.tab.set(tab);
  }
}
