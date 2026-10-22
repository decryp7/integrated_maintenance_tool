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

test('settings dialog changes the theme, colour scheme, date format and density, and persists them', async ({ page, browserName }, testInfo) => {
  const csp = watchCsp(page);
  await openApp(page);
  await page.getByRole('button', { name: 'Settings' }).click();
  const dialog = settingsDialog(page);
  await expect(dialog).toBeVisible();
  await expectNoSeriousViolations(page, testInfo, 'settings-general', '[role=dialog]');

  await dialog.getByRole('button', { name: 'ISO' }).click();
  await dialog.getByRole('button', { name: 'Appearance' }).click();
  await expectNoSeriousViolations(page, testInfo, 'settings-appearance', '[role=dialog]');
  await dialog.getByRole('button', { name: 'light' }).click();
  await expect(page.locator('html')).toHaveAttribute('data-scheme', 'light');
  await dialog.getByRole('radio', { name: /Console/ }).check();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'console');
  await expect(page.locator('html')).toHaveAttribute('data-surface', 'cards');
  await dialog.getByRole('button', { name: 'compact' }).click();
  await expect(page.locator('imt-events-grid')).toHaveCSS('--row-h', '22px');
  await expect(page.locator('html')).toHaveAttribute('data-density', 'compact');
  await expect(page.locator('imt-events-grid .row .time').first()).toHaveText(/^\d{4}-\d{2}-\d{2} /);
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  // Safari does not focus clicked buttons, so there is nothing to restore focus to there.
  if (browserName === 'chromium') await expect(page.getByRole('button', { name: 'Settings' })).toBeFocused();

  await page.reload();
  await expect(page.locator('imt-events-grid .row .time').first()).toHaveText(/^\d{4}-\d{2}-\d{2} /);
  await expect(page.locator('html')).toHaveAttribute('data-scheme', 'light');
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'console');
  expect(csp).toEqual([]);
});

test('the selected segment of a segmented control is a tint with accent text, never a filled primary (G3)', async ({ page }) => {
  await openApp(page);
  await openWithShortcut(page);
  const dialog = settingsDialog(page);
  await sectionNav(page).getByRole('button', { name: 'Appearance' }).click();
  const scheme = dialog.getByTestId('scheme-setting');
  await scheme.getByRole('button', { name: 'dark' }).click();
  const selected = scheme.getByRole('button', { name: 'dark' });
  await expect(selected).toHaveAttribute('aria-pressed', 'true');
  const tint = await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--chip-bg').trim());
  const accent = await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--accent').trim());
  const bg = (b: typeof selected) => b.evaluate((el) => getComputedStyle(el).backgroundColor);
  const probe = await page.evaluate((c) => {
    const d = document.createElement('div');
    d.style.color = c;
    document.body.append(d);
    const rgb = getComputedStyle(d).color;
    d.remove();
    return rgb;
  }, tint);
  const filled = await page.evaluate((c) => {
    const d = document.createElement('div');
    d.style.color = c;
    document.body.append(d);
    const rgb = getComputedStyle(d).color;
    d.remove();
    return rgb;
  }, accent);
  expect(await bg(selected)).toBe(probe);
  expect(await bg(selected)).not.toBe(filled);
  expect(await bg(scheme.getByRole('button', { name: 'light' }))).not.toBe(probe);
});

test('settings can reset the layout', async ({ page }) => {
  await openApp(page);
  await page.getByRole('button', { name: 'Settings' }).click();
  await settingsDialog(page).getByRole('button', { name: 'Reset layout' }).click();
  await expect(settingsDialog(page)).toHaveCount(0);
  await expect(page.getByRole('tab', { name: 'Timeline', selected: true })).toBeVisible();
});

test('engine settings: shortcut, validation error, automatic apply with Undo, reset and restart bar', async ({ page }, testInfo) => {
  const csp = watchCsp(page);
  await openApp(page);
  await openWithShortcut(page);
  const dialog = settingsDialog(page);

  await sectionNav(page).getByRole('button', { name: 'Diagnostics', exact: true }).click();
  await expect(dialog.getByRole('heading', { name: 'Diagnostics' })).toBeVisible();
  // There is no Apply button: changes apply by themselves.
  await expect(dialog.getByRole('button', { name: 'Apply' })).toHaveCount(0);
  await expectNoSeriousViolations(page, testInfo, 'settings-diagnostics', '[role=dialog]');

  // An out-of-range value: the engine's message is shown inline and nothing is applied.
  const size = dialog.getByLabel('Log file size', { exact: true });
  await size.fill('500');
  await expect(dialog.getByText('Must be between 1 MB and 100 MB.')).toBeVisible();
  await expect(size).toHaveAttribute('aria-invalid', 'true');
  await expect(dialog.getByText('Not saved in 1 section')).toBeVisible();

  // A valid value applies by itself with Undo; a restart-only setting raises the restart bar.
  await size.fill('20');
  await expect(dialog.getByText('Must be between 1 MB and 100 MB.')).toHaveCount(0);
  await expect(dialog.getByTestId('settings-notice')).toContainText('Log file size saved.');
  await expect(dialog.getByTestId('settings-restart-bar')).toContainText('after IMT restarts');
  await expect(dialog.getByRole('button', { name: 'Restart now' })).toBeVisible();

  // Undo puts the previous value back; Reset to default is an edit like any other; Reset section is immediate.
  await dialog.getByTestId('settings-undo').click();
  await expect(size).toHaveValue('10');
  await size.fill('30');
  await expect(dialog.getByTestId('settings-notice')).toContainText('saved');
  await dialog.getByRole('button', { name: 'Reset to default: Log file size' }).click();
  await expect(size).toHaveValue('10');
  await dialog.getByLabel(/^Log level/).click();
  await page.getByRole('option', { name: /debug/i }).click();
  await expect(dialog.getByTestId('settings-notice')).toContainText('saved.');
  await dialog.getByRole('button', { name: 'Reset section' }).click();
  await expect(dialog.getByText('Diagnostics reset to defaults.')).toBeVisible();
  await expect(dialog.getByLabel(/^Log level/)).toContainText(/info/i);
  await expect(size).toHaveValue('10');

  // Advanced settings are hidden until asked for.
  await expect(dialog.getByLabel('Snapshot storage', { exact: true })).toHaveCount(0);
  await dialog.getByLabel('Show advanced', { exact: true }).check();
  await expect(dialog.getByLabel('Snapshot storage', { exact: true })).toBeVisible();
  expect(csp).toEqual([]);
});

test('engine settings: search, plugin sections and the guard for refused edits', async ({ page }, testInfo) => {
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

  // A plugin's own section, edited in its unit (seconds), stored in milliseconds, applied by itself.
  await sectionNav(page).getByRole('button', { name: 'Windows diagnostics', exact: true }).click();
  await expect(dialog.getByText('5 s – 1 h')).toBeVisible();
  await dialog.getByLabel('relog timeout', { exact: true }).fill('300');
  await expect(dialog.getByTestId('settings-notice')).toContainText('saved');
  await expect(dialog.getByLabel('relog timeout', { exact: true })).toHaveValue('300');

  // A disabled plugin's section is shown read-only with a note; an absent module says so.
  await sectionNav(page).getByRole('button', { name: /Example plugin/ }).click();
  await expect(dialog.getByRole('note')).toContainText('disabled');
  await expect(dialog.getByLabel('Prefix', { exact: true })).toBeDisabled();
  await sectionNav(page).getByRole('button', { name: /AI assistant/ }).click();
  await expect(dialog.getByRole('note')).toContainText('not installed');
  await expectNoSeriousViolations(page, testInfo, 'settings-plugins', '[role=dialog]');

  // Edits the engine refused are not lost by Escape: the screen asks first. Valid ones are already saved.
  await sectionNav(page).getByRole('button', { name: 'Live tail', exact: true }).click();
  await dialog.getByLabel('Live tail delay', { exact: true }).fill('5');
  await expect(dialog.getByText('Must be between 100 ms and 10 s.')).toBeVisible();
  await dialog.getByLabel('Live tail delay', { exact: true }).press('Escape');
  const guard = dialog.getByRole('alertdialog', { name: 'Unsaved changes' });
  await expect(guard).toBeVisible();
  await guard.getByRole('button', { name: 'Keep editing' }).click();
  await expect(settingsDialog(page)).toBeVisible();
  await dialog.getByRole('button', { name: 'Close' }).last().click();
  await guard.getByRole('button', { name: 'Discard changes' }).click();
  await expect(settingsDialog(page)).toHaveCount(0);

  // Reopened: the refused value is gone, the applied one stays.
  await openWithShortcut(page);
  await sectionNav(page).getByRole('button', { name: 'Live tail', exact: true }).click();
  await expect(dialog.getByLabel('Live tail delay', { exact: true })).toHaveValue('750');
});

test('engine settings: export and import', async ({ page }) => {
  await openApp(page);
  await openWithShortcut(page);
  const dialog = settingsDialog(page);
  await dialog.getByRole('button', { name: 'Export settings…' }).click();
  await expect(dialog.getByText(/Settings exported to/)).toBeVisible();
  await dialog.getByRole('button', { name: 'Import settings…' }).click();
  await expect(dialog.getByText(/^Imported \d+ sections/)).toBeVisible();
});

test('settings: switches keep their labels beside them, readable and clear of badges, in every section', async ({ page }) => {
  await page.setViewportSize({ width: 1024, height: 700 });
  await openApp(page);
  await openWithShortcut(page);
  const dialog = settingsDialog(page);
  const sections = sectionNav(page).getByRole('button');
  let checked = 0;
  // Wait for the section list (the plugin sections load after the dialog opens).
  await expect(sections.nth(1)).toBeVisible();
  for (let i = 0; i < (await sections.count()); i++) {
    const button = sections.nth(i);
    await button.click();
    // Measure only once the clicked section has rendered (its heading shows), not the previous one.
    await expect(button).toHaveAttribute('aria-current', 'page');
    const name = button.locator('.name');
    const title = ((await name.count()) ? await name.innerText() : await button.innerText()).trim();
    await expect(dialog.locator('section.content h3').first()).toHaveText(title);
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
