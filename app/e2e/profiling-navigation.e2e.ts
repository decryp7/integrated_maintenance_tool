import { expect, test } from '@playwright/test';
import { callTree, flame, hotspots, openProfiles, runCommand } from './profiling-support';

const PRICE = 'Shop.Orders.Pricing.PriceCalculator.Calculate';

test.describe('profiling navigation', () => {
  test('the average per call column sorts in the engine', async ({ page }) => {
    await openProfiles(page);
    const view = hotspots(page);
    const avg = view.getByRole('columnheader', { name: /Avg \/ call/ });
    await avg.getByRole('button').click();
    await expect(avg).toHaveAttribute('aria-sort', 'descending');
    await avg.getByRole('button').click();
    await expect(avg).toHaveAttribute('aria-sort', 'ascending');
  });

  test('"Tree" reveals the exact node; "Focus" zooms the flame graph to it with named breadcrumbs', async ({ page }) => {
    await openProfiles(page);
    const view = hotspots(page);
    await view.getByRole('searchbox', { name: 'Search hotspots' }).fill('PriceCalculator');
    await expect(view.locator('tbody td.name button')).toHaveText([PRICE]);

    await view.getByRole('button', { name: `Show ${PRICE} in the call tree` }).click();
    const grid = callTree(page).getByRole('treegrid', { name: /Call tree/ });
    await expect(grid.locator('tr[aria-selected="true"]')).toContainText(PRICE);

    await runCommand(page, 'Profiling: hotspots');
    await view.getByRole('button', { name: `Zoom the flame graph to ${PRICE}` }).click();
    const crumbs = flame(page).getByRole('navigation', { name: 'Zoom path' });
    await expect(crumbs.locator('[aria-current="location"]')).toHaveText(PRICE);
    await expect(crumbs.getByRole('button', { name: 'Shop.Orders.Program.Main' })).toBeVisible();
    await expect(crumbs).not.toContainText('frame #');
  });

  test('open captures are saved with the project and reopened with it', async ({ page }) => {
    await openProfiles(page);
    await runCommand(page, 'Save project as');
    await expect(page.getByRole('status').filter({ hasText: 'Saved Untitled' })).toBeVisible();

    await runCommand(page, 'New project');
    await runCommand(page, 'Profiling: show captures');
    const list = page.getByRole('list', { name: 'Open captures' });
    await expect(list).toHaveCount(0);

    await runCommand(page, 'Open project');
    await expect(list.locator(':scope > li')).toHaveCount(5);
    await expect(page.getByRole('status').filter({ hasText: 'Opened 5 captures.' })).toBeVisible();
    // Reopening left the project clean.
    await expect(page.getByRole('img', { name: 'Unsaved changes' })).toHaveCount(0);
    await expect(list).toContainText('orders.speedscope.json');
    await expect(list).toContainText('heap-3.heapsnapshot');
  });
});
