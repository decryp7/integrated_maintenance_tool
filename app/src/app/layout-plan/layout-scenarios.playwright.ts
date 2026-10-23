/**
 * Playwright helpers for layout scenarios (Debrief ADR 0056, Paddock ADR 0011).
 *
 * SELF-CONTAINED like `layout-plan.ts`: imports only `@playwright/test`, no Debrief code, so it moves to
 * `@decryp7/paddock-ui` unchanged. It asserts every pane against the minimum the app itself declares: each dock group
 * element carries `data-min-width` / `data-min-height` (set from the views' layout declarations), so the tests never
 * duplicate a number.
 */
import { expect, Page } from '@playwright/test';

export interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface GroupMeasure extends Box {
  id: string;
  /** The declared minimum (0 when the group declares none), px. */
  minWidth: number;
  minHeight: number;
  /** The panels (tabs) in the group. */
  tabs: string[];
}

/** A named window size. */
export interface ScenarioSize {
  width: number;
  height: number;
}

export const STANDARD_SIZES: readonly ScenarioSize[] = [
  { width: 1440, height: 900 },
  { width: 1920, height: 1080 },
];

/** Every docked (not floating) group that is on screen, with its box and its declared minimum. */
export async function measureGroups(page: Page): Promise<GroupMeasure[]> {
  return page.evaluate(() =>
    [...document.querySelectorAll<HTMLElement>('.dv-groupview')]
      .filter((g) => !g.closest('.dv-resize-container, .dv-resize-container-floating') && g.offsetParent !== null)
      .map((g, i) => {
        const r = g.getBoundingClientRect();
        return {
          id: g.id || `group-${i}`,
          x: Math.round(r.x),
          y: Math.round(r.y),
          width: Math.round(r.width),
          height: Math.round(r.height),
          minWidth: Number(g.dataset['minWidth'] ?? 0),
          minHeight: Number(g.dataset['minHeight'] ?? 0),
          tabs: [...g.querySelectorAll<HTMLElement>('[role=tab]')].map((t) => t.getAttribute('aria-label') ?? t.textContent?.trim() ?? ''),
        };
      }),
  );
}

/** The group holding a component (a custom element selector); null when it is not shown. */
export async function groupOf(page: Page, selector: string): Promise<Box | null> {
  return page.evaluate((s) => {
    const el = document.querySelector(s) as HTMLElement | null;
    const group = el?.closest('.dv-groupview') as HTMLElement | null;
    if (!el || !el.offsetParent || !group) return null;
    const r = group.getBoundingClientRect();
    return { x: Math.round(r.x), y: Math.round(r.y), width: Math.round(r.width), height: Math.round(r.height) };
  }, selector);
}

/** Every docked group is at or above the minimum it declares, and at least one group declares one. */
export async function expectGroupsAtMinimum(page: Page, tolerance = 1): Promise<GroupMeasure[]> {
  const groups = await measureGroups(page);
  expect(groups.some((g) => g.minWidth > 0), 'groups carry their declared minimum').toBe(true);
  for (const g of groups) {
    expect(g.width, `${g.tabs.join('+')} width ${g.width} under its minimum ${g.minWidth}`).toBeGreaterThanOrEqual(g.minWidth - tolerance);
    if (g.minHeight) expect(g.height, `${g.tabs.join('+')} height ${g.height} under its minimum ${g.minHeight}`).toBeGreaterThanOrEqual(g.minHeight - tolerance);
  }
  return groups;
}

/** No two docked groups overlap (by more than `tolerance` px in both axes). */
export function expectNoOverlap(groups: readonly Box[], tolerance = 2): void {
  for (let i = 0; i < groups.length; i++) {
    for (let j = i + 1; j < groups.length; j++) {
      const a = groups[i]!;
      const b = groups[j]!;
      const w = Math.min(a.x + a.width, b.x + b.width) - Math.max(a.x, b.x);
      const h = Math.min(a.y + a.height, b.y + b.height) - Math.max(a.y, b.y);
      expect(w > tolerance && h > tolerance, `groups ${i} and ${j} overlap (${w}x${h})`).toBe(false);
    }
  }
}

/** The page does not scroll sideways. */
export async function expectNoHorizontalScroll(page: Page): Promise<void> {
  const over = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(over, 'horizontal scroll').toBeLessThanOrEqual(0);
}

/** Resizes the window and waits for the dock to settle (two animation frames and its debounced re-plan). */
export async function resizeWindow(page: Page, size: ScenarioSize, settleMs = 450): Promise<void> {
  await page.setViewportSize(size);
  await page.evaluate(() => new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r()))));
  await page.waitForTimeout(settleMs);
}

/** Pane widths by label, for the report ("before and after"). */
export async function paneWidths(page: Page, panes: Record<string, string>): Promise<Record<string, number>> {
  const out: Record<string, number> = {};
  for (const [label, selector] of Object.entries(panes)) out[label] = (await groupOf(page, selector))?.width ?? -1;
  return out;
}
