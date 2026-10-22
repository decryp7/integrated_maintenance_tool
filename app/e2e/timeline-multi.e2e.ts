import { expect, Locator, Page, test } from '@playwright/test';
import { openApp } from './app';
import { chooseMenu } from './menu-support';

/** The button in the filter bar that shows (and resets) the workspace time range. */
const rangeButton = (page: Page) => page.getByTitle(/Reset to the full range|Drag on the timeline to zoom/);

/** Drags across a timeline's histogram strip (no markers there) between two fractions of the plot width. */
async function brush(page: Page, panel: Locator, from: number, to: number): Promise<void> {
  const box = (await panel.locator('.hist-row .plot').boundingBox())!;
  const y = box.y + box.height / 2;
  await page.mouse.move(box.x + box.width * from, y);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * ((from + to) / 2), y, { steps: 5 });
  await page.mouse.move(box.x + box.width * to, y, { steps: 5 });
  await page.mouse.up();
}

const fromOf = (panel: Locator) => panel.locator('.wrap').getAttribute('data-from');
const textOf = async (loc: Locator) => ((await loc.textContent()) ?? '').trim();

test.describe('multiple timelines', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('a second timeline with its own query zooms with the main one until it is unlinked', async ({ page }) => {
    // Two timelines stacked need the height for both histogram strips to be on screen (the brush is a real mouse drag).
    await page.setViewportSize({ width: 1440, height: 1200 });
    const main = page.locator('imt-timeline-panel:not(.instance)');
    await main.getByRole('button', { name: 'Timeline menu' }).click();
    await page.getByRole('menuitem', { name: 'New timeline' }).click();

    const second = page.locator('imt-timeline-panel.instance');
    await expect(second).toBeVisible();
    await expect(main).toBeVisible();
    await expect(second.getByTestId('timeline-title')).toHaveText('Timeline 2');
    await expect(page.getByRole('tab', { name: 'Timeline 2' })).toBeVisible();

    // Its own query: only errors and worse, so its lanes match fewer events than the main timeline's.
    await second.getByLabel('Events shown').selectOption('own');
    const ql = second.getByLabel('Timeline query');
    await ql.fill('level >= error');
    await ql.press('Enter');
    await expect(second.locator('.lane .meta').first()).toContainText(' of ');
    await expect(main.locator('.lane .meta').first()).not.toContainText(' of ');

    // Rename it.
    await second.getByTestId('timeline-title').dblclick();
    const title = second.getByLabel('Timeline title');
    await title.fill('Errors only');
    await title.press('Enter');
    await expect(page.getByRole('tab', { name: 'Errors only' })).toBeVisible();

    // Linked: zooming the second timeline zooms the workspace and the main timeline.
    const range = rangeButton(page);
    const full = await textOf(range);
    const mainFull = await fromOf(main);
    await brush(page, second, 0.3, 0.6);
    await expect(range).not.toHaveText(full);
    await expect.poll(() => fromOf(main)).not.toBe(mainFull);
    await expect.poll(async () => (await fromOf(second)) === (await fromOf(main))).toBe(true);

    // Unlinked: the second timeline zooms on its own; the main timeline and the workspace range stay.
    await second.getByRole('switch', { name: 'Sync time' }).uncheck();
    await expect(second.getByText('Unlinked')).toBeVisible();
    const linkedRange = await textOf(range);
    const mainZoomed = await fromOf(main);
    await brush(page, second, 0.4, 0.7);
    await expect.poll(() => fromOf(second)).not.toBe(mainZoomed);
    expect(await textOf(range)).toBe(linkedRange);
    expect(await fromOf(main)).toBe(mainZoomed);

    // Zooming the main timeline no longer moves the second one.
    const secondOwn = await fromOf(second);
    await main.getByRole('button', { name: 'Fit' }).click();
    await expect(range).toHaveText(full);
    expect(await fromOf(second)).toBe(secondOwn);
  });

  test('"New timeline" is in the View menu and the panel survives a reload', async ({ page }) => {
    await chooseMenu(page, 'View', 'New timeline');
    const second = page.locator('imt-timeline-panel.instance');
    await expect(second).toBeVisible();
    await second.getByLabel('Lanes').selectOption('level');
    await expect(second.locator('.lane-label .key').first()).toHaveText(/^(Verbose|Debug|Information|Warning|Error|Fatal)$/);

    // The layout is saved shortly after it changes.
    await expect.poll(() => page.evaluate(() => localStorage.getItem('imt.layout')?.includes('timeline#2') ?? false)).toBe(true);
    await page.reload();
    await expect(page.locator('imt-timeline-panel.instance')).toBeVisible();
    await expect(page.locator('imt-timeline-panel.instance').getByLabel('Lanes')).toHaveValue('level');
  });
});
