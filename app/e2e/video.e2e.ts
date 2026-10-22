import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';
import { chooseMenu } from './menu-support';

// The Video view runs against the browser build's mock video engine (src/app/video/mock/mock-video-backend.ts):
// File ▸ Open video… "picks" two demo videos, one playable and one whose file is not where it was.

const view = (page: Page) => page.locator('imt-video-view');
const id = (page: Page, testId: string) => view(page).getByTestId(testId);
const frames = (page: Page) => view(page).getByRole('listbox', { name: 'Frames of the video' }).getByRole('option');

async function openVideos(page: Page) {
  await openApp(page);
  await chooseMenu(page, 'File', 'Open video…');
  await expect(id(page, 'video-pick')).toBeVisible();
  // The last video opened is shown; the first is the playable one.
  await id(page, 'video-pick').selectOption({ label: 'checkout-session.mp4' });
  await expect(id(page, 'video-name').or(id(page, 'video-facts'))).toBeVisible();
}

test.describe('Video', () => {
  test('a video opens with its frames, and a frame can be chosen with the keyboard', async ({ page }, testInfo) => {
    await openVideos(page);
    await expect(frames(page)).toHaveCount(6);
    await expect(id(page, 'video-frame-name')).toHaveText('Frame 1 at 00:00');
    await frames(page).nth(2).click();
    await expect(id(page, 'video-frame-name')).toHaveText('Frame 3 at 00:09');
    await page.keyboard.press('ArrowRight');
    await page.keyboard.press('Space');
    await expect(id(page, 'video-frame-name')).toHaveText('Frame 4 at 00:15');
    await expect(frames(page).nth(3)).toHaveAttribute('aria-selected', 'true');
    await expectNoSeriousViolations(page, testInfo, 'video-view', 'imt-video-view');
  });

  test('a note is saved on the chosen frame', async ({ page }) => {
    await openVideos(page);
    await frames(page).nth(1).click();
    const save = id(page, 'video-note-save');
    await expect(save).toBeDisabled();
    await id(page, 'video-note').fill('The spinner never ends');
    await save.click();
    await expect(save).toBeDisabled();
    await expect(id(page, 'video-frame-1')).toContainText('N');
    await expect(id(page, 'video-frame-1')).toHaveAttribute('aria-label', /has a note/);
    await expect(id(page, 'video-note')).toHaveValue('The spinner never ends');
    // Another frame has its own editor.
    await frames(page).nth(0).click();
    await expect(id(page, 'video-note')).toHaveValue('');
  });

  test('the alignment shows how it was decided and can be moved and reset', async ({ page }, testInfo) => {
    await openVideos(page);
    await expect(id(page, 'video-align-by')).toContainText("the video's own creation time");
    await expect(id(page, 'video-align-confidence')).toHaveText('medium');
    await id(page, 'video-align-offset').fill('-2,5');
    await id(page, 'video-align-shift').click();
    await expect(id(page, 'video-align-by')).toContainText('set by you');
    await expect(id(page, 'video-align-evidence')).toContainText('You moved the start by -2.5 s');
    await expect(id(page, 'video-align-auto')).toBeEnabled();
    await expectNoSeriousViolations(page, testInfo, 'video-alignment', 'imt-alignment-panel');
    await id(page, 'video-align-auto').click();
    await expect(id(page, 'video-align-by')).toContainText("the video's own creation time");
    // The model reads the clock in the chosen frame.
    await frames(page).nth(2).click();
    await id(page, 'video-align-clock').click();
    await expect(id(page, 'video-align-by')).toContainText('the clock on the screen');
    await expect(id(page, 'video-align-evidence')).toContainText('The clock in the frame at 00:09 read');
  });

  test('a video that moved is explained and found again', async ({ page }) => {
    await openVideos(page);
    await id(page, 'video-pick').selectOption({ label: 'moved-recording.mov' });
    await expect(id(page, 'video-missing')).toContainText('not where it was');
    await expect(frames(page)).toHaveCount(5);
    await id(page, 'video-locate').click();
    await expect(id(page, 'video-missing')).toHaveCount(0);
  });

  test('removing a video asks first and names what goes', async ({ page }) => {
    await openVideos(page);
    await id(page, 'video-remove').click();
    const dialog = page.getByRole('dialog', { name: 'Remove the video?' });
    await expect(dialog).toContainText('6 frames');
    await expect(dialog).toContainText('original video file is not touched');
    await dialog.getByRole('button', { name: 'Cancel' }).click();
    await expect(dialog).toHaveCount(0);
    await expect(id(page, 'video-pick').locator('option')).toHaveCount(2);

    await id(page, 'video-remove').click();
    await page.getByRole('dialog', { name: 'Remove the video?' }).getByRole('button', { name: 'Remove video' }).click();
    // One video is left, shown by name.
    await expect(id(page, 'video-name')).toHaveText('moved-recording.mov');
    await expect(id(page, 'video-pick')).toHaveCount(0);
  });

  test('Settings ▸ Video shows the FFmpeg runtime', async ({ page }, testInfo) => {
    await openApp(page);
    await page.getByRole('button', { name: 'Settings' }).click();
    const dialog = page.getByRole('dialog', { name: 'Settings' });
    await dialog.getByRole('button', { name: 'Video' }).click();
    const settings = dialog.locator('imt-video-settings');
    await expect(settings.getByRole('heading', { name: 'FFmpeg runtime' })).toBeVisible();
    await expect(settings.getByTestId('video-settings-custom')).toContainText('/opt/homebrew/bin/ffmpeg');
    await expect(settings.getByTestId('video-settings-mac')).toContainText('brew install ffmpeg');
    await expect(settings.getByTestId('video-detected')).toContainText('Homebrew');
    await expectNoSeriousViolations(page, testInfo, 'video-settings', 'imt-video-settings');
  });

  test('without FFmpeg the view says so and points to Settings', async ({ page }) => {
    await page.goto('/?mockVideoRuntime=windows');
    await expect(page.locator('imt-status-bar')).toContainText('events');
    await chooseMenu(page, 'File', 'Open video…');
    await expect(id(page, 'video-runtime-problem')).toContainText('FFmpeg is not set up');
    await expect(id(page, 'video-error-retry').or(id(page, 'video-runtime-setup'))).toBeVisible();
    await id(page, 'video-runtime-setup').click();
    await expect(page.getByRole('dialog', { name: 'Settings' }).locator('imt-video-settings')).toBeVisible();
    await expect(page.getByTestId('video-install')).toContainText('Download FFmpeg');
  });
});
