import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';
import { chooseMenu } from './menu-support';

async function closeWorkspace(page: Page): Promise<void> {
  await chooseMenu(page, 'File', 'Close workspace');
  await expect(page.locator('imt-welcome')).toBeVisible();
}

async function startDemo(page: Page): Promise<void> {
  await openApp(page);
  await closeWorkspace(page);
  const welcome = page.locator('imt-welcome');
  await expect(welcome).toContainText('IMT never modifies your files.');
  await welcome.getByRole('button', { name: 'Try a demo' }).click();
  await expect(welcome).toHaveCount(0);
  await expect(page.locator('imt-status-bar')).toContainText('events');
}

const tour = (page: Page) => page.getByTestId('tour');

test.describe('demo and guided tour', () => {
  test('the demo opens the sample incident and the tour can be completed with the keyboard', async ({ page }, testInfo) => {
    await startDemo(page);
    await expect(tour(page)).toBeVisible();
    await expect(tour(page)).toContainText('Step 1 of 6');
    await expect(tour(page).getByRole('heading', { name: 'Timeline' })).toBeVisible();
    // Anchored to the timeline with a spotlight, not a centred modal.
    await expect(page.locator('.spot')).toBeVisible();
    await expect(tour(page).getByRole('button', { name: 'Next' })).toBeFocused();
    await expectNoSeriousViolations(page, testInfo, 'tour', '[data-testid=tour]');

    const titles = ['Events', 'Query bar', 'Moment', 'Markers', 'Home'];
    for (const [i, title] of titles.entries()) {
      await page.keyboard.press('ArrowRight');
      await expect(tour(page)).toContainText(`Step ${i + 2} of 6`);
      await expect(tour(page).getByRole('heading', { name: title })).toBeVisible();
    }
    await expect(page.locator('imt-home-view')).toBeVisible();
    await page.keyboard.press('ArrowLeft');
    await expect(tour(page)).toContainText('Step 5 of 6');
    await tour(page).getByRole('button', { name: 'Next' }).click();
    await tour(page).getByRole('button', { name: 'Finish' }).click();
    await expect(tour(page)).toHaveCount(0);

    // Done: a second demo does not start it again, Help → Take the tour does.
    await closeWorkspace(page);
    await page.locator('imt-welcome').getByRole('button', { name: 'Try a demo' }).click();
    await expect(page.locator('imt-status-bar')).toContainText('events');
    await expect(tour(page)).toHaveCount(0);
    await chooseMenu(page, 'Help', 'Take the tour');
    await expect(tour(page)).toContainText('Step 1 of 6');
  });

  test('the tour can be skipped and is remembered as done', async ({ page }) => {
    await startDemo(page);
    await expect(tour(page)).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(tour(page)).toHaveCount(0);
    await page.reload();
    await expect(page.locator('imt-status-bar')).toContainText('events');
    await closeWorkspace(page);
    await page.locator('imt-welcome').getByRole('button', { name: 'Try a demo' }).click();
    await expect(page.locator('imt-status-bar')).toContainText('events');
    // The Focused layout shows the health check's headline instead of opening the Overview.
    await expect(page.getByRole('toolbar', { name: 'Health check' })).toBeVisible();
    await expect(tour(page)).toHaveCount(0);
    // Skip by button too.
    await chooseMenu(page, 'Help', 'Take the tour');
    await tour(page).getByRole('button', { name: 'Skip tour' }).click();
    await expect(tour(page)).toHaveCount(0);
  });
});
