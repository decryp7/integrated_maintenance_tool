import { BrowserContext, ConsoleMessage, expect, Page, test } from '@playwright/test';
import { ensureInspector, openApp } from './app';
import { chooseMenu } from './menu-support';

// The activity log in the browser build: entries go to the console logger (the mock "log sink"), as
// `[IMT.Activity] User <operation> via <trigger>` with the entry's properties as the second argument.

interface Entry {
  message: string;
  props: Record<string, string>;
}

/** Collects the activity entries a page logs. */
function collect(page: Page): Entry[] {
  const entries: Entry[] = [];
  page.on('console', (msg: ConsoleMessage) => {
    if (!msg.text().startsWith('[IMT.Activity]')) return;
    const [first, second] = msg.args();
    // The page may close while the arguments are read (end of test); such late messages are dropped.
    void Promise.all([first?.jsonValue(), second?.jsonValue()])
      .then(([message, props]) =>
        entries.push({ message: String(message).replace('[IMT.Activity] ', ''), props: (props ?? {}) as Record<string, string> }),
      )
      .catch(() => undefined);
  });
  return entries;
}

const tab = (page: Page, name: string) => page.getByRole('tab', { name, exact: true });
const groupOf = (page: Page, name: string) => page.locator('.dv-groupview').filter({ has: tab(page, name) });

async function popOut(page: Page, context: BrowserContext, name: string): Promise<Page> {
  const opened = context.waitForEvent('page');
  await tab(page, name).click();
  await groupOf(page, name).getByRole('button', { name: 'Panel menu', exact: true }).click();
  await page.getByRole('menu', { name: 'Actions' }).getByRole('menuitem', { name: 'Pop out to window', exact: true }).click();
  const popup = await opened;
  await expect(popup.locator('imt-popout-shell')).toBeVisible();
  return popup;
}

/** The index of the first entry at or after `from` matching `want`, or -1. */
function find(entries: Entry[], from: number, want: Partial<Record<string, string | RegExp>>): number {
  for (let i = from; i < entries.length; i++) {
    const p = entries[i].props;
    if (Object.entries(want).every(([k, v]) => (v instanceof RegExp ? v.test(p[k] ?? '') : p[k] === v))) return i;
  }
  return -1;
}

test('a user journey is logged once per operation, in order, with triggers and without sensitive text', async ({ page, context }) => {
  const entries = collect(page);
  await openApp(page);

  // Open the demo from the welcome page.
  await chooseMenu(page, 'File', 'Close workspace');
  await page.locator('imt-welcome').getByRole('button', { name: 'Try a demo' }).click();
  await expect(page.locator('imt-status-bar')).toContainText('events');
  // The demo offers the guided tour (it may appear a moment after the data): skip it.
  const tour = page.getByTestId('tour');
  await expect(tour).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(tour).toHaveCount(0);

  // Filter with words that must never reach the log.
  await page.getByTestId('query-field').fill('needle-secret-words');
  await expect.poll(() => (find(entries, 0, { Operation: 'search.change' }) >= 0 ? 'ok' : entries.map((e) => e.message).join(' | '))).toBe('ok');
  await page.getByTestId('query-field').fill('');
  await expect.poll(() => entries.filter((e) => e.props['Operation'] === 'search.change').length).toBe(2);

  // Select an event.
  const row = page.locator('imt-events-grid [role=row].row').nth(2);
  const rowText = (await row.locator('.msg').innerText()).trim();
  await row.click();
  await expect.poll(() => find(entries, 0, { Operation: 'selection.change' })).toBeGreaterThanOrEqual(0);

  // Pop out the inspector (docked first: by default it is auto-hidden).
  await ensureInspector(page);
  await popOut(page, context, 'Inspector');

  // Change a setting.
  await page.keyboard.press('ControlOrMeta+Comma');
  const settings = page.getByRole('dialog', { name: 'Settings' });
  await settings.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'Diagnostics', exact: true }).click();
  await settings.getByLabel('Log level', { exact: true }).selectOption('debug');
  // Settings apply by themselves (there is no Apply button).
  await expect(settings.getByTestId('settings-notice')).toContainText('saved');
  await page.keyboard.press('Escape');

  // Export.
  await chooseMenu(page, 'File', 'Export events…');
  const dialog = page.getByRole('dialog', { name: 'Export events' });
  await dialog.getByRole('button', { name: 'Export…' }).click();
  // A job writes the file; its message reports the result.
  await expect(page.getByTestId('notice-toast').filter({ hasText: 'Finished: Export events' })).toBeVisible({ timeout: 15_000 });

  // In order, each with its trigger.
  const steps: Partial<Record<string, string | RegExp>>[] = [
    { Title: 'Close workspace', Trigger: 'menu', Outcome: 'ok' },
    { Operation: 'file.open', Trigger: 'button' },
    { Operation: 'search.change', Trigger: 'keyboard' },
    { Operation: 'selection.change', Trigger: 'pointer' },
    { Operation: 'dock.popout', Panel: 'inspector', Trigger: 'context-menu' },
    { Operation: 'settings.change', Section: 'diagnostics', Fields: /logLevel/, Trigger: 'dialog' },
    // The command runs until its dialog is done, so the export it makes is part of it (one entry).
    { Operation: 'file.export', Title: /^Export events/, Trigger: 'menu', Outcome: 'ok' },
  ];
  await expect
    .poll(() => {
      let at = 0;
      for (const want of steps) {
        const i = find(entries, at, want);
        if (i < 0) return `missing ${JSON.stringify(want)} after #${at} in ${entries.map((e) => e.message).join(' | ')}`;
        at = i + 1;
      }
      return 'ok';
    })
    .toBe('ok');

  // Every entry is complete: an operation id, the window, a duration and an outcome.
  for (const e of entries) {
    expect(e.message).toBe(`User ${e.props['Operation']} via ${e.props['Trigger']}`);
    expect(e.props).toMatchObject({ SourceContext: 'IMT.Activity', MessageTemplate: 'User {Operation} via {Trigger}', Window: 'main' });
    expect(e.props['OperationId']).toMatch(/^[0-9a-f-]{36}$/);
    expect(e.props['Outcome']).toMatch(/^(ok|cancelled|error)$/);
  }
  // Nothing sensitive: no search text, no record message.
  const all = JSON.stringify(entries);
  expect(all).not.toContain('needle-secret-words');
  expect(all).not.toContain(rowText.slice(0, 30));
  const search = entries.find((e) => e.props['Operation'] === 'search.change')!;
  expect(search.props['SearchHash']).toMatch(/^[0-9a-f]{8}$/);
  expect(search.props['SearchLength']).toBe('19');
});
