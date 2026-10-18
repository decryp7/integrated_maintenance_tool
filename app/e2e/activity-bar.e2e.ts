import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';
import { openAppWith } from './mock-url';
import { expectNoSeriousViolations } from './axe';

// The left activity bar: every major area one click (or a few keys) away, built from the activity-item registry.

const bar = (page: Page) => page.locator('imt-activity-bar');
const barButton = (page: Page, name: string) => bar(page).getByRole('button', { name, exact: true });
const tab = (page: Page, id: string) => page.locator('.dv-tab', { has: page.locator(`.imt-tab[data-panel-id="${id}"]`) });

test.describe('activity bar', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('lists the areas in order, Settings last', async ({ page }) => {
    const names = await bar(page).getByRole('button').evaluateAll((els) => els.map((e) => e.getAttribute('aria-label')));
    expect(names).toEqual(['Explorer', 'Search', 'Timeline', 'Analysis', 'Metrics', 'Toggle live tail', 'AI assistant', 'Tools', 'Profile editor', 'Set up analysis for this folder', 'Settings']);
    await expect(barButton(page, 'Explorer')).toHaveAttribute('title', /^Explorer \((Ctrl\+B|⌘B)\)$/);
    // Every button is at least 24×24 px (WCAG 2.2 target size).
    for (const box of await bar(page).getByRole('button').evaluateAll((els) => els.map((e) => e.getBoundingClientRect()))) {
      expect(box.width).toBeGreaterThanOrEqual(24);
      expect(box.height).toBeGreaterThanOrEqual(24);
    }
  });

  test('opens Patterns from the Analysis fly-out, which checks it afterwards', async ({ page }, testInfo) => {
    await barButton(page, 'Analysis').click();
    const menu = page.getByRole('menu', { name: 'Analysis' });
    await expect(menu).toBeVisible();
    // The fly-out opens to the right of the bar.
    const [menuBox, buttonBox] = await Promise.all([menu.boundingBox(), barButton(page, 'Analysis').boundingBox()]);
    expect(menuBox!.x).toBeGreaterThan(buttonBox!.x + buttonBox!.width);
    expect(Math.abs(menuBox!.y - buttonBox!.y)).toBeLessThan(2);
    await expect(menu.getByRole('menuitemcheckbox', { name: 'Patterns' })).toHaveAttribute('aria-checked', 'false');
    // Scoped to the bar and its fly-out: this test owns those, not the panels behind them.
    await expectNoSeriousViolations(page, testInfo, 'activity-analysis-menu', '[data-activity-menu=analysis]');
    await expectNoSeriousViolations(page, testInfo, 'activity-bar-open', 'imt-activity-bar');
    await menu.getByRole('menuitemcheckbox', { name: 'Patterns' }).click();
    await expect(menu).toHaveCount(0);
    await expect(tab(page, 'patterns')).toBeVisible();
    await expect(barButton(page, 'Analysis')).toHaveClass(/\bon\b/);
    await barButton(page, 'Analysis').click();
    await expect(page.getByRole('menu', { name: 'Analysis' }).getByRole('menuitemcheckbox', { name: 'Patterns' })).toHaveAttribute('aria-checked', 'true');
    await page.keyboard.press('Escape');
  });

  test('toggles live tail together with the status bar toggle', async ({ page }) => {
    const live = barButton(page, 'Toggle live tail');
    const status = page.locator('imt-status-bar').getByRole('button', { name: 'Live tail', exact: true });
    await expect(live).toHaveAttribute('aria-pressed', 'false');
    await live.click();
    await expect(live).toHaveAttribute('aria-pressed', 'true');
    await expect(status).toHaveAttribute('aria-pressed', 'true');
    await expect(live.locator('.pulse')).toBeVisible();
    await live.click();
    await expect(live).toHaveAttribute('aria-pressed', 'false');
    await expect(status).toHaveAttribute('aria-pressed', 'false');
    await expect(live.locator('.pulse')).toHaveCount(0);
  });

  test('opens the AI chat once AI is set up', async ({ page }) => {
    await barButton(page, 'AI assistant').click();
    await expect(page.locator('imt-ai-chat-panel')).toBeVisible();
    await expect(barButton(page, 'AI assistant')).toHaveClass(/\bon\b/);
  });

  test('opens Process Monitor from the Tools fly-out', async ({ page }) => {
    await barButton(page, 'Tools').click();
    const menu = page.getByRole('menu', { name: 'Tools' });
    await expect(menu.getByRole('menuitemcheckbox', { name: 'Process Monitor' })).toBeVisible();
    await menu.getByRole('menuitemcheckbox', { name: 'Process Monitor' }).click();
    await expect(tab(page, 'process-monitor')).toBeVisible();
    await expect(page.locator('imt-monitor-view')).toBeVisible();
  });

  test('works from the keyboard alone: open, move, choose, and Escape back to the button', async ({ page }, testInfo) => {
    const analysis = barButton(page, 'Analysis');
    await analysis.focus();
    await expect(analysis).toBeFocused();
    await page.keyboard.press('ArrowRight');
    const menu = page.getByRole('menu', { name: 'Analysis' });
    await expect(menu.getByRole('menuitemcheckbox', { name: 'Query results' })).toBeFocused();
    await page.keyboard.press('Escape');
    await expect(menu).toHaveCount(0);
    await expect(analysis).toBeFocused();

    await page.keyboard.press('Enter');
    await expect(menu.getByRole('menuitemcheckbox').first()).toBeFocused();
    for (let i = 0; i < 3; i++) await page.keyboard.press('ArrowDown');
    await expect(menu.getByRole('menuitemcheckbox', { name: 'Patterns' })).toBeFocused();
    await page.keyboard.press('Enter');
    await expect(tab(page, 'patterns')).toBeVisible();

    // Tab reaches the next buttons; Space toggles live tail.
    await analysis.focus();
    await page.keyboard.press('Tab');
    await expect(barButton(page, 'Metrics')).toBeFocused();
    await page.keyboard.press('Tab');
    await expect(barButton(page, 'Toggle live tail')).toBeFocused();
    await page.keyboard.press('Space');
    await expect(barButton(page, 'Toggle live tail')).toHaveAttribute('aria-pressed', 'true');
    await expectNoSeriousViolations(page, testInfo, 'activity-bar', 'imt-activity-bar');
  });

  test('collapses the lower-priority buttons into More on a short window', async ({ page }) => {
    await page.setViewportSize({ width: 1200, height: 420 });
    const more = barButton(page, 'More');
    await expect(more).toBeVisible();
    await expect(barButton(page, 'Settings')).toBeVisible();
    await expect(barButton(page, 'Profile editor')).toHaveCount(0);
    await more.click();
    const menu = page.getByRole('menu', { name: 'More' });
    await menu.getByRole('menuitem', { name: 'Profile editor' }).click();
    await expect(tab(page, 'profile')).toBeVisible();
    await page.setViewportSize({ width: 1440, height: 900 });
    await expect(more).toHaveCount(0);
    await expect(barButton(page, 'Profile editor')).toBeVisible();
  });
});

test('the AI button is hidden while no model is set up (setup stays in the AI menu)', async ({ page }) => {
  await openAppWith(page, '?mockAi=setup');
  await expect(barButton(page, 'Toggle live tail')).toBeVisible();
  await expect(barButton(page, 'AI assistant')).toHaveCount(0);
});
