import { expect, test } from '@playwright/test';
import { openApp, openView } from './app';

// Regression guard: the production build must run under the Tauri CSP (see serve-prod.mjs).
// Angular's critical-CSS inlining once emitted an inline handler that this CSP blocked, which
// left the real app unstyled; any CSP violation or page error here fails the build.
test('production build loads under the Tauri CSP without violations', async ({ page }) => {
  const problems: string[] = [];
  page.on('console', (msg) => {
    const text = msg.text();
    if (/Content Security Policy|Refused to/i.test(text)) problems.push(`console: ${text}`);
  });
  page.on('pageerror', (err) => problems.push(`pageerror: ${err.message}`));
  await page.addInitScript(() => {
    const w = window as unknown as { __cspViolations: string[] };
    w.__cspViolations = [];
    document.addEventListener('securitypolicyviolation', (e) => w.__cspViolations.push(`${e.violatedDirective} ${e.blockedURI}`));
  });

  const response = await page.goto('/');
  expect(response?.headers()['content-security-policy']).toContain("default-src 'self'");
  await openApp(page);

  // Smoke: shell, timeline lanes, styles applied (fonts and theme tokens resolved).
  await expect(page.getByRole('button', { name: 'Explorer' })).toBeVisible();
  await expect(page.locator('debrief-timeline-panel .lane-label')).toHaveCount(7);
  const bg = await page.evaluate(() => getComputedStyle(document.body).backgroundColor);
  expect(bg).not.toBe('rgba(0, 0, 0, 0)');
  await openView(page, 'metrics');
  await expect(page.locator('debrief-chart-card canvas').first()).toBeVisible();
  await page.getByRole('button', { name: 'Profile editor' }).click();
  await expect(page.getByText('Parsing profile')).toBeVisible();

  const violations = await page.evaluate(() => (window as unknown as { __cspViolations: string[] }).__cspViolations);
  expect([...problems, ...violations.map((v) => `violation: ${v}`)]).toEqual([]);
});

// The drag ghost's page (`public/drag-ghost.html`, shown in the shell's ghost window) also runs under the CSP: its
// script and styles are files of the app, and what the shell hands it is shown as text, an SVG path and an image.
test('the drag ghost page renders its content under the Tauri CSP without violations', async ({ page }) => {
  const problems: string[] = [];
  page.on('console', (msg) => {
    if (/Content Security Policy|Refused to/i.test(msg.text())) problems.push(`console: ${msg.text()}`);
  });
  page.on('pageerror', (err) => problems.push(`pageerror: ${err.message}`));
  await page.addInitScript(() => {
    const w = window as unknown as { __cspViolations: string[] };
    w.__cspViolations = [];
    document.addEventListener('securitypolicyviolation', (e) => w.__cspViolations.push(`${e.violatedDirective} ${e.blockedURI}`));
  });
  const response = await page.goto('/drag-ghost.html');
  expect(response?.headers()['content-security-policy']).toContain("default-src 'self'");
  await page.waitForFunction(() => typeof (window as unknown as { debriefGhost?: unknown }).debriefGhost === 'function');
  // A 1×1 PNG.
  const png = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==';
  await page.evaluate(
    (snapshot) => (window as unknown as { debriefGhost: (c: unknown) => void }).debriefGhost({ title: '<b>Inspector</b>', icon: 'M6 9l6 6l6-6', snapshot, dark: true }),
    png,
  );
  await expect(page.locator('.title')).toHaveText('<b>Inspector</b>');
  await expect(page.locator('.title b')).toHaveCount(0);
  await expect(page.locator('html')).toHaveAttribute('data-scheme', 'dark');
  await expect(page.locator('.icon path')).toHaveAttribute('d', 'M6 9l6 6l6-6');
  await expect(page.locator('.snapshot')).toBeVisible();
  expect(await page.locator('.snapshot').evaluate((img: HTMLImageElement) => img.complete && img.naturalWidth)).toBe(1);
  expect(await page.evaluate(() => getComputedStyle(document.body).backgroundColor)).toBe('rgba(0, 0, 0, 0)');
  const violations = await page.evaluate(() => (window as unknown as { __cspViolations: string[] }).__cspViolations);
  expect([...problems, ...violations.map((v) => `violation: ${v}`)]).toEqual([]);
});
