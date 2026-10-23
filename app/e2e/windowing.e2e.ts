import { expect, Page, test } from '@playwright/test';
import { nextRender, openApp } from './app';
import { chooseMenu, openMenu } from './menu-support';

// Windows, the close prompt and background work against the browser build's mock windowing backend
// (src/app/windowing/mock-windowing-backend.ts): each window is a tab with `?window=<label>` and its
// own MockBackend. CSP problems are collected so the file also runs against the production build.

function watchProblems(page: Page): string[] {
  const problems: string[] = [];
  page.on('console', (msg) => {
    if (/Content Security Policy|Refused to/i.test(msg.text())) problems.push(msg.text());
  });
  page.on('pageerror', (err) => problems.push(`pageerror: ${err.message}`));
  return problems;
}

/** New window and Close window are in the File menu. */
async function windowMenu(page: Page, item: string): Promise<void> {
  await chooseMenu(page, 'File', item);
}

const statusBar = (page: Page) => page.locator('debrief-status-bar');

test('New window opens a second window with its own workspace', async ({ page, context }) => {
  const problems = watchProblems(page);
  await openApp(page);
  const opened = context.waitForEvent('page');
  await windowMenu(page, 'New window');
  const second = await opened;
  const secondProblems = watchProblems(second);
  await second.waitForLoadState();
  expect(new URL(second.url()).searchParams.get('window')).toMatch(/^w-\d+$/);
  await expect(statusBar(second)).toContainText('events');
  const sources = (await statusBar(page).locator('.strong').innerText()).trim();
  expect(sources).not.toBe('0 sources');

  // Closing the workspace in the new window leaves the first one alone.
  await chooseMenu(second, 'File', 'Close workspace');
  await expect(statusBar(second)).toContainText('No files open');
  await page.bringToFront();
  await expect(statusBar(page).locator('.strong')).toHaveText(sources);
  await expect(page.locator('debrief-events-grid [role=row].row').first()).toBeVisible();

  // The first window lists the new one in its Window menu.
  const windows = await openMenu(page, 'Window');
  await expect(windows.getByRole('group', { name: 'Windows' }).getByRole('menuitemcheckbox')).toHaveCount(2);
  await page.keyboard.press('Escape');
  expect([...problems, ...secondProblems]).toEqual([]);
});

test('Mod+Shift+N opens a new window', async ({ page, context }) => {
  await openApp(page);
  const opened = context.waitForEvent('page');
  await page.locator('body').click({ position: { x: 5, y: 5 } });
  await page.keyboard.press('ControlOrMeta+Shift+N');
  const second = await opened;
  expect(new URL(second.url()).searchParams.get('window')).toMatch(/^w-\d+$/);
});

test('closing the last window asks once, then remembers "Keep running in tray"', async ({ page }) => {
  const problems = watchProblems(page);
  await openApp(page);
  await windowMenu(page, 'Close window');
  const prompt = page.getByRole('dialog', { name: 'Close Debrief?' });
  await expect(prompt).toBeVisible();
  await expect(prompt.getByRole('checkbox', { name: 'Remember my choice' })).toBeChecked();
  await prompt.getByRole('button', { name: 'Keep running in tray' }).click();
  await expect(prompt).toHaveCount(0);

  // The remembered choice: the mock reports the close it handled (hide into the tray), and nothing asks again.
  const handled = page.waitForEvent('console', (m) => m.text() === '[mock windowing] close_window tray');
  await windowMenu(page, 'Close window');
  await handled;
  await nextRender(page);
  await expect(prompt).toHaveCount(0);
  await expect(statusBar(page)).toContainText('events');
  expect(problems).toEqual([]);
});

test('File ▸ Exit quits Debrief in one step (nothing unsaved: no question)', async ({ page }) => {
  const problems = watchProblems(page);
  await openApp(page);
  const mac = await page.evaluate(() => /Mac|iPhone|iPad/.test(navigator.platform));
  const quit = page.waitForEvent('console', (m) => m.text() === '[mock windowing] quit_app');
  await chooseMenu(page, 'File', mac ? 'Quit Debrief' : 'Exit');
  await quit;
  await expect(page.getByRole('dialog')).toHaveCount(0);
  expect(problems).toEqual([]);
});

test('background work lists running tasks and stops one', async ({ page }) => {
  const problems = watchProblems(page);
  await page.goto('/?mock-tasks');
  await expect(statusBar(page)).toContainText('events');
  const button = statusBar(page).getByRole('button', { name: 'Background work' });
  await expect(button).toHaveText('3');
  await button.click();
  const popover = page.getByRole('dialog', { name: 'Background work running' });
  await expect(popover.getByRole('listitem')).toHaveCount(3);
  await expect(popover).toContainText('Live tail — main');
  await expect(popover.getByRole('button', { name: 'Stop Downloading symbols' })).toBeDisabled();
  await popover.getByRole('button', { name: 'Stop Process monitor' }).click();
  await expect(button).toHaveText('2');
  await expect(popover.getByRole('listitem')).toHaveCount(2);
  await page.keyboard.press('Escape');
  await expect(popover).toHaveCount(0);
  expect(problems).toEqual([]);
});

test('no background work button without background work', async ({ page }) => {
  await openApp(page);
  await expect(statusBar(page).getByRole('button', { name: 'Background work' })).toHaveCount(0);
});

test('files from another app: the open-ask dialog opens here, or drops the files when dismissed', async ({ page }) => {
  const problems = watchProblems(page);
  const resolved: string[] = [];
  page.on('console', (msg) => {
    const m = /^\[mock windowing\] resolve_open (\w+)$/.exec(msg.text());
    if (m) resolved.push(m[1]);
  });
  const ask = page.getByRole('dialog', { name: 'Open 2 files' });

  // Escape.
  await page.goto('/?mock-open-ask=a.log,b.log');
  await expect(ask).toBeVisible();
  await expect(ask.getByRole('list', { name: 'Files to open' }).getByRole('listitem')).toHaveText(['a.log', 'b.log']);
  await page.keyboard.press('Escape');
  await expect(ask).toHaveCount(0);
  await expect.poll(() => resolved).toEqual(['cancel']);

  // The header's close button.
  await page.reload();
  await ask.getByRole('button', { name: 'Close dialog' }).click();
  await expect(ask).toHaveCount(0);
  await expect.poll(() => resolved).toEqual(['cancel', 'cancel']);

  // The backdrop.
  await page.reload();
  await expect(ask).toBeVisible();
  await page.mouse.click(5, 5);
  await expect(ask).toHaveCount(0);
  await expect.poll(() => resolved).toEqual(['cancel', 'cancel', 'cancel']);

  // A choice is sent once, and closing afterwards does not cancel it.
  await page.reload();
  await ask.getByRole('button', { name: 'This window' }).click();
  await expect(ask).toHaveCount(0);
  // A cancel on close would be logged as the dialog closes, before it leaves the page: the list is final now.
  await nextRender(page);
  expect(resolved).toEqual(['cancel', 'cancel', 'cancel', 'here']);
  expect(problems).toEqual([]);
});
