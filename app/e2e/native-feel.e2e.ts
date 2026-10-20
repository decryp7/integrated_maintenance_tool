import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';

// The app should feel like a desktop app: no browser context menu on its chrome, no reload, page zoom or other
// browser keys. The guard only prevents the browser's default, so IMT's own menus and shortcuts keep working.

/** Records, after every other listener, whether the browser default of the next events of these types was prevented. */
async function watch(page: Page, types: string[]): Promise<void> {
  await page.evaluate((list) => {
    const w = window as unknown as { __feel: Record<string, boolean[]> };
    w.__feel = {};
    for (const type of list) {
      w.__feel[type] = [];
      // Added last on the window (bubble phase): runs after IMT's handlers and the guard.
      window.addEventListener(
        type,
        (e) => {
          // Pressing a modifier on its own is not a shortcut.
          if (e instanceof KeyboardEvent && ['Control', 'Shift', 'Alt', 'Meta'].includes(e.key)) return;
          w.__feel[type]!.push(e.defaultPrevented);
        },
        { passive: true },
      );
    }
  }, types);
}

const prevented = (page: Page, type: string) => page.evaluate((t) => (window as unknown as { __feel: Record<string, boolean[]> }).__feel[t] ?? [], type);

test.describe('native feel', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('right-clicking chrome shows no browser menu; IMT context menus still open; text fields keep theirs', async ({ page }) => {
    await watch(page, ['contextmenu']);
    await page.locator('imt-status-bar').click({ button: 'right' });
    await expect.poll(() => prevented(page, 'contextmenu')).toEqual([true]);
    // IMT's own menu on a dock tab.
    await page.getByRole('tab', { name: 'Events', exact: true }).click({ button: 'right' });
    await expect(page.getByRole('menu')).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(page.getByRole('menu')).toHaveCount(0);
    // A text field keeps the native menu (cut, copy, paste).
    await page.getByRole('combobox').or(page.getByRole('textbox')).first().click({ button: 'right' });
    await expect.poll(async () => (await prevented(page, 'contextmenu')).at(-1)).toBe(false);
  });

  test('reload keys do not reload the window', async ({ page }) => {
    await page.evaluate(() => ((window as unknown as { __marker: number }).__marker = 42));
    await watch(page, ['keydown']);
    await page.locator('imt-status-bar').click();
    await page.keyboard.press('F5');
    await page.keyboard.press('Control+r');
    await page.keyboard.press('Control+p');
    await page.keyboard.press('Control+Equal');
    await expect.poll(() => prevented(page, 'keydown')).toEqual([true, true, true, true]);
    await page.waitForTimeout(500);
    expect(await page.evaluate(() => (window as unknown as { __marker?: number }).__marker)).toBe(42);
  });

  test('Ctrl+wheel never zooms the page; the timeline still zooms', async ({ page }) => {
    await watch(page, ['wheel']);
    const range = page.getByTitle(/Reset to the full range|Drag on the timeline to zoom/).first();
    const before = await range.innerText();
    const box = (await page.locator('imt-timeline-panel .hist-row .plot').boundingBox())!;
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.keyboard.down('Control');
    await page.mouse.wheel(0, -400);
    await page.keyboard.up('Control');
    await expect
      .poll(async () => {
        const seen = await prevented(page, 'wheel');
        return seen.length > 0 && seen.every(Boolean);
      })
      .toBe(true);
    await expect(range).not.toHaveText(before);
    expect(await page.evaluate(() => window.visualViewport?.scale ?? 1)).toBe(1);
  });

  test('images and links cannot be dragged out of the window', async ({ page }) => {
    const result = await page.evaluate(() => {
      const a = document.createElement('a');
      a.href = '#x';
      a.textContent = 'link';
      document.body.append(a);
      const e = new DragEvent('dragstart', { bubbles: true, cancelable: true });
      a.dispatchEvent(e);
      a.remove();
      return e.defaultPrevented;
    });
    expect(result).toBe(true);
  });
});
