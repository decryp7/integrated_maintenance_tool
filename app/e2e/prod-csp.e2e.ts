import { expect, test } from '@playwright/test';
import { openApp } from './app';

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
  await expect(page.locator('imt-timeline-panel .lane-label')).toHaveCount(7);
  const bg = await page.evaluate(() => getComputedStyle(document.body).backgroundColor);
  expect(bg).not.toBe('rgba(0, 0, 0, 0)');
  await page.getByRole('tab', { name: /Metrics/ }).click();
  await expect(page.locator('imt-chart-card canvas').first()).toBeVisible();
  await page.getByRole('button', { name: 'Profile editor' }).click();
  await expect(page.getByText('Parsing profile')).toBeVisible();

  const violations = await page.evaluate(() => (window as unknown as { __cspViolations: string[] }).__cspViolations);
  expect([...problems, ...violations.map((v) => `violation: ${v}`)]).toEqual([]);
});
