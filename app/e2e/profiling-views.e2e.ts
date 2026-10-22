import { expect, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { callTree, compare, flame, hotspotNames, hotspots, openProfiles, runCommand, showCaptures, threads } from './profiling-support';

test.describe('Profiling views', () => {
  test('open a profile: hotspots sorted by self time, labelled sampled/exact, then group by module', async ({ page }, testInfo) => {
    await openProfiles(page);
    const view = hotspots(page);
    const captures = await showCaptures(page);
    await expect(captures).toContainText('orders.speedscope.json');
    await expect(captures).toContainText('heap-3.heapsnapshot');
    // Import notes make limits visible.
    await expect(captures).toContainText('Stack depth limit');
    await page.getByRole('tab', { name: 'Hotspots', exact: true }).click();

    // Default sort: self of the metric, descending.
    const selfHeader = view.getByRole('columnheader', { name: /Self %/ });
    await expect(selfHeader).toHaveAttribute('aria-sort', 'descending');
    const names = await hotspotNames(page);
    expect(names[0]).toBe('Shop.Orders.OrderService.Place');

    // Sampled (≈) vs exact (=) tags on the headers.
    await expect(view.getByRole('columnheader', { name: /Self time/ }).locator('[data-measurement]')).toHaveAttribute('data-measurement', 'sampled');
    await expect(view.getByRole('columnheader', { name: /Exceptions/ }).locator('[data-measurement]')).toHaveAttribute('data-measurement', 'exact');
    await expect(view.getByRole('columnheader', { name: /Calls/ }).first().locator('[data-measurement]')).toHaveText('=');

    // Sort by wait time.
    await view.getByRole('button', { name: 'Wait', exact: true }).click();
    await expect(view.getByRole('columnheader', { name: /Wait/ })).toHaveAttribute('aria-sort', 'descending');
    await expect(view.locator('tbody td.name button').first()).toHaveText('ntdll!NtWaitForSingleObject');

    await view.getByRole('combobox', { name: 'Group by' }).selectOption('module');
    await expect(view.getByRole('columnheader', { name: /Module/ })).toBeVisible();
    await expect(view.locator('tbody td.name button').first()).toBeVisible();
    expect(await hotspotNames(page)).toContain('Shop.Orders');
    await expectNoSeriousViolations(page, testInfo, 'profiling-hotspots', 'imt-hotspots-view');
  });

  test('call tree: expand the hottest path and move with the keyboard', async ({ page }, testInfo) => {
    await openProfiles(page);
    await runCommand(page, 'Profiling: call tree');
    const tree = callTree(page);
    const grid = tree.getByRole('treegrid', { name: /Call tree/ });
    await expect(grid.locator('tbody tr[aria-level="1"]')).toHaveCount(1);
    await tree.getByRole('button', { name: 'Expand hottest path' }).click();
    await expect(grid.locator('tbody tr[aria-level="4"]').first()).toBeVisible();
    await expect(grid.locator('tr[aria-selected="true"]')).toContainText('Shop.Orders.OrderService.Place');

    // Keyboard: Home, then Right expands nothing new (already open) and moves to the child.
    const first = grid.locator('tbody tr').first();
    await first.focus();
    await page.keyboard.press('Home');
    await page.keyboard.press('ArrowLeft');
    await expect(first).toHaveAttribute('aria-expanded', 'false');
    await page.keyboard.press('ArrowRight');
    await expect(first).toHaveAttribute('aria-expanded', 'true');
    await page.keyboard.press('ArrowDown');
    await expect(grid.locator('tbody tr').nth(1)).toBeFocused();
    await expectNoSeriousViolations(page, testInfo, 'profiling-tree', 'imt-call-tree-view');
  });

  test('flame graph: zoom by click and breadcrumb, keyboard, search highlight', async ({ page }, testInfo) => {
    await openProfiles(page);
    await runCommand(page, 'Profiling: flame graph');
    const view = flame(page);
    const surface = view.getByRole('group', { name: /Flame graph of orders/ });
    await expect(surface).toBeVisible();
    await expect(view.locator('canvas')).toBeVisible();

    // Keyboard: the arrow away from the root goes to the callee; Enter zooms.
    await surface.focus();
    await page.keyboard.press('Home');
    await expect(view.getByTestId('flame-announce')).toContainText('Shop.Orders.Program.Main');
    await page.keyboard.press('ArrowUp');
    await expect(view.getByTestId('flame-announce')).toContainText('depth 1');
    await page.keyboard.press('Enter');
    const crumbs = view.getByRole('navigation', { name: 'Zoom path' });
    await expect(crumbs.locator('[aria-current="location"]')).not.toHaveText('All');
    await page.keyboard.press('Escape');
    await expect(crumbs.locator('[aria-current="location"]')).toHaveText('Shop.Orders.Program.Main');
    await crumbs.getByRole('button', { name: 'All' }).click();
    await expect(crumbs.locator('[aria-current="location"]')).toHaveText('All');

    // Click zoom: the bottom row (flame layout) is depth 0 = Main.
    const box = (await surface.boundingBox())!;
    await page.mouse.click(box.x + box.width / 2, box.y + box.height - 5);
    await expect(crumbs.locator('[aria-current="location"]')).toHaveText('Shop.Orders.Program.Main');
    await view.getByRole('button', { name: 'Reset zoom' }).click();

    // Search: the engine's mask highlights matches and reports the matched share.
    await view.getByRole('searchbox', { name: 'Search frames' }).fill('Json');
    await expect(view.getByTestId('flame-matched')).toContainText('Matched');
    await view.locator('details.frames summary').click();
    await expect(view.locator('details.frames .tag').first()).toHaveText('match');

    await view.getByRole('combobox', { name: 'Colour by' }).selectOption('module');
    await expect(view.getByRole('group', { name: 'Colour legend' })).toContainText('Other modules');
    await view.getByRole('button', { name: 'Icicle' }).click();
    await expectNoSeriousViolations(page, testInfo, 'profiling-flame', 'imt-flame-view');
  });

  test('differential flame graph against a baseline, with the function diff', async ({ page }, testInfo) => {
    await openProfiles(page);
    // Make the regression the target, the baseline the original.
    await (await showCaptures(page)).getByRole('button', { name: 'orders-regression.speedscope.json', exact: true }).click();
    await runCommand(page, 'Profiling: compare');
    const view = compare(page);
    await view.getByRole('combobox', { name: 'Baseline profile' }).selectOption({ label: 'orders.speedscope.json' });
    await expect(view.getByRole('group', { name: 'Colour legend' })).toContainText('more than the baseline');
    const table = view.getByRole('table', { name: /Function changes/ });
    await expect(table.locator('tbody tr').first()).toBeVisible();
    await view.getByRole('combobox', { name: 'Sort changes' }).selectOption('deltaSelf');
    await expect(table.locator('tbody tr').first()).toContainText('Utf8JsonWriter.WriteString');
    await expect(table.locator('tbody tr', { hasText: 'AuditLog.Write' }).locator('.tag')).toHaveText('new');
    await expectNoSeriousViolations(page, testInfo, 'profiling-compare', 'imt-compare-view');
  });

  test('brushing the thread timeline narrows the hotspots', async ({ page }, testInfo) => {
    await openProfiles(page);
    const before = await hotspots(page).getByTestId('hotspots-count').innerText();
    await runCommand(page, 'Profiling: thread timeline');
    const view = threads(page);
    const lanes = view.locator('.lanes');
    await expect(lanes.locator('.lane')).toHaveCount(4);
    const box = (await lanes.boundingBox())!;
    const y = box.y + 20;
    await page.mouse.move(box.x + 260, y);
    await page.mouse.down();
    await page.mouse.move(box.x + 420, y, { steps: 5 });
    await page.mouse.up();
    await expect(view.getByTestId('range-pill')).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'profiling-threads', 'imt-thread-timeline-view');

    await runCommand(page, 'Profiling: hotspots');
    await expect(hotspots(page).getByTestId('range-pill')).toBeVisible();
    await expect(hotspots(page).getByTestId('hotspots-count')).not.toHaveText(before);
    await hotspots(page).getByRole('button', { name: 'Clear time range' }).click();
    await expect(hotspots(page).getByTestId('hotspots-count')).toHaveText(before);
  });
});
