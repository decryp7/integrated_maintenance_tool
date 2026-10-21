import { expect, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { callTree, findings, heap, openProfiles, runCommand, threads } from './profiling-support';

test.describe('Profiling findings and memory', () => {
  test('findings link to their evidence: call tree path, thread timeline events', async ({ page }, testInfo) => {
    await openProfiles(page);
    await runCommand(page, 'Profiling: findings');
    const view = findings(page);
    const lock = view.locator('li.finding[data-kind="lockContention"]');
    await expect(lock).toContainText('Critical');
    await expect(lock).toContainText('Lock contention');
    await expectNoSeriousViolations(page, testInfo, 'profiling-findings', 'imt-findings-view');

    await lock.getByRole('button', { name: /Open evidence/ }).click();
    const grid = callTree(page).getByRole('treegrid');
    await expect(grid.locator('tr[aria-selected="true"]')).toContainText('System.Threading.Monitor.Enter');

    await runCommand(page, 'Profiling: findings');
    await view.locator('li.finding[data-kind="gcPressure"]').getByRole('button', { name: /Open evidence/ }).click();
    await expect(threads(page).locator('.lg.hl')).toContainText('GC');
  });

  test('heap: types, snapshot diff with new types, steady growth and the path to a GC root', async ({ page }, testInfo) => {
    // Five views, each checked with axe: long while the suite runs in parallel.
    test.slow();
    await openProfiles(page);
    await runCommand(page, 'Profiling: memory');
    const view = heap(page);
    // The memory view starts on its findings (problem first).
    await expect(view.getByRole('list', { name: 'Memory findings' }).locator('li.finding').first()).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'profiling-heap-findings', 'imt-heap-view');
    await view.getByRole('tab', { name: 'Types' }).click();
    await expect(view.getByRole('table', { name: 'Types' })).toContainText('Shop.Orders.Order');
    await expectNoSeriousViolations(page, testInfo, 'profiling-heap-types', 'imt-heap-view');

    await view.getByRole('tab', { name: 'Compare snapshots' }).click();
    await view.getByRole('combobox', { name: 'Before snapshot' }).selectOption({ label: 'heap-1.heapsnapshot' });
    await view.getByRole('combobox', { name: 'After snapshot' }).selectOption({ label: 'heap-2.heapsnapshot' });
    const diff = view.getByRole('table', { name: /Snapshot comparison/ });
    await expect(diff.locator('tr', { hasText: 'Shop.Orders.AuditLog' }).first().locator('.tag')).toHaveText('new');
    await expectNoSeriousViolations(page, testInfo, 'profiling-heap-diff', 'imt-heap-view');

    await view.getByRole('tab', { name: 'Growth' }).click();
    const growth = view.getByRole('table', { name: 'Growth across snapshots' });
    const order = growth.locator('tr', { has: page.getByRole('button', { name: 'Shop.Orders.Order', exact: true }) });
    await expect(order).toContainText('steady growth');
    await expect(order.getByRole('img', { name: /Trend of Shop.Orders.Order/ })).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'profiling-heap-growth', 'imt-heap-view');

    await order.getByRole('button', { name: 'Shop.Orders.Order', exact: true }).click();
    await expect(view.getByRole('tab', { name: 'Instances and paths' })).toHaveAttribute('aria-selected', 'true');
    await view.getByRole('table', { name: 'Instances by retained size' }).getByRole('button').first().click();
    const chain = view.getByTestId('root-path').first();
    await expect(chain.locator('li').first()).toContainText('GC root');
    await expect(chain).toContainText('_orders');
    await expect(chain.locator('li').last()).toContainText('Shop.Orders.Order');
    await expectNoSeriousViolations(page, testInfo, 'profiling-heap-paths', 'imt-heap-view');

    await view.getByRole('tab', { name: 'Dominator tree' }).click();
    const dom = view.getByRole('treegrid', { name: 'Dominator tree' });
    await expect(dom.locator('tbody tr').first()).toBeVisible();
    await dom.locator('tbody tr').first().locator('.tw').click();
    await expect(dom.locator('tr[aria-level="2"]').first()).toBeVisible();
  });
});
