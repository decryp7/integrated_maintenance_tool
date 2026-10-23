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

// A small dialog window is fitted to its content: at the height the shell asks the window for there is no vertical
// scrollbar and no empty gap below the buttons (the browser build publishes the requested height on <html>).
const FIT: [string, string, object][] = [
  ['close-prompt', 'Close Debrief?', {}],
  ['unsaved-changes', 'Unsaved changes', { name: 'Untitled', action: 'quit' }],
  ['open-ask', 'Open 1 file', { ask: { count: 1, names: ['a.log'] } }],
];
for (const [id, title, input] of FIT) {
  test(`the ${id} dialog window fits its content: no scrollbar and no gap`, async ({ page }) => {
    await page.setViewportSize({ width: 460, height: 700 });
    await page.goto(`/#/dialog?id=${id}&host=main&label=d-main-1&title=${encodeURIComponent(title)}&dialogInput=${encodeURIComponent(JSON.stringify(input))}`);
    const shell = page.locator('debrief-dialog-shell');
    await expect(shell.locator('[data-testid=dialog-loading]')).toHaveCount(0, { timeout: 15_000 });
    await expect.poll(() => page.evaluate(() => document.documentElement.dataset['fittedHeight'] ?? ''), { timeout: 10_000 }).not.toBe('');
    const fitted = Number(await page.evaluate(() => document.documentElement.dataset['fittedHeight']));
    await page.setViewportSize({ width: 460, height: fitted });
    const m = await page.evaluate(() => {
      const body = document.querySelector('debrief-dialog-shell main') as HTMLElement;
      const content = body.firstElementChild as HTMLElement;
      const pad = parseFloat(getComputedStyle(body).paddingBottom);
      return { scroll: body.scrollHeight, client: body.clientHeight, gap: body.getBoundingClientRect().bottom - pad - content.getBoundingClientRect().bottom };
    });
    expect(m.scroll).toBeLessThanOrEqual(m.client);
    expect(m.gap).toBeGreaterThanOrEqual(-1);
    expect(m.gap).toBeLessThanOrEqual(4);
  });
}

// A dialog that shows a streamed or loaded answer is not fitted to its first short state: it opens at a usable height
// from the registry, the answer scrolls inside, and the footer stays visible. A small decision dialog still fits exactly.
test('the ai-narrative window with a long answer is at least 480 px tall, scrolls inside and keeps its footer visible', async ({ page }) => {
  const text = Array.from({ length: 60 }, (_, i) => `Paragraph ${i + 1}: the payment gateway timed out after retrying the upstream call [#1].`).join('\n\n');
  const preset = { text, citations: [{ n: 1, id: 1 }], evidence: {}, usage: {} };
  await page.setViewportSize({ width: 680, height: 560 });
  await page.goto(`/#/dialog?id=ai-narrative&host=main&label=d-main-1&title=Explain%20event%201&dialogInput=${encodeURIComponent(JSON.stringify({ preset }))}`);
  const shell = page.locator('debrief-dialog-shell');
  await expect(shell.locator('[data-testid=dialog-loading]')).toHaveCount(0, { timeout: 15_000 });
  const answer = shell.getByTestId('ai-narrative');
  await expect(answer).toContainText('Paragraph 1');
  // Not fitted to the content: the registry gives it a height, so no fit is ever requested.
  expect(await page.evaluate(() => document.documentElement.dataset['fittedHeight'] ?? '')).toBe('');
  const m = await answer.evaluate((el) => ({ client: el.clientHeight, scroll: el.scrollHeight, overflowX: el.scrollWidth - el.clientWidth }));
  expect(m.client).toBeGreaterThanOrEqual(300);
  expect(m.scroll).toBeGreaterThan(m.client);
  expect(m.overflowX).toBeLessThanOrEqual(0);
  const close = shell.getByRole('button', { name: 'Close' });
  await expect(close).toBeVisible();
  const box = (await close.boundingBox())!;
  expect(box.y + box.height).toBeLessThanOrEqual(560);
  const win = (await shell.boundingBox())!;
  expect(win.height).toBeGreaterThanOrEqual(480);
});
