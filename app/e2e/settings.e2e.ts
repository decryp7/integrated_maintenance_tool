import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';

// The Settings screen runs against the browser build's mock settings backend
// (src/app/settings/mock-settings-backend.ts), which mimics the engine's schema and messages.
// CSP problems are collected so the same file can also run against the production build.

function watchCsp(page: Page): string[] {
  const problems: string[] = [];
  page.on('console', (msg) => {
    if (/Content Security Policy|Refused to/i.test(msg.text())) problems.push(msg.text());
  });
  page.on('pageerror', (err) => problems.push(`pageerror: ${err.message}`));
  return problems;
}

const settingsDialog = (page: Page) => page.getByRole('dialog', { name: 'Settings' });
const sectionNav = (page: Page) => settingsDialog(page).getByRole('navigation', { name: 'Settings sections' });

async function openWithShortcut(page: Page) {
  await page.keyboard.press('ControlOrMeta+Comma');
  await expect(settingsDialog(page)).toBeVisible();
}

test('settings dialog changes theme, date format and density, and persists them', async ({ page, browserName }, testInfo) => {
  const csp = watchCsp(page);
  await openApp(page);
  await page.getByRole('button', { name: 'Settings' }).click();
  const dialog = settingsDialog(page);
  await expect(dialog).toBeVisible();
  await expectNoSeriousViolations(page, testInfo, 'settings-general', '[role=dialog]');

  await dialog.getByRole('button', { name: 'light' }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await dialog.getByRole('button', { name: 'ISO' }).click();
  await dialog.getByRole('button', { name: 'compact' }).click();
  await expect(page.locator('imt-events-grid')).toHaveCSS('--row-h', '22px');
  await expect(page.locator('imt-events-grid .row .time').first()).toHaveText(/^\d{4}-\d{2}-\d{2} /);
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  // Safari does not focus clicked buttons, so there is nothing to restore focus to there.
  if (browserName === 'chromium') await expect(page.getByRole('button', { name: 'Settings' })).toBeFocused();

  await page.reload();
  await expect(page.locator('imt-events-grid .row .time').first()).toHaveText(/^\d{4}-\d{2}-\d{2} /);
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  expect(csp).toEqual([]);
});

test('settings can reset the layout', async ({ page }) => {
  await openApp(page);
  await page.getByRole('button', { name: 'Settings' }).click();
  await settingsDialog(page).getByRole('button', { name: 'Reset layout' }).click();
  await expect(settingsDialog(page)).toHaveCount(0);
  await expect(page.getByRole('tab', { name: 'Timeline', selected: true })).toBeVisible();
});

test('engine settings: shortcut, validation error, apply, reset and restart banner', async ({ page }, testInfo) => {
  const csp = watchCsp(page);
  await openApp(page);
  await openWithShortcut(page);
  const dialog = settingsDialog(page);

  await sectionNav(page).getByRole('button', { name: 'Diagnostics', exact: true }).click();
  await expect(dialog.getByRole('heading', { name: 'Diagnostics' })).toBeVisible();
  await expectNoSeriousViolations(page, testInfo, 'settings-diagnostics', '[role=dialog]');

  // An out-of-range value: the engine's message is shown inline and nothing is applied.
  const size = dialog.getByLabel('Log file size', { exact: true });
  await size.fill('500');
  await dialog.getByRole('button', { name: 'Apply' }).click();
  await expect(dialog.getByText('Must be between 1 MB and 100 MB.')).toBeVisible();
  await expect(size).toHaveAttribute('aria-invalid', 'true');

  // A valid value applies; a restart-only setting raises the restart banner.
  await size.fill('20');
  await expect(dialog.getByText('Must be between 1 MB and 100 MB.')).toHaveCount(0);
  await dialog.getByRole('button', { name: 'Apply' }).click();
  await expect(dialog.getByText('Settings applied.')).toBeVisible();
  await expect(dialog.getByRole('status').filter({ hasText: 'take effect after IMT restarts' })).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Restart' })).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Apply' })).toBeDisabled();

  // Reset one field to its default (an edit to apply), then the whole section.
  await dialog.getByRole('button', { name: 'Reset Log file size to default' }).click();
  await expect(size).toHaveValue('10');
  await expect(dialog.getByText('Unapplied changes in 1 section')).toBeVisible();
  await dialog.getByLabel('Log level', { exact: true }).selectOption('debug');
  await dialog.getByRole('button', { name: 'Reset section' }).click();
  await expect(dialog.getByText('Diagnostics reset to defaults.')).toBeVisible();
  await expect(dialog.getByLabel('Log level', { exact: true })).toHaveValue('info');
  await expect(size).toHaveValue('10');

  // Advanced settings are hidden until asked for.
  await expect(dialog.getByLabel('Snapshot storage', { exact: true })).toHaveCount(0);
  await dialog.getByLabel('Show advanced', { exact: true }).check();
  await expect(dialog.getByLabel('Snapshot storage', { exact: true })).toBeVisible();
  expect(csp).toEqual([]);
});

test('engine settings: search, plugin sections and the unsaved-changes guard', async ({ page }, testInfo) => {
  await openApp(page);
  await openWithShortcut(page);
  const dialog = settingsDialog(page);

  // Search matches labels, descriptions and keys across sections.
  await dialog.getByRole('searchbox', { name: 'Search settings' }).fill('relog');
  await expect(dialog.getByRole('heading', { name: 'Windows diagnostics' })).toBeVisible();
  await expect(dialog.locator('imt-setting-field')).toHaveCount(1);
  await dialog.getByRole('searchbox', { name: 'Search settings' }).fill('liveTailDebounce');
  await expect(dialog.getByLabel('Live tail delay', { exact: true })).toBeVisible();
  await dialog.getByRole('searchbox', { name: 'Search settings' }).fill('');

  // A plugin's own section, edited in its unit (seconds), stored in milliseconds.
  await sectionNav(page).getByRole('button', { name: 'Windows diagnostics', exact: true }).click();
  await expect(dialog.getByText('5 s – 1 h')).toBeVisible();
  await dialog.getByLabel('relog timeout', { exact: true }).fill('300');
  await expect(sectionNav(page).getByRole('img', { name: 'unapplied changes' })).toBeVisible();
  await dialog.getByRole('button', { name: 'Apply' }).click();
  await expect(dialog.getByText('Settings applied.')).toBeVisible();
  await expect(dialog.getByLabel('relog timeout', { exact: true })).toHaveValue('300');

  // A disabled plugin's section is shown read-only with a note; an absent module says so.
  await sectionNav(page).getByRole('button', { name: /Example plugin/ }).click();
  await expect(dialog.getByRole('note')).toContainText('disabled');
  await expect(dialog.getByLabel('Prefix', { exact: true })).toBeDisabled();
  await sectionNav(page).getByRole('button', { name: /AI assistant/ }).click();
  await expect(dialog.getByRole('note')).toContainText('not installed');
  await expectNoSeriousViolations(page, testInfo, 'settings-plugins', '[role=dialog]');

  // Unapplied edits are not lost by Escape: the screen asks first.
  await sectionNav(page).getByRole('button', { name: 'Workspace', exact: true }).click();
  await dialog.getByLabel('Live tail delay', { exact: true }).fill('900');
  await dialog.getByLabel('Live tail delay', { exact: true }).press('Escape');
  const guard = dialog.getByRole('alertdialog', { name: 'Unapplied changes' });
  await expect(guard).toBeVisible();
  await guard.getByRole('button', { name: 'Keep editing' }).click();
  await expect(settingsDialog(page)).toBeVisible();
  await dialog.getByRole('button', { name: 'Cancel' }).click();
  await guard.getByRole('button', { name: 'Discard changes' }).click();
  await expect(settingsDialog(page)).toHaveCount(0);

  // Reopened: the discarded value is gone, the applied one stays.
  await openWithShortcut(page);
  await sectionNav(page).getByRole('button', { name: 'Workspace', exact: true }).click();
  await expect(dialog.getByLabel('Live tail delay', { exact: true })).toHaveValue('750');
});

test('engine settings: export and import', async ({ page }) => {
  await openApp(page);
  await openWithShortcut(page);
  const dialog = settingsDialog(page);
  await dialog.getByRole('button', { name: 'Export…' }).click();
  await expect(dialog.getByText(/Settings exported to/)).toBeVisible();
  await dialog.getByRole('button', { name: 'Import…' }).click();
  await expect(dialog.getByText(/^Imported \d+ sections/)).toBeVisible();
});

test('settings: switches keep their labels beside them, readable and clear of badges, in every section', async ({ page }) => {
  await page.setViewportSize({ width: 1024, height: 700 });
  await openApp(page);
  await openWithShortcut(page);
  const dialog = settingsDialog(page);
  const sections = sectionNav(page).getByRole('button');
  let checked = 0;
  for (let i = 0; i < (await sections.count()); i++) {
    await sections.nth(i).click();
    const problems = await dialog.evaluate((root) => {
      const out: string[] = [];
      for (const field of root.querySelectorAll('imt-setting-field')) {
        const input = field.querySelector('input[role=switch]');
        const text = field.querySelector('imt-switch .text');
        if (!input || !text) continue;
        const i = input.getBoundingClientRect();
        const t = text.getBoundingClientRect();
        const name = text.textContent?.trim();
        if (i.width < 26 || i.height < 14) out.push(`${name}: switch is ${i.width}x${i.height}`);
        if (t.left < i.right) out.push(`${name}: label starts under the switch`);
        if (t.height > 40) out.push(`${name}: label wraps to ${Math.round(t.height)}px`);
        for (const b of field.querySelectorAll('.badge, .reset')) {
          const r = b.getBoundingClientRect();
          if (r.left < t.right && r.right > t.left && r.top < t.bottom && r.bottom > t.top) out.push(`${name}: overlaps "${b.textContent?.trim()}"`);
        }
      }
      return out;
    });
    expect(problems).toEqual([]);
    checked += await dialog.locator('imt-setting-field input[role=switch]').count();
  }
  expect(checked).toBeGreaterThan(3);
});
