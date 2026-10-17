import { ConsoleMessage, expect, Page, test } from '@playwright/test';
import { chooseMenu } from './menu-support';

// Error capture: every failure is logged once, with the operation that caused it. The browser build logs to the
// console (the mock log sink); `?imtFault=view:*` (core/activity/fault-injection.ts) makes every view throw.

interface LogLine {
  category: string;
  message: string;
  props: Record<string, string>;
}

function collect(page: Page): LogLine[] {
  const lines: LogLine[] = [];
  page.on('console', (msg: ConsoleMessage) => {
    const m = /^\[([\w.]+)\] /.exec(msg.text());
    if (!m) return;
    const [first, second] = msg.args();
    // The page may close while the arguments are read (end of test); such late messages are dropped.
    void Promise.all([first?.jsonValue(), second?.jsonValue()])
      .then(([message, props]) =>
        lines.push({ category: m[1], message: String(message).slice(m[0].length), props: (props ?? {}) as Record<string, string> }),
      )
      .catch(() => undefined);
  });
  return lines;
}

test('a view that throws is contained and logged with the operation that opened it, for every registered view', async ({ page }) => {
  test.setTimeout(120_000);
  const lines = collect(page);
  await page.goto('/?imtFault=view:*');
  await expect(page.locator('.imt-panel-failed').first()).toBeVisible();

  // Every view the app offers, from View ▸ Other windows….
  await chooseMenu(page, 'View', 'Other windows…');
  const picker = page.getByRole('dialog', { name: 'Other windows' });
  const options = picker.getByRole('option');
  await expect(options.first()).toBeVisible();
  const count = await options.count();
  expect(count).toBeGreaterThan(10);
  await page.keyboard.press('Escape');

  const notLogged: string[] = [];
  let correlated = 0;
  for (let i = 0; i < count; i++) {
    await chooseMenu(page, 'View', 'Other windows…');
    const option = picker.getByRole('option').nth(i);
    const title = (await option.locator('.title, .label').first().innerText().catch(() => option.innerText())).trim();
    const before = lines.length;
    await option.click();
    // Its failure is logged (now, or when it first opened with the layout).
    const logged = await expect
      .poll(() => lines.some((l) => l.category === 'ui.panel' && l.message === 'Panel failed' && title.includes(l.props['Panel'] ?? '\u0000')), { timeout: 3000 })
      .toBe(true)
      .then(() => true, () => false);
    if (!logged) notLogged.push(title);
    // A panel failing now is correlated with the operation that opened it (a tab already open in the
    // background renders, and fails, outside any operation).
    for (const fresh of lines.slice(before).filter((l) => l.category === 'ui.panel')) {
      expect(fresh.props['Window']).toBe('main');
      expect(fresh.props['Error']).toContain('Injected fault');
      const op = fresh.props['OperationId'];
      if (op) {
        expect(lines.some((l) => l.category === 'IMT.Activity' && l.props['OperationId'] === op)).toBe(true);
        correlated++;
      }
    }
  }
  expect(notLogged).toEqual([]);
  expect(correlated).toBeGreaterThan(0);
  // The rest of the app still works.
  await expect(page.getByRole('menubar', { name: 'Application menu' })).toBeVisible();
});

test('uncaught errors and unhandled rejections are logged with their type, stack and the current operation', async ({ page }) => {
  const lines = collect(page);
  await page.goto('/');
  await expect(page.getByRole('grid', { name: 'Events' })).toBeVisible();
  await page.evaluate(() => {
    setTimeout(() => {
      throw new TypeError('async boom in /Users/alice/secret/file.log');
    });
    void Promise.reject(new RangeError('rejected promise'));
  });
  await expect.poll(() => lines.filter((l) => l.category === 'ui.error').length).toBeGreaterThanOrEqual(2);
  const errors = lines.filter((l) => l.category === 'ui.error');
  const thrown = errors.find((l) => l.message.includes('async boom'))!;
  expect(thrown.message).toBe('async boom in <path>');
  expect(thrown.props).toMatchObject({ ErrorType: 'TypeError', Window: 'main' });
  expect(thrown.props['Stack']).toBeTruthy();
  const rejected = errors.find((l) => l.message.includes('rejected promise'))!;
  expect(rejected.props['ErrorType']).toBe('RangeError');
  // Errors carry the latest activity as breadcrumbs (Rust attaches them in the desktop app).
  await chooseMenu(page, 'Help', 'Keyboard shortcuts');
  await page.keyboard.press('Escape');
  await expect.poll(() => lines.some((l) => l.category === 'IMT.Activity' && l.props['Trigger'] === 'menu')).toBe(true);
  await page.evaluate(() => void Promise.reject(new Error('after a menu command')));
  await expect.poll(() => lines.some((l) => l.message === 'after a menu command')).toBe(true);
  const last = lines.find((l) => l.message === 'after a menu command')!;
  expect(last.props['View']).not.toMatch(/[×▾📌]/u);
  expect(JSON.parse(last.props['Breadcrumbs']).some((b: { operation: string; trigger: string }) => b.trigger === 'menu')).toBe(true);
});
