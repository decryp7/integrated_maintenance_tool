import { expect, Page, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { ask, openChat, runCommand, waitAnswered } from './ai-support';
import { openAppWith } from './mock-url';

/** The app with the AI mock at `search` (its own step time: `openAiApp` fixes a fast one). */
async function openAiApp(page: Page, search: string): Promise<void> {
  await openAppWith(page, `?${search.includes('mockAiStepMs') ? '' : 'mockAiStepMs=5&'}${search}`);
  await expect(page.locator('imt-events-grid [role=row].row').first()).toBeVisible();
}

// Jobs, notifications and research against the mock backends (src/app/jobs/mock-jobs.ts,
// src/app/notifications/mock-notifications.ts and the AI mock). The thresholds of the engine (jobs show after
// 2 s, a toast after 10 s) are shortened with `mockJobsVisibleMs` / `mockJobsAttentionMs` so the tests stay fast;
// the rules themselves are tested in Rust (app/src-tauri/src/jobs).

const jobsPane = (page: Page) => page.locator('imt-jobs-panel');
const noticesPane = (page: Page) => page.locator('imt-notifications-panel');

async function openJobs(page: Page): Promise<void> {
  await page.locator('[data-activity-id="jobs"]').click();
  await expect(jobsPane(page)).toBeVisible();
}

async function openNotifications(page: Page): Promise<void> {
  await page.locator('[data-activity-id="notifications"]').click();
  await expect(noticesPane(page)).toBeVisible();
}

/** Explains the first event: no dialog, the request runs as a job and a toast says so. */
async function explainInBackground(page: Page): Promise<void> {
  await page.locator('imt-events-grid [role=row].row').first().click();
  await runCommand(page, 'AI: Explain the selected event');
  const started = page.getByTestId('notice-toast').filter({ hasText: /Explaining event #\d+ — running in Jobs/ });
  await expect(started).toBeVisible();
  await expect(started.getByTestId('notice-toast-primary')).toHaveText('Show in Jobs');
  await expect(page.getByRole('dialog')).toHaveCount(0);
}

/** Saves a bundle and closes the dialog while it is written (a slow export job). */
async function exportBundle(page: Page, name: string): Promise<void> {
  await runCommand(page, 'Share as bundle');
  const dialog = page.getByRole('dialog', { name: 'Share as bundle' });
  await dialog.getByRole('textbox', { name: 'Bundle name' }).fill(name);
  await dialog.getByRole('button', { name: 'Save bundle…' }).click();
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
}

test.describe('jobs', () => {
  test.slow();
  test('a slow AI answer and a slow export run side by side; the toast View opens the answer', async ({ page }, testInfo) => {
    await openAiApp(page, 'mockAiStepMs=600&mockExportMs=8000&mockJobsVisibleMs=200&mockJobsAttentionMs=1000');
    await explainInBackground(page);
    await exportBundle(page, 'Case 7');
    await openJobs(page);
    const pane = jobsPane(page);
    await expect(pane.locator('li').filter({ hasText: 'Explain event' })).toBeVisible();
    await expect(pane.locator('li').filter({ hasText: 'Share bundle Case 7.imtz' })).toContainText('Running');
    await expect(page.getByTestId('status-jobs')).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'jobs-pane', 'imt-jobs-panel');

    // The AI answer finishes first: "Explanation ready" with View, and View opens the answer with its citations.
    const toast = page.getByTestId('notice-toast').filter({ hasText: 'Explanation ready' });
    await expect(toast).toBeVisible({ timeout: 15_000 });
    await expectNoSeriousViolations(page, testInfo, 'jobs-toast', 'imt-notification-toasts');
    await toast.getByTestId('notice-toast-primary').click();
    await expect(page.getByTestId('ai-narrative')).toContainText('failed order submission');
    await page.keyboard.press('Escape');
    await expect(pane.locator('li').filter({ hasText: 'Explain event' })).toContainText('Done');
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

    await exportBundle(page, 'Case 9');
    const failed = pane.locator('li').filter({ hasText: 'Share bundle Case 9.imtz' });
    await expect(failed).toContainText('Failed');
    await expect(failed).toContainText('The disk is full');
    await failed.getByRole('button', { name: /^Retry / }).click();
    await expect(pane.locator('li').filter({ hasText: 'Share bundle Case 9.imtz' }).filter({ hasText: 'Done' })).toBeVisible();
  });
});

test.describe('notifications', () => {
  test('a finished job is an unread message; View reads it; live alerts merge; Mark all read', async ({ page }, testInfo) => {
    await openAiApp(page, 'mockAiStepMs=200&mockJobsVisibleMs=0&mockJobsAttentionMs=0&mockLive=1&mockLiveRate=20&mockLiveRotateMs=0&mockLiveAlertMs=300');
    const bell = page.getByTestId('status-notifications');
    await explainInBackground(page);
    await expect(bell).toContainText('1', { timeout: 10_000 });
    await openNotifications(page);
    const pane = noticesPane(page);
    const finished = pane.locator('li').filter({ hasText: 'Explanation ready' });
    await expect(finished).toHaveClass(/unread/);
    await expectNoSeriousViolations(page, testInfo, 'notifications-pane', 'imt-notifications-panel');
    await finished.getByRole('button', { name: 'View', exact: true }).click();
    await expect(page.getByTestId('ai-narrative')).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(finished).not.toHaveClass(/unread/);

    // Live alerts: the same kind on the same instance merges into one message with a count.
    await page.locator('imt-status-bar').getByRole('button', { name: 'Live tail', exact: true }).click();
    await expect(pane.locator('li').filter({ hasText: /\(×\d+\)/ }).first()).toBeVisible({ timeout: 20_000 });
    await page.locator('imt-status-bar').getByRole('button', { name: 'Live tail', exact: true }).click();
    await pane.getByTestId('notices-mark-all').click();
    await expect(pane.locator('li.unread')).toHaveCount(0);
    await expect(bell).not.toContainText(/\d/);
    await expectNoSeriousViolations(page, testInfo, 'notifications-read', 'imt-notifications-panel');
  });
});

test.describe('research', () => {
  test('a question that needs the web is researched by itself and answered with citations', async ({ page }, testInfo) => {
    await openAiApp(page, 'mockJobsVisibleMs=0');
    const chat = await openChat(page);
    await expect(chat.getByTestId('chat-mode')).toHaveText('Research');
    await ask(chat, 'what does HRESULT 0x80070005 mean for IIS?');
    await waitAnswered(chat);
    const answer = chat.getByTestId('ai-answer-0');
    await expect(answer).toContainText('E_ACCESSDENIED');
    await expect(answer).toContainText('Likely causes and fixes');
    await expect(answer.locator('.sources')).toContainText('Microsoft Learn');
    await expect(answer).toContainText('skipped: a login');
    await expect(chat.getByTestId('chat-agent')).not.toBeChecked();
    await expect(chat.getByTestId('chat-read-along')).not.toBeChecked();
    // The research ran as a job: "Research finished" with View.
    await expect(page.getByTestId('notice-toast').filter({ hasText: 'Research finished' })).toBeVisible();
    await openJobs(page);
    await expect(jobsPane(page).locator('li').filter({ hasText: 'Research: what does HRESULT 0x80070005' })).toContainText('Done');
    await expectNoSeriousViolations(page, testInfo, 'research-answer', 'imt-ai-chat-panel');
  });

  test('Research this on an event confirms a search with workspace data, and the answer opens in the chat', async ({ page }, testInfo) => {
    await openAiApp(page, 'mockJobsVisibleMs=0');
    const chat = await openChat(page);
    await chat.getByTestId('chat-research').click();
    const dialog = page.getByRole('dialog', { name: 'Research the web' });
    await dialog.getByLabel('What should the AI find out?').fill('why does WEB02 deny access?');
    await dialog.getByTestId('research-run').click();
    await expect(dialog.getByTestId('research-flagged')).toContainText('WEB02');
    await expectNoSeriousViolations(page, testInfo, 'research-dialog', '[role=dialog]');
    await dialog.getByLabel('Search text (optional; the question is used when empty)').fill('IIS access denied 0x80070005');
    await dialog.getByLabel('Results to read').selectOption('3');
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
    await expect(chat.getByTestId('chat-agent')).toBeChecked();
    await expect(chat.getByTestId('chat-mode')).toHaveText('Agent');
  });
});
