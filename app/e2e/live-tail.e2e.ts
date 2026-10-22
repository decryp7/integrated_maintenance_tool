import { expect, Page, test } from '@playwright/test';
import { gridTotal, parseCount } from './app';
import { expectNoSeriousViolations } from './axe';
import { openAppWith } from './mock-url';
import { chooseMenu } from './menu-support';

/** Realistic live simulation: 40 events/s, no rotations or alerts unless a test asks for them. */
const LIVE = '?mockLive=1&mockLiveRate=40&mockLiveRotateMs=0&mockLiveAlertMs=0';

const liveButton = (page: Page) => page.locator('imt-status-bar').getByRole('button', { name: 'Live tail', exact: true });
const timelineTo = async (page: Page) => Number(await page.locator('imt-timeline-panel .wrap').getAttribute('data-to'));
const timelineFrom = async (page: Page) => Number(await page.locator('imt-timeline-panel .wrap').getAttribute('data-from'));

async function liveOn(page: Page): Promise<void> {
  await liveButton(page).click();
  await expect(liveButton(page)).toHaveAttribute('aria-pressed', 'true');
}

/** The `setLive` calls the mock backend received (published by `?mockLive=1`). */
function liveCalls(page: Page) {
  return page.evaluate(() => (globalThis as unknown as { __imtMockLiveCalls?: { enabled: boolean; options: Record<string, unknown> | null }[] }).__imtMockLiveCalls ?? []);
}

/** Live alerts on Playwright's clock: one live tick every 500 ms (few ticks to run), the alert interval added per test. */
const ALERTS = '?mockLive=1&mockLiveRate=20&mockLiveMs=500&mockLiveRotateMs=0';

async function openView(page: Page, name: 'Alerts' | 'Tail status'): Promise<void> {
  await chooseMenu(page, 'View', 'Live', name);
}

/**
 * Opens the app on Playwright's clock: the page's timers (and so every mock timer, see core/mock/mock-clock.ts) still
 * run in real time, and `page.clock.runFor` moves them on at once, so alerts due every second arrive on cue however
 * loaded the runner is.
 */
async function openOnClock(page: Page, search: string): Promise<void> {
  await page.clock.install();
  await openAppWith(page, search);
}

test.describe('live tail: follow now', () => {
  test('live on: new events appear, and the timeline slides with the newest event in a live window', async ({ page }) => {
    await openAppWith(page, LIVE);
    await expect(gridTotal(page)).toHaveText(/^\d[\d,]*$/);
    const start = parseCount(await gridTotal(page).innerText());
    await liveOn(page);
    const follow = page.getByTestId('follow-toggle');
    await expect(follow).toHaveAttribute('aria-pressed', 'true');
    await expect.poll(async () => parseCount(await gridTotal(page).innerText()), { timeout: 10_000 }).toBeGreaterThan(start);

    await page.getByRole('combobox', { name: 'Live window' }).selectOption('1m');
    // The engine (mock) resolves the window: the timeline spans one minute up to the newest event...
    await expect.poll(async () => (await timelineTo(page)) - (await timelineFrom(page)), { timeout: 10_000 }).toBeLessThanOrEqual(60_000);
    const to1 = await timelineTo(page);
    // ...and slides as new events arrive.
    await expect.poll(() => timelineTo(page), { timeout: 10_000 }).toBeGreaterThan(to1);
    await expect.poll(() => timelineFrom(page), { timeout: 10_000 }).toBeGreaterThan(to1 - 60_000);
    // The grid follows newest-first at the top.
    await page.getByRole('button', { name: /Oldest first/ }).click();
    const first = page.locator('imt-events-grid [role=row].row').first();
    await expect(first).toContainText(/Live event|Process stats/);
    const t1 = await first.textContent();
    await expect.poll(() => first.textContent(), { timeout: 10_000 }).not.toBe(t1);
  });

  test('a manual zoom pauses following; Resume follows again; not following shows "N new events"', async ({ page }, testInfo) => {
    await openAppWith(page, LIVE);
    await liveOn(page);
    await page.getByRole('combobox', { name: 'Live window' }).selectOption('5m');
    await expect(page.getByTestId('follow-toggle')).toBeVisible();
    await page.locator('imt-timeline-panel').getByRole('button', { name: 'Zoom in' }).click();
    const resume = page.getByTestId('resume-following');
    await expect(resume).toHaveText(/Paused — Resume following/);
    await expectNoSeriousViolations(page, testInfo, 'live-paused', 'imt-timeline-panel .head');
    // While paused the grid holds its rows and counts what arrives.
    await expect(page.getByTestId('new-events')).toBeVisible({ timeout: 10_000 });
    await expect(page.getByTestId('new-events')).toHaveText(/\d[\d,]* new events? — show/);
    const toBefore = await timelineTo(page);
    await resume.click();
    await expect(page.getByTestId('follow-toggle')).toHaveAttribute('aria-pressed', 'true');
    await expect(page.getByTestId('new-events')).toHaveCount(0);
    await expect.poll(() => timelineTo(page), { timeout: 10_000 }).toBeGreaterThan(toBefore);
    // Follow off: the pill comes back and shows the held events when clicked.
    await page.getByTestId('follow-toggle').click();
    await expect(page.getByTestId('follow-toggle')).toHaveAttribute('aria-pressed', 'false');
    const pill = page.getByTestId('new-events');
    await expect(pill).toBeVisible({ timeout: 10_000 });
    // Clicking shows the held events: the count starts again from zero. Follow is still off, so events arriving
    // after the click are counted again (the pill may come back at once at 40 events/s); wait for a count high
    // enough that "started again" is unambiguous.
    const held = async () => ((await pill.count()) ? parseCount(await pill.innerText()) : 0);
    await expect.poll(held, { timeout: 10_000 }).toBeGreaterThan(60);
    const before = await held();
    await pill.click();
    await expect.poll(held, { timeout: 5_000 }).toBeLessThan(before);
  });
});

test.describe('live tail: alerts', () => {
  test('an alert toast says its severity in words; Show applies the evidence query', async ({ page }, testInfo) => {
    await openOnClock(page, `${ALERTS}&mockLiveAlertMs=1500`);
    await liveOn(page);
    // The first alert (an error spike) is due 1.5 s of live ticks after live tail starts.
    await page.clock.runFor(2_000);
    // The toast stack shows the newest three: take a visible error spike (an older toast may be folded away meanwhile).
    const toast = page.getByTestId('alert-toast').filter({ hasText: 'Error spike', visible: true }).last();
    await expect(toast).toBeVisible();
    await expect(toast).toContainText('Critical');
    await expect(toast).toContainText('Error spike');
    await expectNoSeriousViolations(page, testInfo, 'alert-toast', 'imt-alert-toasts');
    await toast.getByRole('button', { name: 'Show' }).click();
    // The evidence: errors and fatals in the alert's time range (which pauses following).
    const level = (l: string) => page.getByRole('toolbar', { name: 'Levels' }).locator(`button[data-level=${l}]`);
    await expect(level('Error')).toHaveAttribute('aria-pressed', 'true');
    await expect(level('Fatal')).toHaveAttribute('aria-pressed', 'true');
    await expect(level('Information')).toHaveAttribute('aria-pressed', 'false');
    await expect(page.getByTestId('resume-following')).toBeVisible();
    await expect(page.locator('imt-events-grid [role=row].row').first()).toBeVisible();
  });

  test('mute this kind for 15 min stops its toasts', async ({ page }) => {
    // Not waiting on timers (the clock moves them on) but rendering every simulated frame of ~9 s of live tail.
    test.slow();
    await openOnClock(page, `${ALERTS}&mockLiveAlertMs=1000`);
    // The alerts panel lists every alert received, so the test can tell when the next error spike was due.
    await openView(page, 'Alerts');
    await liveOn(page);
    // One alert a second, in turn: error spike, rule, silence.
    await page.clock.runFor(1_500);
    const toast = page.getByTestId('alert-toast').filter({ hasText: 'Error spike' }).first();
    await expect(toast).toBeVisible();
    await toast.getByRole('button', { name: 'Mute this kind for 15 min' }).click();
    await expect(page.getByTestId('alert-toast').filter({ hasText: 'Error spike' })).toHaveCount(0);
    // The next alerts of other kinds still toast.
    await page.clock.runFor(1_000);
    await expect(page.getByTestId('alert-toast').filter({ hasText: /Rule: payment errors|Silence/ }).first()).toBeVisible();
    // Two more rule alerts mean an error spike was due in between, and it was not toasted (the engine holds muted
    // kinds back).
    const ruleRows = page.locator('imt-alerts-view').getByTestId('alert-row').filter({ hasText: 'Rule: payment errors' });
    const rules = await ruleRows.count();
    await page.clock.runFor(6_000);
    await expect.poll(() => ruleRows.count()).toBeGreaterThanOrEqual(rules + 2);
    await expect(page.getByTestId('alert-toast').filter({ hasText: 'Error spike' })).toHaveCount(0);
  });

  test('the alerts panel lists, filters, clears, counts on its tab, and passes axe', async ({ page }, testInfo) => {
    // Two axe scans and every simulated frame of live tail rendered: CPU work, not waiting on timers.
    test.slow();
    await openOnClock(page, `${ALERTS}&mockLiveAlertMs=500`);
    await openView(page, 'Alerts');
    const panel = page.locator('imt-alerts-view');
    await expect(panel).toContainText('No alerts yet');
    await liveOn(page);
    // Three alerts (one of each kind) are due within 1.5 s of live ticks.
    await page.clock.runFor(2_000);
    await expect.poll(() => panel.getByTestId('alert-row').count()).toBeGreaterThanOrEqual(3);
    await expect(page.locator('.imt-tab-label', { hasText: /^Alerts \(\d+\)$/ })).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'alerts-panel', 'imt-alerts-view');
    await liveButton(page).click();
    await expect(liveButton(page)).toHaveAttribute('aria-pressed', 'false');
    // The toasts float over the panel (critical ones stay until closed): close them so they cannot cover its buttons.
    const dismiss = page.getByRole('button', { name: 'Dismiss alert' });
    // (A warning toast may time out on its own between the count and the click.)
    while (await dismiss.count()) await dismiss.first().click({ timeout: 2_000 }).catch(() => undefined);
    const filters = panel.getByRole('group', { name: 'Filter by severity' });
    await filters.getByRole('button', { name: /Critical/ }).click();
    await expect(filters.getByRole('button', { name: /Critical/ })).toHaveAttribute('aria-pressed', 'true');
    const rows = panel.getByTestId('alert-row');
    // Web-first: the filtered list has rows, and none of another severity (a count read at once may predate the filter).
    await expect(rows.first()).toContainText('Critical');
    await expect(rows.filter({ hasNotText: 'Critical' })).toHaveCount(0);
    await filters.getByRole('button', { name: /^All/ }).click();
    await panel.getByRole('button', { name: 'Show evidence: Error spike' }).first().click();
    await expect(page.getByRole('toolbar', { name: 'Levels' }).locator('button[data-level=Error]')).toHaveAttribute('aria-pressed', 'true');
    // "Show" brings the events grid to the front; go back to the alerts tab.
    await page.locator('.imt-tab-label', { hasText: /^Alerts/ }).click();
    await panel.getByRole('button', { name: 'Clear' }).click();
    await expect(panel).toContainText('No alerts yet');
    await expect(page.locator('.imt-tab-label', { hasText: /^Alerts$/ })).toBeVisible();
  });

  test('the alert rules editor shows the engine error next to the rule and saves a valid rule', async ({ page }, testInfo) => {
    await openAppWith(page, LIVE);
    await openView(page, 'Alerts');
    await page.locator('imt-alerts-view').getByRole('button', { name: 'Alert rules…' }).click();
    const dialog = page.getByRole('dialog', { name: 'Live alert rules' });
    await expect(dialog).toBeVisible();
    await dialog.getByRole('button', { name: '+ Add rule' }).click();
    const rule = dialog.locator('fieldset.rule');
    await rule.getByLabel('Window').fill('soon');
    await rule.getByLabel('Window').press('Tab');
    await dialog.getByRole('button', { name: 'Validate' }).click();
    await expect(rule.getByRole('alert')).toContainText('live.rule "rule 1".window');
    await expectNoSeriousViolations(page, testInfo, 'rules-editor', '[role=dialog]');
    await rule.getByLabel('Window').fill('2m');
    await rule.getByLabel('Window').press('Tab');
    const condition = rule.getByRole('combobox', { name: 'Condition of rule 1' });
    await condition.fill('level = "Error"');
    await dialog.getByRole('button', { name: 'Validate' }).click();
    await expect(dialog).toContainText('The rules are valid');
    await dialog.getByRole('button', { name: 'Save' }).click();
    await expect(dialog).toHaveCount(0);
  });
});

test.describe('live tail: status, sources and opening', () => {
  test('the tail status panel shows each file and links to Settings ▸ Live tail, whose controls call set_live', async ({ page }, testInfo) => {
    await openAppWith(page, LIVE);
    await openView(page, 'Tail status');
    const panel = page.locator('imt-tail-status-view');
    await expect(panel.getByTestId('tail-file').first()).toBeVisible();
    await panel.getByRole('button', { name: 'Live tail off' }).click();
    await expect(panel.getByRole('button', { name: 'Live tail on' })).toHaveAttribute('aria-pressed', 'true');
    await expect(panel).toContainText('lines/s');
    await expectNoSeriousViolations(page, testInfo, 'tail-status', 'imt-tail-status-view');

    // The options live in Settings ▸ Live tail (I2); Settings… opens them.
    await panel.getByTestId('tail-settings').click();
    const dialog = page.getByRole('dialog', { name: 'Settings' });
    const form = dialog.getByRole('form', { name: 'Live tail options' });
    await expect(form).toBeVisible();
    await form.getByLabel('Watch').selectOption('polling');
    await expect.poll(async () => (await liveCalls(page)).at(-1)?.options?.['watch']).toBe('polling');
    await form.getByLabel('Poll every').fill('500');
    await form.getByLabel('Poll every').press('Tab');
    await expect.poll(async () => (await liveCalls(page)).at(-1)?.options?.['pollIntervalMs']).toBe(500);
    await form.getByLabel('Keep at most').first().fill('100000');
    await form.getByLabel('Keep at most').first().press('Tab');
    await expect.poll(async () => ((await liveCalls(page)).at(-1)?.options?.['retention'] as { maxRecords?: number })?.maxRecords).toBe(100000);
    await form.getByLabel('Deleted files').selectOption('drop');
    await expect.poll(async () => (await liveCalls(page)).at(-1)?.options?.['deletedFiles']).toBe('drop');
    await form.getByLabel('Flush a partial line after').fill('750');
    await form.getByLabel('Flush a partial line after').press('Tab');
    await expect.poll(async () => (await liveCalls(page)).at(-1)?.options?.['carryFlushMs']).toBe(750);
    // Detector interval: applied, and clamped to the engine's range (0 .. 600000 ms).
    const detect = form.getByTestId('detect-every');
    await expect(detect).toHaveValue('1000');
    await detect.fill('250');
    await detect.press('Tab');
    await expect.poll(async () => (await liveCalls(page)).at(-1)?.options?.['detectEveryMs']).toBe(250);
    await detect.fill('99999999');
    await detect.press('Tab');
    await expect.poll(async () => (await liveCalls(page)).at(-1)?.options?.['detectEveryMs']).toBe(600000);
    await expect(detect).toHaveValue('600000');
    await page.keyboard.press('Escape');
    await expect(dialog).toHaveCount(0);
    await expect(panel.getByTestId('tail-file').filter({ hasText: 'Polling' }).first()).toBeVisible({ timeout: 10_000 });
  });

  test('rolling files of one log collapse into a group that expands, with a rotated badge', async ({ page }, testInfo) => {
    await openOnClock(page, '?mockLive=1&mockLiveRate=20&mockLiveRotateMs=1000&mockLiveAlertMs=0');
    await liveOn(page);
    // Two rotations are due within 2 s of live ticks.
    await page.clock.runFor(2_500);
    const group = page.getByTestId('source-group').first();
    await expect(group).toBeVisible();
    await expect(group).toHaveAttribute('aria-expanded', 'false');
    await expect(group).toContainText(/\d+ files/);
    await group.click();
    await expect(group).toHaveAttribute('aria-expanded', 'true');
    const members = page.locator('imt-sidebar .src.member');
    await expect.poll(() => members.count()).toBeGreaterThanOrEqual(2);
    await expect(page.locator('imt-sidebar .state', { hasText: 'Rotated' }).first()).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'source-groups', 'imt-sidebar');
    await group.click();
    await expect(members).toHaveCount(0);
  });

  test('"Open for live tail" starts at the end: the new file has no history and only gets new lines', async ({ page }, testInfo) => {
    await openAppWith(page, LIVE);
    await page.getByRole('button', { name: 'Open for live tail' }).click();
    const dialog = page.getByRole('dialog', { name: 'Open for live tail' });
    const atEnd = dialog.getByRole('checkbox', { name: 'Live: start at end (only new lines)' });
    await expect(atEnd).toBeChecked();
    await expectNoSeriousViolations(page, testInfo, 'open-live', '[role=dialog]');
    await dialog.getByRole('button', { name: 'Open files…' }).click();
    await expect(dialog).toHaveCount(0);
    await expect(liveButton(page)).toHaveAttribute('aria-pressed', 'true');
    const row = page.locator('imt-sidebar .src', { hasText: 'extra-service.log' });
    await expect(row).toBeVisible();
    // Its history was skipped: none of the file's existing lines ("Sent N notifications") were imported...
    await page.getByTestId('query-field').fill('notifications');
    await expect(gridTotal(page)).toHaveText('0');
    // ...but it grows with the new lines.
    await expect.poll(async () => parseCount((await row.locator('.count').innerText()) || '0'), { timeout: 10_000 }).toBeGreaterThan(0);
    await expect(gridTotal(page)).toHaveText('0');
  });
});
