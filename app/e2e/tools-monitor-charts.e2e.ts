import { expect, Page, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { openAppWith, openView } from './tools-support';

/** Opens the Process Monitor, starts a run on one target and returns the live panel. */
async function startMonitor(page: Page, search = '', dotnet = false) {
  await openAppWith(page, search);
  await openView(page, 'Process Monitor');
  const panel = page.locator('imt-monitor-view');
  await expect(panel.getByTestId('monitor-live')).toHaveCount(0);
  await panel.getByLabel('Target value 1').fill('OrderService*');
  if (dotnet) await panel.getByLabel('.NET runtime counters (Framework)').check();
  await panel.getByRole('button', { name: 'Start' }).click();
  await expect(panel.getByTestId('monitor-status')).toContainText('Running');
  const live = panel.getByTestId('monitor-live');
  await expect(live).toBeVisible();
  return live;
}

test.describe('Process monitor live charts', () => {
  test('charts every metric per process, with window, pause/follow, legend toggles and tooltips', async ({ page }, testInfo) => {
    const live = await startMonitor(page);
    const process = live.locator('[data-group=process] figure');
    await expect(process).toHaveCount(7);
    for (let i = 0; i < 7; i++) await expect(process.nth(i).locator('canvas').first()).toBeVisible();
    // Without runtime counters there is no .NET group.
    await expect(live.locator('[data-group=dotnet]')).toHaveCount(0);

    const legend = live.getByRole('group', { name: 'Processes' });
    await expect(legend.getByRole('button')).toHaveText(['OrderService.exe · 4120', 'Worker.exe · 5232', 'sqlservr.exe · 1880']);

    // Hover: the axis tooltip lists the processes with exact values and the unit.
    const cpu = live.locator('figure[data-metric=cpu]');
    await cpu.scrollIntoViewIfNeeded();
    await cpu.locator('canvas').first().hover({ position: { x: 150, y: 80 } });
    await expect(cpu).toContainText('OrderService.exe · 4120');
    await expect(cpu).toContainText(/\d %/);

    // Legend toggle switches the process off in every chart.
    const worker = legend.getByRole('button', { name: /Worker\.exe/ });
    await worker.click();
    await expect(worker).toHaveAttribute('aria-pressed', 'false');
    await worker.click();
    await expect(worker).toHaveAttribute('aria-pressed', 'true');

    // Window selector.
    const windows = live.getByRole('group', { name: 'Time window' });
    await windows.getByRole('button', { name: '15 min', exact: true }).click();
    await expect(windows.getByRole('button', { name: '15 min', exact: true })).toHaveAttribute('aria-pressed', 'true');
    await expect(windows.getByRole('button', { name: '5 min', exact: true })).toHaveAttribute('aria-pressed', 'false');

    // Pause keeps what is shown; Follow resumes.
    const follow = live.getByTestId('monitor-follow');
    await expect(follow).toHaveAttribute('aria-pressed', 'true');
    await follow.click();
    await expect(follow).toHaveText(/Paused/);
    const until = await live.getByTestId('monitor-live-until').innerText();
    await page.waitForTimeout(2500);
    await expect(live.getByTestId('monitor-live-until')).toHaveText(until);
    await follow.click();
    await expect(follow).toHaveText(/Following/);

    await expectNoSeriousViolations(page, testInfo, 'process-monitor-live');
  });

  test('a second run charts only its own processes, never the earlier run still open', async ({ page }) => {
    const live = await startMonitor(page);
    const panel = page.locator('imt-monitor-view');
    const legend = live.getByRole('group', { name: 'Processes' });
    await expect(legend.getByRole('button')).toHaveText(['OrderService.exe · 4120', 'Worker.exe · 5232', 'sqlservr.exe · 1880']);

    await panel.getByRole('button', { name: 'Stop' }).click();
    await expect(panel.getByTestId('monitor-status')).toContainText('Stopped');
    await panel.getByRole('button', { name: 'Start' }).click();
    await expect(panel.getByTestId('monitor-status')).toContainText('Running');

    // Both runs' outputs are open; the charts ask for the new run's source id only.
    await expect(legend.getByRole('button')).toHaveText(['OrderService.exe · 5120', 'Worker.exe · 6232', 'sqlservr.exe · 2880']);
  });

  test('adds the .NET runtime charts when the run samples runtime counters', async ({ page }) => {
    const live = await startMonitor(page, '', true);
    const dotnet = live.getByRole('group', { name: '.NET runtime' });
    await expect(dotnet.locator('figure')).toHaveCount(4);
    await expect(dotnet.locator('figcaption .title')).toHaveText(['GC heap', '% time in GC', 'Exceptions', 'Lock contention']);
    await expect(dotnet.locator('canvas').first()).toBeVisible();
  });

  test('shows an empty state before the first samples', async ({ page }) => {
    const live = await startMonitor(page, '?mock-monitor-charts=empty');
    await expect(live.getByTestId('monitor-live-empty')).toContainText('No samples in the last 5 minutes');
  });

  test('shows a failed load inline and recovers with Retry', async ({ page }) => {
    const live = await startMonitor(page, '?mock-monitor-charts=fail');
    const error = live.getByTestId('monitor-live-error');
    await expect(error).toContainText('locked by another process');
    await error.getByRole('button', { name: 'Retry' }).click();
    await expect(error).toHaveCount(0);
    await expect(live.locator('[data-group=process] figure')).toHaveCount(7);
  });
});
