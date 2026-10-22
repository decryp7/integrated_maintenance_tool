import { expect, Page, test } from '@playwright/test';
import { gridTotal, initialTotal, openApp, parseCount } from './app';
import { expectNoSeriousViolations } from './axe';
import { chooseMenu } from './menu-support';

/** Turns "Home: show after every import" on (it is off by default). */
async function enableAutoOpen(page: Page): Promise<void> {
  await setAutoOpen(page, true);
}

async function setAutoOpen(page: Page, on: boolean): Promise<void> {
  await page.getByRole('button', { name: 'Settings' }).click();
  const dialog = page.getByRole('dialog', { name: 'Settings' });
  const auto = dialog.getByRole('checkbox', { name: /Home: show after every import/ });
  if (on) await auto.check();
  else await auto.uncheck();
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
}

test.describe('home (dashboard and health check)', () => {
  test('by default: the headline is on the timeline header and Review opens Home', async ({ page }, testInfo) => {
    await openApp(page);
    const total = parseCount(await initialTotal(page));
    await page.locator('imt-sidebar').getByRole('button', { name: 'Open files' }).click();
    await expect(page.locator('imt-sidebar')).toContainText('msinfo32.nfo', { timeout: 10_000 });
    await expect(page.locator('imt-home-view')).toHaveCount(0);
    const headline = page.getByRole('toolbar', { name: 'Health check' });
    await expect(headline).toContainText(/crash/);
    await expectNoSeriousViolations(page, testInfo, 'home-headline', '[data-testid=health-headline]');
    await headline.getByRole('button', { name: /errors?$/ }).click();
    await expect.poll(async () => parseCount(await gridTotal(page).innerText())).toBeLessThan(total);
    await headline.getByRole('button', { name: 'Review' }).click();
    await expect(page.locator('imt-home-view')).toBeVisible();
    await expect(headline).toHaveCount(0);
  });

  test('opens after an import when turned on, shows the dashboard, and its actions work', async ({ page }, testInfo) => {
    await openApp(page);
    await enableAutoOpen(page);
    const total = parseCount(await initialTotal(page));
    await expect(page.locator('imt-home-view')).toHaveCount(0);

    await page.locator('imt-sidebar').getByRole('button', { name: 'Open files' }).click();
    const home = page.locator('imt-home-view');
    await expect(home).toBeVisible({ timeout: 10_000 });
    const region = home.getByRole('region', { name: 'Home dashboard' });
    await expect(region.getByText('Sources', { exact: true })).toBeVisible();
    await expect(region).toContainText('msinfo32.nfo');
    // The tiles and the history chart come from the engine's Home summary.
    await expect(region.getByTestId('home-tiles')).toContainText('Events');
    await expect(region.getByTestId('home-history')).toBeVisible();
    const findings = region.getByTestId('home-findings');
    const critical = findings.locator('.finding[data-severity=critical]');
    await expect(critical.first()).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'home', 'imt-home-view');

    // "Show these events" applies the finding's time range: fewer events match.
    await critical.getByRole('button', { name: 'Show these events' }).first().click();
    await expect.poll(async () => parseCount(await gridTotal(page).innerText())).toBeLessThan(total);

    // The unrecognised file's action opens the format wizard.
    // The list is virtualised: scroll to the end for the info findings.
    await findings.evaluate((el) => el.scrollTo(0, el.scrollHeight));
    await findings.locator('.finding[data-severity=info]').getByRole('button', { name: 'Set up format…' }).first().click();
    await expect(page.getByRole('dialog', { name: 'Set up format' })).toBeVisible();
  });

  test('can be turned off for later imports', async ({ page }) => {
    await openApp(page);
    await enableAutoOpen(page);
    await page.locator('imt-sidebar').getByRole('button', { name: 'Open files' }).click();
    const home = page.locator('imt-home-view');
    await expect(home).toBeVisible({ timeout: 10_000 });
    await home.getByRole('checkbox', { name: 'Show Home after opening files' }).uncheck();
    // Close the panel (View ▸ Home toggles it), open more files: it stays closed.
    await chooseMenu(page, 'View', 'Home');
    await expect(home).toHaveCount(0);
    await page.locator('imt-sidebar').getByRole('button', { name: 'Open folder' }).click();
    await expect(page.locator('imt-sidebar')).toContainText('extra-folder', { timeout: 10_000 });
    await expect(home).toHaveCount(0);
  });

  test('can be turned off in General settings', async ({ page }) => {
    await openApp(page);
    await enableAutoOpen(page);
    await enableAutoOpen(page);
    await setAutoOpen(page, false);
    await page.locator('imt-sidebar').getByRole('button', { name: 'Open files' }).click();
    await expect(page.locator('imt-sidebar')).toContainText('msinfo32.nfo', { timeout: 10_000 });
    await expect(page.locator('imt-home-view')).toHaveCount(0);
  });
});

test.describe('home quick actions and history', () => {
  // Known gap (open): since Home is a document beside the timeline, a click in its history chart no longer narrows the range here.
  test.fixme('quick actions open files and a click on the history sets the time range', async ({ page }) => {
    await openApp(page);
    await enableAutoOpen(page);
    const total = parseCount(await initialTotal(page));
    await page.locator('imt-sidebar').getByRole('button', { name: 'Open files' }).click();
    const home = page.locator('imt-home-view');
    await expect(home).toBeVisible({ timeout: 10_000 });
    await expect(home.getByTestId('home-open-files')).toBeVisible();
    // Open last project needs a recent project: disabled on a fresh profile.
    await expect(home.getByTestId('home-open-last')).toBeDisabled();
    // A click inside the history chart narrows the time range to that bar.
    const chart = home.getByTestId('home-history');
    await expect(chart).toBeVisible();
    const box = await chart.boundingBox();
    expect(box).toBeTruthy();
    // Bars may not sit under the exact centre of a wide chart: try a few places along it until one narrows the range.
    const narrowed = async () => parseCount(await gridTotal(page).innerText()) < total;
    for (const at of [0.5, 0.3, 0.7, 0.15, 0.85]) {
      await page.mouse.click(box!.x + box!.width * at, box!.y + box!.height * 0.5);
      if (await expect.poll(narrowed, { timeout: 1500 }).toBe(true).then(() => true, () => false)) break;
    }
    await expect.poll(narrowed).toBe(true);
  });
});
