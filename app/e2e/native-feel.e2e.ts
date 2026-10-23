import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';

// The app should feel like a desktop app: no browser context menu on its chrome, no reload, page zoom or other
// browser keys. The guard only prevents the browser's default, so Debrief's own menus and shortcuts keep working.

/** Records, after every other listener, whether the browser default of the next events of these types was prevented. */
async function watch(page: Page, types: string[]): Promise<void> {
  await page.evaluate((list) => {
    const w = window as unknown as { __feel: Record<string, boolean[]> };
    w.__feel = {};
    for (const type of list) {
      w.__feel[type] = [];
      // Added last on the window (bubble phase): runs after Debrief's handlers and the guard.
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

  test('right-clicking chrome shows no browser menu; Debrief context menus still open; text fields keep theirs', async ({ page }) => {
    await watch(page, ['contextmenu']);
    await page.locator('debrief-status-bar').click({ button: 'right' });
    await expect.poll(() => prevented(page, 'contextmenu')).toEqual([true]);
    // Debrief's own menu on a dock tab.
    await page.getByRole('tab', { name: 'Events', exact: true }).click({ button: 'right' });
    await expect(page.getByRole('menu')).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(page.getByRole('menu')).toHaveCount(0);
    // A text field keeps the native menu (cut, copy, paste).
    await page.getByRole('combobox').or(page.getByRole('textbox')).first().click({ button: 'right' });
    await expect.poll(async () => (await prevented(page, 'contextmenu')).at(-1)).toBe(false);
  });

  test('log text can be selected; chrome cannot', async ({ page }) => {
    const userSelect = (sel: string) =>
      page
        .locator(sel)
        .first()
        .evaluate((el) => {
          const cs = getComputedStyle(el) as CSSStyleDeclaration & { webkitUserSelect?: string };
          return cs.userSelect || cs.webkitUserSelect || '';
        });
    const selected = () => page.evaluate(() => window.getSelection()?.toString().trim() ?? '');
    // Near the start of the message: selecting a row opens the inspector over the right of the grid.
    const message = page.locator('debrief-events-grid [role=row].row .msg').first();
    await message.dblclick({ position: { x: 12, y: 6 } });
    await expect.poll(selected).not.toBe('');
    expect(await userSelect('debrief-events-grid [role=row].row .msg')).toBe('text');
    // The toolbar and the status bar are chrome: double-clicking their text selects nothing.
    // (WebKit reports an inherited `none` as `auto`, so what counts is that nothing gets selected.)
    for (const chrome of ['debrief-status-bar', 'debrief-title-bar', 'debrief-events-grid .bar']) {
      await page.evaluate(() => window.getSelection()?.removeAllRanges());
      await page.locator(chrome).first().dblclick({ position: { x: 40, y: 8 } });
      expect(await selected()).toBe('');
    }
  });

  test("right-clicking selected text opens Debrief's menu with Copy instead of the browser menu", async ({ page }) => {
    await page.locator('debrief-events-grid [role=row].row').first().click();
    const message = page.locator('debrief-inspector .message');
    await expect(message).not.toBeEmpty();
    expect(await message.evaluate((el) => getComputedStyle(el).userSelect || (getComputedStyle(el) as CSSStyleDeclaration & { webkitUserSelect?: string }).webkitUserSelect)).toBe('text');
    await message.dblclick({ position: { x: 12, y: 6 } });
    await expect.poll(() => page.evaluate(() => window.getSelection()?.toString().trim() ?? '')).not.toBe('');
    await watch(page, ['contextmenu']);
    await message.click({ button: 'right', position: { x: 12, y: 6 } });
    await expect.poll(() => prevented(page, 'contextmenu')).toEqual([true]);
    const menu = page.getByRole('menu');
    await expect(menu.getByRole('menuitem', { name: 'Copy', exact: true })).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(menu).toHaveCount(0);
  });

  test('middle-click does not start autoscroll on chrome, and files dropped outside drop targets do not navigate', async ({ page }) => {
    await watch(page, ['mousedown']);
    const box = (await page.locator('debrief-status-bar').boundingBox())!;
    await page.mouse.move(box.x + 40, box.y + box.height / 2);
    await page.mouse.down({ button: 'middle' });
    await page.mouse.up({ button: 'middle' });
    await expect.poll(() => prevented(page, 'mousedown')).toEqual([true]);
    const dropped = await page.evaluate(() => {
      const data = new DataTransfer();
      data.items.add(new File(['x'], 'app.log', { type: 'text/plain' }));
      const target = document.querySelector('debrief-status-bar')!;
      const over = new DragEvent('dragover', { bubbles: true, cancelable: true, dataTransfer: data });
      target.dispatchEvent(over);
      const drop = new DragEvent('drop', { bubbles: true, cancelable: true, dataTransfer: data });
      target.dispatchEvent(drop);
      return [over.defaultPrevented, drop.defaultPrevented];
    });
    expect(dropped).toEqual([true, true]);
  });

  test('reload keys do not reload the window', async ({ page }) => {
    await page.evaluate(() => ((window as unknown as { __marker: number }).__marker = 42));
    await watch(page, ['keydown']);
    await page.locator('debrief-status-bar').click();
    await page.keyboard.press('F5');
    await page.keyboard.press('ControlOrMeta+r');
    await page.keyboard.press('ControlOrMeta+p');
    await page.keyboard.press('ControlOrMeta+Equal');
    await expect.poll(() => prevented(page, 'keydown')).toEqual([true, true, true, true]);
    await page.waitForTimeout(500);
    expect(await page.evaluate(() => (window as unknown as { __marker?: number }).__marker)).toBe(42);
  });

  test('Ctrl+wheel never zooms the page; the timeline still zooms', async ({ page }) => {
    await watch(page, ['wheel']);
    const range = page.getByTitle(/Reset to the full range|Drag on the timeline to zoom/).first();
    const before = await range.innerText();
    const box = (await page.locator('debrief-timeline-panel .hist-row .plot').boundingBox())!;
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
