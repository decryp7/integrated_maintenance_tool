import { expect, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';
import { chooseMenu } from './menu-support';

// "Set up analysis for this folder…" against the mock backend: open a folder, a message in Notifications offers the wizard,
// select proposals, review the profile diff, apply, and the timeline lanes and markers change.

test.describe('Folder set-up', () => {
  test('open folder → wizard → select proposals → apply → lanes and markers updated', async ({ page }, testInfo) => {
    await openApp(page);
    const lanes = page.locator('debrief-timeline-panel .lane-label .key');
    await expect(lanes.first()).toBeVisible();
    const before = await lanes.allInnerTexts();
    expect(before.some((l) => /\d{3,4}$/.test(l))).toBe(true); // lanes include process ids

    await chooseMenu(page, 'File', 'Open folder…');
    // Offered in Notifications (the bell), not as a banner over the workspace.
    await expect(page.getByTestId('folder-setup-hint')).toHaveCount(0);
    await page.getByTestId('status-notifications').click();
    const notices = page.locator('debrief-notifications-panel');
    // Offered once the folder is imported (the import can take a few seconds while the suite runs).
    await expect(notices).toContainText('New folder? Set up analysis', { timeout: 15_000 });
    await notices.getByRole('button', { name: 'Set up analysis…' }).click();

    const dialog = page.getByRole('dialog', { name: 'Set up analysis for this folder' });
    await expect(dialog.getByTestId('survey-facts')).toContainText('Sampled');
    await expect(dialog.getByRole('group', { name: 'Instances and lanes' })).toBeVisible();
    await expect(dialog.getByTestId('proposal-instance:MachineName').getByTestId('lane-preview')).toContainText('WEB02');
    await expect(dialog.getByTestId('skipped')).toContainText('Skipped proposals (1)');
    await expectNoSeriousViolations(page, testInfo, 'folder-setup-proposals', '[role=dialog]');

    // Opt in to dropping the cache noise as well; deselect the saved query.
    await dialog.getByTestId('proposal-noise:0').getByRole('checkbox').check();
    await dialog.getByTestId('proposal-query:error:0').getByRole('checkbox').uncheck();
    await dialog.getByTestId('review').click();
    await expect(dialog.getByTestId('diff')).toContainText('MachineName');
    // The run rule extends the profile's [[span]] "run" rule with the proposed start and stop markers.
    await expect(dialog.getByTestId('diff')).toContainText('[[span]]');
    await expect(dialog.getByTestId('diff')).toContainText('reindex-start');
    await expect(dialog.getByTestId('diff-summary')).toContainText('added');
    await expectNoSeriousViolations(page, testInfo, 'folder-setup-review', '[role=dialog]');
    await dialog.getByTestId('apply').click();
    await expect(dialog.getByTestId('applied')).toBeVisible();
    await dialog.getByTestId('open-timeline').click();
    await expect(dialog).toHaveCount(0);

    // One lane per machine now, the proposed marker is in the legend; the wizard opens again from the File menu.
    await expect.poll(async () => (await lanes.allInnerTexts()).some((l) => /\d{3,4}$/.test(l))).toBe(false);
    await expect(lanes.filter({ hasText: 'WEB02' })).toHaveCount(1);
    await expect(page.locator('debrief-timeline-panel').getByRole('group', { name: 'Markers' })).toContainText('Reindex start');
    await chooseMenu(page, 'File', 'Set up analysis for this folder…');
    const again = page.getByRole('dialog', { name: 'Set up analysis for this folder' });
    await expect(again.getByTestId('survey-facts')).toBeVisible();
    await again.getByRole('button', { name: 'Close', exact: true }).click();
    await expect(again).toHaveCount(0);
  });
});
