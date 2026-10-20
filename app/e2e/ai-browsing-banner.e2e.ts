import { expect, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';

// The "AI is browsing" banner the shell puts over the browser window (app/src-tauri/src/browser/driving.rs), in
// the browser build: the app booted at `#/ai-browsing?host=main`, with the mock browser driven by agent mode or
// research (`?mockDriving=`). The real banner window (placement, title, frame) is tested in Rust.

test.describe('AI is browsing banner', () => {
  test('shows that an agent drives the browser and Stop gives it back', async ({ page }, testInfo) => {
    await page.goto('/?mockDriving=agent#/ai-browsing?host=main');
    const banner = page.getByTestId('ai-browsing');
    await expect(banner).toContainText('AI is browsing');
    await expect(banner).toContainText("agent mode is driving");
    await expect(page.locator('imt-root')).toHaveCount(0);
    await expectNoSeriousViolations(page, testInfo, 'ai-browsing-banner', 'imt-ai-browsing-banner');
    await banner.getByRole('button', { name: 'Stop' }).click();
    await expect(banner).toHaveCount(0);
    await expect(page.getByTestId('ai-browsing-idle')).toContainText('The browser is yours again');
  });

  test('research has its own Stop', async ({ page }) => {
    await page.goto('/?mockDriving=research#/ai-browsing?host=main');
    const banner = page.getByTestId('ai-browsing');
    await expect(banner).toContainText('Research is reading result pages');
    await banner.getByRole('button', { name: 'Stop' }).click();
    await expect(banner).toHaveCount(0);
  });
});
