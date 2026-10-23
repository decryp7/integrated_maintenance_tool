import { expect, Page, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { ask, openChat, runCommand, waitAnswered } from './ai-support';
import { openAppWith } from './mock-url';

/** The app with the AI mock at `search` (its own step time: `openAiApp` fixes a fast one). */
async function openAiApp(page: Page, search: string): Promise<void> {
  await openAppWith(page, `?${search.includes('mockAiStepMs') ? '' : 'mockAiStepMs=5&'}${search}`);
  await expect(page.locator('debrief-events-grid [role=row].row').first()).toBeVisible();
}

// Jobs, notifications and research against the mock backends (src/app/jobs/mock-jobs.ts,
// src/app/notifications/mock-notifications.ts and the AI mock). The thresholds of the engine (jobs show after
// 2 s, a toast after 10 s) are shortened with `mockJobsVisibleMs` / `mockJobsAttentionMs` so the tests stay fast;
// the rules themselves are tested in Rust (app/src-tauri/src/jobs).

const jobsPane = (page: Page) => page.locator('debrief-jobs-panel');
const noticesPane = (page: Page) => page.locator('debrief-notifications-panel');

// Jobs and Notifications are status tools: they slide out of the bottom edge and slide away again when focus moves
// elsewhere (a dialog, Escape), so each assertion on them opens them first (and does nothing when they are open).
async function openJobs(page: Page) {
  if (!(await jobsPane(page).isVisible())) await runCommand(page, 'View: Jobs');
  await expect(jobsPane(page)).toBeVisible();
  return jobsPane(page);
}

async function openNotifications(page: Page) {
  if (!(await noticesPane(page).isVisible())) await page.getByTestId('status-notifications').click();
  await expect(noticesPane(page)).toBeVisible();
  return noticesPane(page);
}

/** Explains the first event: no dialog, the request runs as a job and a toast says so. */
async function explainInBackground(page: Page): Promise<void> {
  await page.locator('debrief-events-grid [role=row].row').first().click();
  await runCommand(page, 'AI: Explain the selected event');
  const started = page.getByTestId('notice-toast').filter({ hasText: /Explaining event #\d+ — running in Jobs/ });
  await expect(started).toBeVisible();
  await expect(started.getByTestId('notice-toast-primary')).toHaveText('Show in Jobs');
  await expect(page.getByRole('dialog')).toHaveCount(0);
}

/** Exports a new incident report named `title`; the dialog closes at once while a slow export job writes it. */
async function exportReport(page: Page, title: string): Promise<void> {
  await runCommand(page, 'New incident report');
  const view = page.locator('debrief-report-view');
  await view.getByRole('textbox', { name: 'Report title' }).fill(title);
  await view.getByRole('textbox', { name: 'Report title' }).press('Tab');
  await runCommand(page, 'Export report');
  const dialog = page.getByRole('dialog', { name: 'Export report' });
  await dialog.getByRole('button', { name: 'Export…' }).click();
  await expect(dialog).toHaveCount(0);
}

test.describe('jobs', () => {
  test.slow();
  test('a slow AI answer and a slow export run side by side; the toast View opens the answer', async ({ page }, testInfo) => {
    await openAiApp(page, 'mockAiStepMs=600&mockExportMs=8000&mockJobsVisibleMs=200&mockJobsAttentionMs=1000');
    await explainInBackground(page);
    await exportReport(page, 'Case 7');
    await openJobs(page);
    const pane = jobsPane(page);
    await expect(pane.locator('li').filter({ hasText: 'Explain event' })).toBeVisible();
    await expect(pane.locator('li').filter({ hasText: 'Export report case-7.html' })).toContainText('Running');
    await expect(page.getByTestId('status-jobs')).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'jobs-pane', 'debrief-jobs-panel');

    // The AI answer finishes first: "Explanation ready" with View, and View opens the answer with its citations.
    const toast = page.getByTestId('notice-toast').filter({ hasText: 'Explanation ready' });
    await expect(toast).toBeVisible({ timeout: 15_000 });
    await expectNoSeriousViolations(page, testInfo, 'jobs-toast', 'debrief-notification-toasts');
    await toast.getByTestId('notice-toast-primary').click();
    await expect(page.getByTestId('ai-narrative')).toContainText('failed order submission');
    await page.keyboard.press('Escape');
    await expect((await openJobs(page)).locator('li').filter({ hasText: 'Explain event' })).toContainText('Done');
  });

  test('a waiting AI request can be cancelled and a failed export retried', async ({ page }) => {
    await openAiApp(page, 'mockAiStepMs=2500&mockJobsVisibleMs=0&mockJobsFailOnce=export');
    await explainInBackground(page);
    await runCommand(page, 'AI: Find likely root causes');
    await expect(page.getByTestId('notice-toast').filter({ hasText: 'Looking for root causes — running in Jobs' })).toBeVisible();
    await openJobs(page);
    const pane = jobsPane(page);
    const waiting = pane.locator('li').filter({ hasText: 'Find likely root causes' });
    await expect(waiting).toContainText('Queued (position 1)');
    await waiting.getByRole('button', { name: /^Cancel / }).click();
    await expect(waiting).toContainText('Cancelled');

    await exportReport(page, 'Case 9');
    const failed = (await openJobs(page)).locator('li').filter({ hasText: 'Export report case-9.html' });
    await expect(failed).toContainText('Failed');
    await expect(failed).toContainText('The disk is full');
    await failed.getByRole('button', { name: /^Retry / }).click();
    await expect((await openJobs(page)).locator('li').filter({ hasText: 'Export report case-9.html' }).filter({ hasText: 'Done' })).toBeVisible();
  });
});

test.describe('notifications', () => {
  test('a finished job is an unread message; View reads it; live alerts merge; Mark all read', async ({ page }, testInfo) => {
    // On Playwright's clock: the mock's live alerts are moved on below instead of waited for in real time (rendering
    // every simulated frame still takes a while on a loaded runner, hence slow).
    test.slow();
    await page.clock.install();
    await openAiApp(page, 'mockAiStepMs=200&mockJobsVisibleMs=0&mockJobsAttentionMs=0&mockLive=1&mockLiveRate=20&mockLiveRotateMs=0&mockLiveAlertMs=300');
    const bell = page.getByTestId('status-notifications');
    await explainInBackground(page);
    await expect(bell).toContainText('1', { timeout: 10_000 });
    const pane = await openNotifications(page);
    const finished = pane.locator('li').filter({ hasText: 'Explanation ready' });
    await expect(finished).toHaveClass(/unread/);
    await expectNoSeriousViolations(page, testInfo, 'notifications-pane', 'debrief-notifications-panel');
    await finished.getByRole('button', { name: 'View', exact: true }).click();
    await expect(page.getByTestId('ai-narrative')).toBeVisible();
    await page.keyboard.press('Escape');
    await openNotifications(page);
    await expect(finished).not.toHaveClass(/unread/);

    // Live alerts: the same kind on the same instance merges into one message with a count.
    await page.locator('debrief-status-bar').getByRole('button', { name: 'Live tail', exact: true }).click();
    // An alert every 300 ms of live ticks, the kinds in turn: within 3 s each kind came more than once.
    await page.clock.runFor(3_000);
    await openNotifications(page);
    await expect(pane.locator('li').filter({ hasText: /\(×\d+\)/ }).first()).toBeVisible();
    await page.locator('debrief-status-bar').getByRole('button', { name: 'Live tail', exact: true }).click();
    // The slid-out list is still closing from the click in the status bar: open it again until it stays.
    await expect(async () => {
      await openNotifications(page);
      await pane.getByTestId('notices-mark-all').dispatchEvent('click', undefined, { timeout: 2000 });
    }).toPass({ timeout: 20_000 });
    await expect(pane.locator('li.unread')).toHaveCount(0);
    await expect(bell).not.toContainText(/\d/);
    await openNotifications(page);
    await expectNoSeriousViolations(page, testInfo, 'notifications-read', 'debrief-notifications-panel');
  });
});

test.describe('research', () => {
  test('a question that needs the web is researched by itself and answered with citations', async ({ page }, testInfo) => {
    await openAiApp(page, 'mockJobsVisibleMs=0');
    const chat = await openChat(page);
    // One Research entry; with automatic research on, its tooltip says the AI also researches by itself.
    await expect(chat.getByTestId('chat-research')).toHaveAttribute('title', /by itself/);
    await ask(chat, 'what does HRESULT 0x80070005 mean for IIS?');
    await waitAnswered(chat);
    const answer = chat.getByTestId('ai-answer-0');
    await expect(answer).toContainText('E_ACCESSDENIED');
    await expect(answer).toContainText('Likely causes and fixes');
    await expect(answer.locator('.sources')).toContainText('Microsoft Learn');
    await expect(answer).toContainText('skipped: a login');
    await expect(chat.getByTestId('ai-browser-menu')).toContainText('Browser: Off');
    // The research ran as a job: "Research finished" with View.
    await expect(page.getByTestId('notice-toast').filter({ hasText: 'Research finished' })).toBeVisible();
    await expect((await openJobs(page)).locator('li').filter({ hasText: 'Research: what does HRESULT 0x80070005' })).toContainText('Done');
    await expectNoSeriousViolations(page, testInfo, 'research-answer', 'debrief-ai-chat-panel');
  });

  test('Research… drafts the search text, confirms workspace data in it, and the answer opens in the chat', async ({ page }, testInfo) => {
    await openAiApp(page, 'mockJobsVisibleMs=0');
    const chat = await openChat(page);
    await chat.getByTestId('chat-research').click();
    const dialog = page.getByRole('dialog', { name: 'Research the web' });
    await dialog.getByLabel('What should the AI find out?').fill('why does WEB02 deny access?');
    await expect(dialog.getByRole('button', { name: 'Start research' })).toBeDisabled();
    await dialog.getByRole('button', { name: 'Suggest search text' }).click();
    await expect(dialog.getByTestId('research-flagged')).toContainText('WEB02');
    // Nothing was searched: the AI wrote the search text and the dialog shows it, with who wrote it.
    await expect(dialog.getByLabel('Search text', { exact: true })).toHaveValue('WEB02 deny access');
    await expect(dialog.getByTestId('research-source')).toContainText('The AI wrote it');
    await expectNoSeriousViolations(page, testInfo, 'research-dialog', '[role=dialog]');
    await dialog.getByLabel('Search text', { exact: true }).fill('IIS access denied 0x80070005');
    await dialog.getByLabel('Results to read').selectOption('10');
    await dialog.getByTestId('research-run').click();
    await expect(dialog).toHaveCount(0);
    const toast = page.getByTestId('notice-toast').filter({ hasText: 'Research finished' });
    await expect(toast).toBeVisible({ timeout: 15_000 });
    await toast.getByTestId('notice-toast-primary').click();
    await expect(chat).toContainText('E_ACCESSDENIED');
  });

  test('with automatic research off the answer offers Research this and agent mode', async ({ page }) => {
    await openAiApp(page, 'mockResearch=off');
    const chat = await openChat(page);
    await ask(chat, 'look up HRESULT 0x80070005');
    await waitAnswered(chat);
    const offer = chat.getByTestId('research-offer');
    await expect(offer).toBeVisible();
    await offer.getByTestId('research-offer-agent').click();
    await expect(chat.getByTestId('ai-browser-menu')).toContainText('Browser: Agent');
  });
});
