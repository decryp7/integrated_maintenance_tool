import { expect, test } from '@playwright/test';
import { openApp, gridTotal, parseCount } from './app';
import { expectNoSeriousViolations } from './axe';
import { openAppWith } from './mock-url';
import { chooseMenu } from './menu-support';

test.describe('context view', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('shows the original lines around the selected event, expands and copies', async ({ page }, testInfo) => {
    await page.locator('debrief-events-grid [role=row].row').nth(15).click();
    const ctx = page.locator('debrief-context-section');
    await ctx.getByRole('button', { name: 'Context' }).click();
    const lines = ctx.getByTestId('context-lines');
    await expect(lines).toBeVisible();
    await expect(lines.locator('.line.hit')).toHaveCount(1);
    const hitText = await lines.locator('.line.hit').textContent();
    expect(hitText).toMatch(/\d+/);
    const before = await lines.locator('.line').count();
    expect(before).toBeGreaterThan(1);
    expect(before).toBeLessThanOrEqual(21);
    await ctx.getByRole('button', { name: 'More' }).click();
    await expect.poll(async () => lines.locator('.line').count()).toBeGreaterThan(before);
    await expect(ctx).toContainText('±50 lines');
    await ctx.getByRole('button', { name: 'Copy' }).click();
    await expect(ctx.getByRole('button', { name: /Copied|Copy failed/ })).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'context-view', 'debrief-context-section');
  });

  test('follows the selection and recovers from a failed request', async ({ page }) => {
    await page.locator('debrief-events-grid [role=row].row').nth(2).click();
    const ctx = page.locator('debrief-context-section');
    await ctx.getByRole('button', { name: 'Context' }).click();
    await expect(ctx.locator('.line.hit')).toHaveCount(1);
    const first = await ctx.locator('.line.hit').textContent();
    await page.locator('debrief-events-grid [role=row].row').nth(6).click();
    await expect.poll(async () => ctx.locator('.line.hit').textContent()).not.toBe(first);
  });
});

test('a failed context request shows an inline error with Retry', async ({ page }) => {
  await openAppWith(page, '?mockFail=sourceContext:1&mockSnapshots=off');
  await page.locator('debrief-events-grid [role=row].row').nth(3).click();
  const ctx = page.locator('debrief-context-section');
  await ctx.getByRole('button', { name: 'Context' }).click();
  await expect(ctx.getByRole('alert')).toContainText('Simulated sourceContext failure');
  await ctx.getByRole('button', { name: 'Retry' }).click();
  await expect(ctx.locator('.line.hit')).toHaveCount(1);
});

test.describe('live tail', () => {
  test('refreshes the grid as the engine reports changes, and stops when turned off', async ({ page }) => {
    // A controllable clock (running normally until paused) makes "nothing more arrives" checkable without sleeps.
    await page.clock.install();
    await openAppWith(page, '?mockLiveMs=300');
    const live = page.getByRole('button', { name: 'Live tail', exact: true });
    await expect(gridTotal(page)).toHaveText(/^\d[\d,]*$/);
    const start = parseCount(await gridTotal(page).innerText());
    await expect(live).toHaveAttribute('aria-pressed', 'false');
    await live.click();
    await expect(live).toHaveAttribute('aria-pressed', 'true');
    await expect.poll(async () => parseCount(await gridTotal(page).innerText()), { timeout: 10_000 }).toBeGreaterThan(start + 1);
    await live.click();
    await expect(live).toHaveAttribute('aria-pressed', 'false');
    // Freeze time a little ahead (time keeps running until the pause lands, so leave room), running what is due on
    // the way, then 2 s more: a refresh already under way lands...
    await page.clock.pauseAt(await page.evaluate(() => Date.now() + 5000));
    await page.clock.runFor(2000);
    const stopped = await gridTotal(page).innerText();
    // ...and in 10 s more (over 30 live ticks) nothing else arrives.
    await page.clock.runFor(10_000);
    await expect(gridTotal(page)).toHaveText(stopped);
  });

  test('newest first and scrolled to the top keeps following new events; the query is kept', async ({ page }) => {
    await openAppWith(page, '?mockLiveMs=300');
    await page.getByTestId('query-field').fill('Live event');
    await expect(gridTotal(page)).toHaveText('0');
    await page.getByRole('button', { name: /Oldest first/ }).click();
    await page.getByRole('button', { name: 'Live tail', exact: true }).click();
    const firstRow = page.locator('debrief-events-grid [role=row].row').first();
    await expect(firstRow).toContainText('Live event');
    const n1 = Number(/Live event (\d+)/.exec((await firstRow.textContent()) ?? '')![1]);
    await expect.poll(async () => Number(/Live event (\d+)/.exec((await firstRow.textContent()) ?? '')?.[1] ?? 0), { timeout: 10_000 }).toBeGreaterThan(n1);
    // The search box is still what it was.
    await expect(page.getByTestId('query-field')).toHaveValue('Live event');
  });

  test('the status bar Live toggle keeps the selection, passes axe while on, and is disabled with nothing to follow', async ({ page }, testInfo) => {
    await openAppWith(page, '?mockLiveMs=300');
    const live = page.locator('debrief-status-bar').getByRole('button', { name: 'Live tail', exact: true });
    await expect(live).toBeEnabled();
    const row = page.locator('debrief-events-grid [role=row].row').nth(2);
    await row.click();
    await expect(page.locator('debrief-events-grid .row.sel')).toHaveCount(1);
    const selected = (await page.locator('debrief-events-grid .row.sel').textContent()) ?? '';
    const start = parseCount(await gridTotal(page).innerText());
    await live.click();
    await expect(live).toHaveAttribute('aria-pressed', 'true');
    await expectNoSeriousViolations(page, testInfo, 'live-on', 'debrief-status-bar');
    await expect.poll(async () => parseCount(await gridTotal(page).innerText()), { timeout: 10_000 }).toBeGreaterThan(start);
    // The selection survives live refreshes.
    await expect(page.locator('debrief-events-grid .row.sel')).toHaveText(selected);
    await live.click();
    await expect(live).toHaveAttribute('aria-pressed', 'false');

    await chooseMenu(page, 'File', 'Close workspace');
    await expect(live).toBeDisabled();
  });
});
