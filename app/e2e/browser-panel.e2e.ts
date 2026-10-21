import { expect, Page, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { openAiApp, runCommand } from './ai-support';

// The docked Browser panel (ADR 0024) against the mock backends: the panel's toolbar, "AI is browsing" with Stop in
// its header and on its tab, and where the page would be. The browser build has no native page webview: the mock
// backend keeps the panel's last `browser_place` in `window.__imtMockBrowserView`, so these tests check what the
// shell would be told (the page area, and whether the page may show).

interface View {
  visible: boolean;
  shown: boolean;
  rect: { x: number; y: number; width: number; height: number } | null;
  scale: number;
}

const view = (page: Page) => page.evaluate(() => (window as unknown as { __imtMockBrowserView?: View }).__imtMockBrowserView ?? null);

async function openPanel(page: Page) {
  await runCommand(page, 'Web: Open the browser panel');
  const panel = page.locator('imt-web-panel');
  await expect(panel.getByTestId('web-address')).toBeVisible();
  return panel;
}

test.describe('Browser panel', () => {
  test('docks with a toolbar, places the page over its area and hides it while an overlay covers it', async ({ page }, testInfo) => {
    await openAiApp(page);
    const panel = await openPanel(page);
    await expect(panel.getByTestId('web-empty')).toContainText('it shows here');
    await expect.poll(async () => (await view(page))?.visible).toBe(false);
    await panel.getByTestId('web-address').fill('TimeoutException gateway');
    await panel.getByRole('button', { name: 'Go' }).click();
    await expect(panel.getByTestId('web-page')).toContainText('google.com/search?q=TimeoutException+gateway');
    // The page shows over the page area, in CSS pixels with the device pixel ratio.
    await expect.poll(async () => (await view(page))?.shown).toBe(true);
    const area = await panel.getByTestId('web-page-area').boundingBox();
    const v = await view(page);
    expect(v?.rect?.width).toBeCloseTo(area!.width, 0);
    expect(v?.rect?.y).toBeCloseTo(area!.y, 0);
    expect(v?.scale).toBeGreaterThan(0);
    await expectNoSeriousViolations(page, testInfo, 'browser-panel', 'imt-web-panel');
    // The command palette opens over the page: it steps aside, and the panel says why.
    await page.keyboard.press('ControlOrMeta+k');
    await expect(page.getByRole('combobox', { name: 'Type a command' })).toBeFocused();
    await expect.poll(async () => (await view(page))?.shown).toBe(false);
    await expect(panel.getByTestId('web-page-hidden')).toContainText('hidden while a menu');
    await page.keyboard.press('Escape');
    await expect.poll(async () => (await view(page))?.shown).toBe(true);
    await expect(panel.getByTestId('web-page-hidden')).toHaveCount(0);
    // Ctrl+L puts the keyboard in the address bar.
    await panel.getByTestId('web-page-area').focus();
    await page.keyboard.press('ControlOrMeta+l');
    await expect(panel.getByTestId('web-address')).toBeFocused();
    // Closing the page hides it; the panel stays.
    await panel.getByTestId('web-close').click();
    await expect(panel.getByTestId('web-empty')).toBeVisible();
    await expect.poll(async () => (await view(page))?.shown).toBe(false);
  });

  test('says the AI is browsing in its header and on its tab, and Stop gives the page back', async ({ page }, testInfo) => {
    await openAiApp(page, 'mockDriving=agent');
    const panel = await openPanel(page);
    const banner = panel.getByTestId('ai-browsing');
    await expect(banner).toContainText('AI is browsing');
    await expect(banner).toContainText('agent mode is driving');
    await expect(banner).toHaveAttribute('role', 'status');
    await expect(page.locator('.imt-tab[data-panel-id="web"] .imt-tab-label')).toHaveText('Browser · AI is browsing');
    await expectNoSeriousViolations(page, testInfo, 'browser-panel-driving', 'imt-web-panel');
    await banner.getByRole('button', { name: 'Stop the AI browsing' }).click();
    await expect(banner).toHaveCount(0);
    await expect(page.locator('.imt-tab[data-panel-id="web"] .imt-tab-label')).toHaveText('Browser');
  });

  test('research has its own Stop', async ({ page }) => {
    await openAiApp(page, 'mockDriving=research');
    const panel = await openPanel(page);
    const banner = panel.getByTestId('ai-browsing');
    await expect(banner).toContainText('Research is reading result pages');
    await banner.getByRole('button', { name: 'Stop the AI browsing' }).click();
    await expect(banner).toHaveCount(0);
  });
});
