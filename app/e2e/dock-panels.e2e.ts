import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';
import { chooseMenu } from './menu-support';

// A panel scrolls in its own lists, never as a whole. Absolutely positioned content (text for screen readers above
// all) inside a scrolling list whose containing block lies outside the panel escapes the list: far down a long list
// it made dockview's view around the group scroll past the panel into empty space (seen in the AI chat on macOS).

/** Positioned elements inside a scrolling list of a panel whose containing block is outside that panel. */
function escapes(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const out: string[] = [];
    for (const panel of Array.from(document.querySelectorAll<HTMLElement>('.debrief-panel'))) {
      for (const el of Array.from(panel.querySelectorAll<HTMLElement>('*'))) {
        if (getComputedStyle(el).position !== 'absolute') continue;
        const cb = el.offsetParent;
        if (!cb || panel.contains(cb)) continue;
        for (let a = el.parentElement; a && a !== panel; a = a.parentElement) {
          const overflow = getComputedStyle(a).overflowY;
          if (overflow === 'auto' || overflow === 'scroll') {
            const view = panel.querySelector('debrief-panel-outlet')?.firstElementChild?.tagName.toLowerCase() ?? '?';
            out.push(`${view}: ${el.tagName.toLowerCase()}.${String(el.className)} escapes ${a.tagName.toLowerCase()}.${String(a.className)}`);
            break;
          }
        }
      }
    }
    return out;
  });
}

test('no panel lets positioned content escape its scrolling lists', async ({ page }) => {
  test.setTimeout(180_000);
  await openApp(page);
  const picker = page.getByRole('dialog', { name: 'Other windows' });
  await chooseMenu(page, 'View', 'Other windows…');
  // count() does not wait: the list renders after the dialog opens.
  await expect(picker.getByRole('option').first()).toBeVisible();
  const count = await picker.getByRole('option').count();
  await page.keyboard.press('Escape');
  expect(count).toBeGreaterThan(20);
  const found = new Set<string>();
  for (let i = 0; i < count; i++) {
    await chooseMenu(page, 'View', 'Other windows…');
    await picker.getByRole('option').nth(i).click();
    await expect(picker).toHaveCount(0);
    // A view that opens a dialog of its own (a wizard) is closed again; panels stay.
    await page.keyboard.press('Escape');
    for (const e of await escapes(page)) found.add(e);
  }
  expect([...found]).toEqual([]);
});
