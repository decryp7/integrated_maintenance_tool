import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';
import { chooseMenu } from './menu-support';
import { runCommand } from './profiling-support';

const dump = (page: Page) => page.locator('imt-dump-view');
const umdh = (page: Page) => page.locator('imt-umdh-view');

test.describe('Dumps', () => {
  test('a crash dump opens on its findings and leads to the culprit frame', async ({ page }, testInfo) => {
    await openApp(page);
    await chooseMenu(page, 'File', 'Open dump…');
    const view = dump(page);
    await expect(view.getByTestId('dump-exception')).toContainText('CLR_EXCEPTION');
    await expect(view.getByRole('tab', { name: 'Findings' })).toHaveAttribute('aria-selected', 'true');
    const findings = view.getByRole('list', { name: 'Findings' });
    await expect(findings.locator('li.finding').first()).toHaveAttribute('data-kind', 'crashCause');
    await expect(findings).toContainText('Likely culprit: Shop.Orders.dll!OrderCache::Add+0x4a');
    await expectNoSeriousViolations(page, testInfo, 'dump-findings', 'imt-dump-view');

    await view.getByRole('button', { name: /^Show the evidence of Likely culprit/ }).click();
    await expect(view.getByRole('tab', { name: 'Threads' })).toHaveAttribute('aria-selected', 'true');
    const stack = view.getByTestId('dump-stack');
    await expect(stack.locator('tr[data-culprit]')).toContainText('likely culprit');
    // The threads listbox: keyboard selection changes the stack.
    const threads = view.getByRole('listbox', { name: 'Threads' });
    await expect(threads.getByRole('option')).toHaveCount(4);
    await threads.getByRole('option').nth(3).click();
    await expect(stack).toContainText('Timer::Run');
    await page.keyboard.press('ArrowUp');
    await page.keyboard.press('Space');
    await expect(stack).toContainText('Ledger::Post');
    await expectNoSeriousViolations(page, testInfo, 'dump-threads', 'imt-dump-view');

    await view.getByRole('tab', { name: 'Modules' }).click();
    await expect(view.getByTestId('dump-modules').locator('tbody tr')).toHaveCount(6);
  });

  test('UMDH snapshots show the leaking stack and its tree', async ({ page }, testInfo) => {
    await openApp(page);
    await chooseMenu(page, 'File', 'Open UMDH snapshots…');
    const view = umdh(page);
    const findings = view.getByRole('list', { name: 'Findings' });
    await expect(findings.locator('li.finding').first()).toHaveAttribute('data-kind', 'leakGrowth');
    await expect(findings).toContainText('Steady growth in Shop.Orders!OrderCache::Add+0x4a across 3 snapshots');
    await expectNoSeriousViolations(page, testInfo, 'umdh-findings', 'imt-umdh-view');

    await view.getByRole('tab', { name: 'Growth' }).click();
    const growth = view.getByTestId('umdh-growth');
    await expect(growth.locator('tbody tr').first()).toContainText('+20 MB');
    await growth.locator('tbody tr').first().click();
    await expect(view.getByTestId('umdh-stack')).toContainText('allocated here');
    await view.getByTestId('umdh-group-module').check();
    await expect(growth.locator('tbody tr').first()).toContainText('Shop.Orders');

    await view.getByRole('tab', { name: 'Stack tree' }).click();
    const tree = view.getByRole('tree', { name: /Growing stacks/ });
    await expect(tree.getByRole('treeitem')).toHaveCount(1);
    await tree.getByRole('button', { name: 'Expand OrderService!wmain' }).click();
    await expect(tree.getByRole('treeitem').nth(1)).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'umdh-tree', 'imt-umdh-view');
  });

  test('Create dump of process writes a dump into the chosen folder and opens it', async ({ page }) => {
    await openApp(page);
    await runCommand(page, 'Create dump of process');
    const view = page.locator('imt-create-dump-view');
    const list = view.getByRole('listbox', { name: 'Processes' });
    await expect(list.getByRole('option')).toHaveCount(5);
    await view.getByRole('searchbox', { name: 'Filter processes' }).fill('4120');
    await list.getByRole('option').first().click();
    await view.getByTestId('capture-kind-heap').check();
    await view.getByTestId('capture-folder').click();
    await view.getByTestId('capture-create').click();
    // The written dump opens in the Dump view.
    await expect(dump(page).getByTestId('dump-exception')).toBeVisible();
    await expect(dump(page).getByTestId('dump-pick')).toContainText('OrderService.exe.4120.heap');
  });

  test('the dump appears on the log timeline at its time', async ({ page }) => {
    await openApp(page);
    await chooseMenu(page, 'File', 'Open dump…');
    await expect(dump(page).getByTestId('dump-exception')).toBeVisible();
    // The dump view opens tabbed with the timeline; bring it back, then open the dump from its marker.
    await page.getByRole('tab', { name: /Timeline/ }).click();
    const marker = page.locator('imt-timeline-panel').getByRole('button', { name: 'Open dump OrderService.exe.4120.dmp' });
    await expect(marker).toBeVisible();
    await marker.click();
    await expect(dump(page).getByTestId('dump-exception')).toBeVisible();
  });
});
