import { inject, Injectable } from '@angular/core';
import { DockService } from '../dock/dock.service';
import { PANEL } from '../dock/dock-context';
import { DebriefCommand } from '../extensions/tokens';
import { WorkspaceStore } from '../core/workspace.store';

/** Marks a shell region (activity bar, side bar, filter bar, status bar): a landmark that F6 focuses. */
export const REGION_ATTR = 'data-debrief-region';
const GROUP = '.dv-groupview';

/** Whether an element takes up room on screen (a hidden dock group or a closed side bar does not). */
function shown(el: HTMLElement): boolean {
  return el.getClientRects().length > 0;
}

/**
 * The regions in F6 order: the activity bar, the side bar, the filter bar, each dock group (in document order:
 * left to right, top to bottom, floating groups last) and the status bar. Only regions on screen count.
 */
export function regionsOf(root: ParentNode, isShown: (el: HTMLElement) => boolean = shown): HTMLElement[] {
  const marked = (name: string) => root.querySelector<HTMLElement>(`[${REGION_ATTR}="${name}"]`);
  const groups = [...root.querySelectorAll<HTMLElement>(GROUP)];
  return [marked('activity'), marked('sidebar'), marked('filters'), ...groups, marked('status')].filter((el): el is HTMLElement => !!el && isShown(el));
}

/** Focuses the element `selector` names, retrying briefly while it is being created or its tab is being shown. */
export function focusWhenThere(selector: string, attempts = 20): void {
  const el = document.querySelector<HTMLElement>(selector);
  el?.focus();
  if ((!el || document.activeElement !== el) && attempts > 0) setTimeout(() => focusWhenThere(selector, attempts - 1), 50);
}

/** The index of the region holding `el`, or -1. */
export function regionIndex(regions: readonly HTMLElement[], el: Element | null): number {
  return el ? regions.findIndex((r) => r.contains(el)) : -1;
}

/**
 * Focuses a region: a dock group by its active tab (from where Tab enters the panel and arrows switch tabs), any
 * other region itself (its landmark, `tabindex=-1`, shows the focus ring; Tab goes on into it).
 */
export function focusRegion(region: HTMLElement): void {
  if (region.matches(GROUP)) {
    const tab = region.querySelector<HTMLElement>('.dv-tab.dv-active-tab') ?? region.querySelector<HTMLElement>('[role=tab]');
    (tab ?? region).focus();
    return;
  }
  region.focus();
}

/**
 * Keyboard movement between the window's regions (desktop-IDE conventions): F6 / Shift+F6 cycle them, Ctrl+0 the side
 * bar, Ctrl+Alt+1…9 the dock groups ({@link handleKey}), Ctrl+Shift+G the events grid.
 */
@Injectable({ providedIn: 'root' })
export class RegionNavigator {
  private readonly store = inject(WorkspaceStore);
  private readonly dock = inject(DockService);

  /** Moves focus to the next (1) or previous (-1) region, wrapping around. */
  cycle(direction: 1 | -1, root: ParentNode = document, isShown: (el: HTMLElement) => boolean = shown): boolean {
    const regions = regionsOf(root, isShown);
    if (!regions.length) return false;
    const at = regionIndex(regions, document.activeElement);
    const next = at < 0 ? (direction === 1 ? 0 : regions.length - 1) : (at + direction + regions.length) % regions.length;
    focusRegion(regions[next]!);
    return true;
  }

  /** Focuses dock group `n` (1-based, document order). */
  group(n: number, root: ParentNode = document, isShown: (el: HTMLElement) => boolean = shown): boolean {
    const group = [...root.querySelectorAll<HTMLElement>(GROUP)].filter(isShown)[n - 1];
    if (!group) return false;
    focusRegion(group);
    return true;
  }

  /** Focuses the side bar, opening it first. */
  sidebar(): void {
    if (!this.store.sidebarOpen()) this.store.toggleSidebar();
    focusWhenThere(`[${REGION_ATTR}="sidebar"]`);
  }

  /** Brings the events grid to the front and focuses it. */
  events(): void {
    this.dock.show(PANEL.events);
    focusWhenThere('debrief-events-grid [role=grid]');
  }

  /**
   * Ctrl+Alt+1…9 focus dock group 1…9 (Ctrl+1…9 switch perspectives). Read from `e.code`: with Alt held, macOS
   * reports a symbol in `e.key`, not the digit. Handled here rather than as nine palette commands; returns true when
   * the key was one of them.
   */
  handleKey(e: KeyboardEvent): boolean {
    const digit = /^Digit([1-9])$/.exec(e.code ?? '')?.[1];
    if (!digit || !e.ctrlKey || !e.altKey || e.shiftKey || e.metaKey) return false;
    e.preventDefault();
    this.group(Number(digit));
    return true;
  }

  readonly commands: DebriefCommand[] = (() => {
    const category = $localize`:@@shell.regions.category:View`;
    return [
      { id: 'view.focusNextRegion', title: $localize`:@@shell.regions.next:Focus next region`, category, keybinding: 'F6', run: () => void this.cycle(1) },
      { id: 'view.focusPreviousRegion', title: $localize`:@@shell.regions.previous:Focus previous region`, category, keybinding: 'Shift+F6', run: () => void this.cycle(-1) },
      { id: 'view.focusSidebar', title: $localize`:@@shell.regions.sidebar:Focus side bar`, category, keybinding: 'Mod+0', run: () => this.sidebar() },
      { id: 'view.focusEvents', title: $localize`:@@shell.regions.events:Focus events grid`, category, keybinding: 'Mod+Shift+G', run: () => this.events() },
    ];
  })();
}
