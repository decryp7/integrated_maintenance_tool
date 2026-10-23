import { expect, test } from '@playwright/test';

// A dialog window is its own small Angular bootstrap (ADR 0050). The browser build can open its route by URL, so this
// checks that each dialog renders with only the providers of a dialog window, with no missing-provider errors.
const IDS: [string, string][] = [
  ['settings', 'Settings'],
  ['about', 'About Debrief'],
  ['whats-new', "What's new"],
  ['updates', 'Check for updates'],
  ['cheat-sheet', 'Keyboard shortcuts'],
  ['ai-setup', 'Set up the AI assistant'],
  ['ai-models', 'AI models'],
  ['redacted-copy', 'Share redacted copy'],
  ['export', 'Export events'],
  ['report-problem', 'Report a problem'],
  ['collector', 'Collect diagnostics bundle'],
  ['alert-rules', 'Live alert rules'],
  ['open-live', 'Open for live tail'],
  ['paste', 'Paste as log'],
  ['import-range', 'Open a time range'],
  ['recent-projects', 'Recent projects'],
  ['manage-layouts', 'Manage perspectives'],
  ['save-layout', 'Save as perspective'],
  ['redaction', 'Redaction settings'],
  ['add-tile', 'Add tile'],
  ['profile-native', 'Profile native process'],
  ['profile-dotnet', 'Profile .NET process'],
];

for (const [id, title] of IDS) {
  test(`the ${id} dialog window renders with the dialog providers`, async ({ page }) => {
    const errors: string[] = [];
    page.on('console', (m) => m.type() === 'error' && errors.push(m.text()));
    page.on('pageerror', (e) => errors.push(e.message));
    await page.goto(`/#/dialog?id=${id}&host=main&label=d-main-1&title=${encodeURIComponent(title)}&dialogInput=%7B%7D`);
    const shell = page.locator('debrief-dialog-shell');
    await expect(shell).toBeVisible();
    await expect(shell.locator('[data-testid=dialog-loading]')).toHaveCount(0, { timeout: 15_000 });
    await expect(shell.locator('[data-testid=dialog-error]')).toHaveCount(0);
    expect(errors.filter((e) => /NG0\d+|No provider|NullInjector/.test(e))).toEqual([]);
  });
}

// A dialog window is a full-height column: the footer sits at the bottom of the window and the body fills the space
// between, at any window size (no sizes left over from the in-page overlay).
for (const size of [{ width: 1021, height: 727 }, { width: 760, height: 560 }]) {
  test(`the settings dialog window fills a ${size.width}x${size.height} window with its footer at the bottom`, async ({ page }) => {
    await page.setViewportSize(size);
    await page.goto('/#/dialog?id=settings&host=main&label=d-main-1&title=Settings&dialogInput=%7B%7D');
    const shell = page.locator('debrief-dialog-shell');
    await expect(shell.locator('[data-testid=dialog-loading]')).toHaveCount(0, { timeout: 15_000 });
    const footer = shell.locator('.footer');
    await expect(footer).toBeVisible();
    const fb = (await footer.boundingBox())!;
    const host = (await shell.locator('debrief-settings-screen').boundingBox())!;
    // The footer ends within the shell's padding of the window bottom; the content spans the window height.
    expect(size.height - (fb.y + fb.height)).toBeLessThanOrEqual(24);
    expect(host.height).toBeGreaterThan(size.height - 48);
  });
}
