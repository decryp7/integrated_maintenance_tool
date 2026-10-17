import { BrowserContext, expect, Page, test } from '@playwright/test';
import { gridTotal, initialTotal, openApp, parseCount } from './app';
import { expectNoSeriousViolations } from './axe';
import { runCommand } from './analysis-support';
import { menuItem, openMenu } from './menu-support';

// Pop-out windows in the browser build: `window.open` popups of the same app, synced over a BroadcastChannel (the
// same port interface as the Tauri events between real OS windows).

const tab = (page: Page, name: string) => page.getByRole('tab', { name, exact: true });
const groupOf = (page: Page, name: string) => page.locator('.dv-groupview').filter({ has: tab(page, name) });

/** Pops a panel out with its header button and returns the new window once its shell is up. */
async function popOut(page: Page, context: BrowserContext, name: string): Promise<Page> {
  const opened = context.waitForEvent('page');
  await tab(page, name).click();
  await groupOf(page, name).getByRole('button', { name: 'Pop out to window' }).click();
  const popup = await opened;
  await expect(popup.locator('imt-popout-shell')).toBeVisible();
  await expect(popup.getByRole('heading', { level: 1, name })).toBeVisible();
  await expect(tab(page, name)).toHaveCount(0);
  return popup;
}

test.describe('pop-out windows', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('a panel pops out into a second window and the host selection reaches its inspector', async ({ page, context }) => {
    const popup = await popOut(page, context, 'Inspector');
    expect(popup.url()).toContain('#/popout?view=inspector');
    await expect(popup.getByRole('button', { name: 'Dock back into the main window' })).toBeVisible();
    const row = page.locator('imt-events-grid [role=row].row').nth(3);
    const message = (await row.locator('.msg').innerText()).trim();
    await row.click();
    await expect(popup.locator('imt-inspector')).toContainText(message.slice(0, 20));
    // And back: moving the selection in the host moves it in the pop-out.
    const next = page.locator('imt-events-grid [role=row].row').nth(6);
    const nextMessage = (await next.locator('.msg').innerText()).trim();
    await next.click();
    await expect(popup.locator('imt-inspector')).toContainText(nextMessage.slice(0, 20));
  });

  test('a query changed in the pop-out filters the host grid', async ({ page, context }) => {
    const before = parseCount(await initialTotal(page));
    const popup = await popOut(page, context, 'Query');
    const input = popup.getByRole('combobox', { name: 'Query' });
    await input.click();
    await input.fill('level >= error');
    await input.press('ControlOrMeta+Enter');
    await expect(popup.locator('imt-query-view .applied')).toContainText('level >= error');
    await expect.poll(async () => {
      const n = parseCount(await gridTotal(page).innerText());
      return n > 0 && n < before;
    }).toBe(true);
    await expect(page.locator('imt-events-grid [role=row].row').filter({ hasText: /INF|WRN|DBG|VRB/ })).toHaveCount(0);
  });

  test('Dock back returns the panel to where it was', async ({ page, context }) => {
    const popup = await popOut(page, context, 'Inspector');
    const closed = popup.waitForEvent('close');
    await popup.getByRole('button', { name: 'Dock back into the main window' }).click();
    await closed;
    await expect(tab(page, 'Inspector')).toBeVisible();
    // Back on the right of the documents, as before.
    const inspector = (await page.locator('imt-inspector').boundingBox())!;
    const events = (await page.locator('imt-events-grid').boundingBox())!;
    expect(inspector.x).toBeGreaterThanOrEqual(events.x + events.width - 2);
  });

  test('closing the pop-out window docks the panel back; it is never lost', async ({ page, context }) => {
    const popup = await popOut(page, context, 'Metrics');
    await popup.close();
    await expect(tab(page, 'Metrics')).toBeVisible();
    await expect(groupOf(page, 'Metrics')).toContainText('Timeline');
  });

  test('the View menu shows a popped-out panel as open; closing it there closes the window', async ({ page, context }) => {
    const popup = await popOut(page, context, 'Inspector');
    const item = menuItem(await openMenu(page, 'View'), 'Inspector');
    await expect(item).toHaveAttribute('aria-checked', 'true');
    const closed = popup.waitForEvent('close');
    await item.click();
    await closed;
    await expect(tab(page, 'Inspector')).toHaveCount(0);
  });

  test('a tab dragged out of the window pops out (pointer drag, released outside)', async ({ page, context }) => {
    const opened = context.waitForEvent('page');
    const t = (await tab(page, 'Inspector').boundingBox())!;
    await page.mouse.move(t.x + t.width / 2, t.y + t.height / 2);
    await page.mouse.down();
    await page.mouse.move(t.x + t.width / 2 - 40, t.y + 60, { steps: 5 });
    await page.mouse.move(t.x + t.width / 2 + 200, -40, { steps: 5 });
    await page.mouse.up();
    const popup = await opened;
    await expect(popup.getByRole('heading', { level: 1, name: 'Inspector' })).toBeVisible();
    await expect(tab(page, 'Inspector')).toHaveCount(0);
  });

  test('a pop-out dragged back over the host shows its guides and docks where it is dropped', async ({ page, context }) => {
    const popup = await popOut(page, context, 'Inspector');
    await expect(popup.locator('imt-inspector')).toBeVisible();
    // The left edge guide of the host's dock, in screen coordinates (the browser adapter maps them back).
    const dock = (await page.locator('.imt-dock-mount').boundingBox())!;
    const origin = await page.evaluate(() => ({ x: screenX + Math.max(0, (outerWidth - innerWidth) / 2), y: screenY + Math.max(0, outerHeight - innerHeight) }));
    const at = { x: origin.x + dock.x + 26, y: origin.y + dock.y + dock.height / 2 };
    const handle = popup.locator('imt-popout-title-bar .bar');
    await handle.dispatchEvent('pointerdown', { button: 0, pointerId: 1 });
    await handle.dispatchEvent('pointermove', { screenX: at.x + 300, screenY: at.y, pointerId: 1 });
    await handle.dispatchEvent('pointermove', { screenX: at.x, screenY: at.y, pointerId: 1 });
    await expect(page.locator('.imt-guide-edges .imt-guide-left.hot')).toBeVisible();
    await expect(page.locator('.imt-guide-preview')).toBeVisible();
    const closed = popup.waitForEvent('close');
    await handle.dispatchEvent('pointerup', { pointerId: 1 });
    await closed;
    await expect(tab(page, 'Inspector')).toBeVisible();
    await expect(page.locator('.imt-drop-guides.on')).toHaveCount(0);
    const inspector = (await page.locator('imt-inspector').boundingBox())!;
    const events = (await page.locator('imt-events-grid').boundingBox())!;
    expect(inspector.x + inspector.width).toBeLessThanOrEqual(events.x + 2);
  });

  test('the browser pop-out has no "Always on top" (a browser window cannot stay on top)', async ({ page, context }) => {
    const popup = await popOut(page, context, 'Inspector');
    await expect(popup.getByRole('button', { name: 'Dock back into the main window' })).toBeVisible();
    await expect(popup.getByRole('button', { name: 'Always on top' })).toHaveCount(0);
    await expect(popup).toHaveTitle('Inspector \u2014 Untitled');
  });

  test('the pop-out shell is accessible and follows the host theme', async ({ page, context }) => {
    const popup = await popOut(page, context, 'Inspector');
    for (const theme of ['dark', 'light'] as const) {
      await runCommand(page, `Theme: ${theme}`);
      await expect(popup.locator('html')).toHaveAttribute('data-theme', theme);
      await expectNoSeriousViolations(popup, test.info(), `popout-shell-${theme}`);
    }
    // The title bar's icon buttons are reachable with the keyboard.
    const dockBack = popup.getByRole('button', { name: 'Dock back into the main window' });
    await dockBack.focus();
    await expect(dockBack).toBeFocused();
    await expect(dockBack).toHaveAttribute('title', 'Dock back into the main window');
  });

  test('the pop-out reopens where it was after a reload of the host', async ({ page, context }) => {
    const popup = await popOut(page, context, 'Inspector');
    await expect.poll(() => page.evaluate(() => JSON.parse(localStorage.getItem('imt.layout') ?? 'null')?.extras?.poppedOut?.length ?? 0)).toBe(1);
    const reopened = context.waitForEvent('page', { predicate: (p) => p !== popup });
    await page.reload();
    // The old pop-out closes itself once its host is gone; the reloaded host opens it again.
    const again = await reopened;
    await expect(again.getByRole('heading', { level: 1, name: 'Inspector' })).toBeVisible();
    await expect(tab(page, 'Inspector')).toHaveCount(0);
  });
});
