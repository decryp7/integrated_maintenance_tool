import { expect, Page, test } from '@playwright/test';
import { gridTotal, initialTotal, openApp } from './app';
import { expectNoSeriousViolations } from './axe';
import { openAppWith } from './mock-url';

async function runCommand(page: Page, title: string): Promise<void> {
  await page.keyboard.press('ControlOrMeta+k');
  const input = page.getByRole('combobox', { name: 'Type a command' });
  await input.fill(title);
  await expect(page.locator('imt-command-palette').getByRole('option').first()).toContainText(title);
  await page.keyboard.press('Enter');
  await expect(input).toHaveCount(0);
}

const DOWNTIME = 'net.decryptology.windows/downtime';

test.describe('plugin annotations on the timeline', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('bands, glyphs, legend toggle, tooltip and evidence', async ({ page }, testInfo) => {
    const timeline = page.locator('imt-timeline-panel');
    const legend = timeline.getByRole('group', { name: 'Annotations' });
    await expect(legend).toContainText('Annotations');
    const downtime = timeline.locator(`.lane .ann[data-kind="${DOWNTIME}"]`);
    // WEB02 crashed and restarted: its downtime is drawn on both WEB02 lanes.
    await expect(downtime).toHaveCount(2);
    await expect(timeline.locator('.lane .ann.point').first()).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'timeline-annotations');

    // Hiding a kind from the legend removes its bands; showing it again brings them back.
    const toggle = legend.locator(`[data-kind="${DOWNTIME}"]`);
    await toggle.click();
    await expect(toggle).toHaveAttribute('aria-pressed', 'false');
    await expect(downtime).toHaveCount(0);
    await toggle.click();
    await expect(downtime).toHaveCount(2);

    // Tooltip: label, owner and time (also on keyboard focus; markers may sit on top of the band).
    await downtime.first().focus();
    const tip = page.getByTestId('annotation-tip');
    await expect(tip).toContainText('Unexpected shutdown');
    await expect(tip).toContainText('net.decryptology.windows');
    await expect(tip).toContainText('→');

    // Click applies the evidence query (machine filter and time range).
    const all = await initialTotal(page);
    await downtime.first().press('Enter');
    await expect(gridTotal(page)).not.toHaveText(all);
    await expect(page.getByRole('button', { name: 'Fit' })).toBeEnabled();
  });

  test('the Evidence menu reaches every annotation from the keyboard', async ({ page }, testInfo) => {
    const timeline = page.locator('imt-timeline-panel');
    // Bands and glyphs are pointer shortcuts (markers may cover them): they are not tab stops.
    await expect(timeline.locator('.lane .ann').first()).toHaveAttribute('tabindex', '-1');
    const evidence = timeline.getByRole('group', { name: 'Annotations' }).getByRole('button', { name: 'Evidence…' });
    await evidence.focus();
    await page.keyboard.press('Enter');
    const menu = page.getByRole('menu');
    const item = menu.getByRole('menuitem', { name: /Unexpected shutdown/ }).first();
    await expect(item).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'timeline-evidence-menu');
    const all = await initialTotal(page);
    await item.click();
    await expect(menu).toHaveCount(0);
    await expect(gridTotal(page)).not.toHaveText(all);
  });
});

test('annotations come back after a failed request with Retry', async ({ page }) => {
  // Load the app once, with the failure (no prior load to wait for), and sync on the timeline's lanes: the
  // annotations are requested only once the timeline has data, so the error can only appear after them.
  await openAppWith(page, '?mockFail=annotations:1&mockSnapshots=off');
  const timeline = page.locator('imt-timeline-panel');
  await expect(timeline.locator('.lane').first()).toBeVisible();
  const legend = timeline.getByRole('group', { name: 'Annotations' });
  await expect(legend.getByRole('alert')).toHaveText('Annotations unavailable');
  await expect(timeline.locator('.lane .ann')).toHaveCount(0);
  await legend.getByRole('button', { name: 'Retry' }).click();
  await expect(timeline.locator('.lane .ann').first()).toBeVisible();
  await expect(legend.getByRole('alert')).toHaveCount(0);
});

test.describe('plugin marker kinds', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('are listed read-only with their owner, can be switched off and restyled', async ({ page }, testInfo) => {
    await runCommand(page, 'Manage markers');
    const table = page.getByRole('table', { name: 'Plugin marker kinds' });
    const crash = table.locator('tr[data-kind="net.decryptology.windows/app-crash"]');
    await expect(crash).toContainText('net.decryptology.windows');
    await expect(crash).toContainText('read-only');
    await expect(crash.getByRole('button', { name: /^Edit/ })).toHaveCount(0);
    await expectNoSeriousViolations(page, testInfo, 'markers-plugin-kinds');

    await table.getByRole('checkbox', { name: 'Disable Application crash' }).click();
    await expect(crash).toHaveClass(/off/);
    await page.getByRole('tab', { name: 'Timeline' }).click();
    await expect(page.locator('imt-timeline-panel .legend').getByRole('button', { name: 'Application crash' })).toHaveCount(0);
    await page.getByRole('tab', { name: 'Markers' }).click();
    await table.getByRole('checkbox', { name: 'Enable net.decryptology.windows/app-crash' }).click();
    await expect(crash).not.toHaveClass(/off/);

    await table.getByRole('button', { name: 'Restyle Application crash' }).click();
    const dialog = page.getByRole('dialog', { name: 'Restyle "Application crash"' });
    await dialog.getByLabel('Label').fill('App crashed');
    await dialog.getByLabel('Badge (1–3 characters)').fill('X');
    await dialog.getByRole('button', { name: 'Save style' }).click();
    await expect(dialog).toHaveCount(0);
    await expect(table.locator('tr[data-kind="net.decryptology.windows/app-crash"]')).toContainText('App crashed');
    await table.getByRole('button', { name: 'Reset style of App crashed' }).click();
    await expect(table.locator('tr[data-kind="net.decryptology.windows/app-crash"]')).toContainText('Application crash');
  });
});
