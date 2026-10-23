import { expect, test } from '@playwright/test';
import { openApp } from './app';
import { runCommand } from './analysis-support';

test.describe('release polish', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('the command bar reads "<name> — type a command" with a space before the dash', async ({ page }) => {
    const text = (await page.getByTestId('command-centre-name').innerText()).replace(/\s+/g, ' ');
    expect(text).toMatch(/\S — type a command$/);
  });

  test('a long query shows its start again when the field loses focus, with the full text as the tooltip', async ({ page }) => {
    const field = page.getByTestId('query-field');
    const long = 'checkout payment timeout '.repeat(12).trim();
    await field.click();
    await field.fill(long);
    await field.press('End');
    await page.getByTestId('view-options').focus();
    await expect(field).not.toBeFocused();
    expect(await field.evaluate((e: HTMLTextAreaElement) => e.scrollLeft)).toBe(0);
    await expect(field).toHaveAttribute('title', long);
  });

  test('a floated panel is sized for its content and stays inside the window', async ({ page }) => {
    await runCommand(page, 'Float panel');
    const floating = page.locator('.dv-resize-container').first();
    await expect(floating).toBeVisible();
    const box = (await floating.boundingBox())!;
    expect(box.width).toBeGreaterThan(700);
    expect(box.height).toBeGreaterThan(520);
    const vp = page.viewportSize()!;
    expect(box.x + box.width).toBeLessThanOrEqual(vp.width);
    expect(box.y + box.height).toBeLessThanOrEqual(vp.height);
  });

  test('Home fits the default window without scrolling its first screen, Recent activity collapsed', async ({ page }) => {
    await runCommand(page, 'Show Home');
    const home = page.locator('debrief-home-view');
    await page.locator('.dv-groupview').filter({ has: home }).getByRole('button', { name: 'Maximize panel' }).click();
    await expect(home.getByTestId('home-history')).toBeVisible();
    await expect(home.locator('details.feed')).not.toHaveAttribute('open', '');
    const scroll = home.locator('.scroll');
    const [client, full] = await scroll.evaluate((e) => [e.clientHeight, e.scrollHeight]);
    expect(full - client).toBeLessThanOrEqual(40); // at most the collapsed Recent activity line below the fold
  });

  test('the last row of a scrolled list ends inside its scroll area', async ({ page }) => {
    await runCommand(page, 'Show patterns');
    const list = page.locator('debrief-patterns-view ul.body');
    await expect(list.locator('li').first()).toBeVisible();
    await list.evaluate((e) => (e.scrollTop = e.scrollHeight));
    const last = (await list.locator('li').last().boundingBox())!;
    const area = (await list.boundingBox())!;
    expect(last.y + last.height).toBeLessThanOrEqual(area.y + area.height);
  });
});
