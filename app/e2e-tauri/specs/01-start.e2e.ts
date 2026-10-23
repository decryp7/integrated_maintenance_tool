import { $, browser, expect } from '@wdio/globals';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { chooseMenu, dialog, visibleAlerts, waitForEvents, waitForShell } from '../support/app';
import { APP_DIR } from '../support/paths';

const version = (
  JSON.parse(readFileSync(join(APP_DIR, 'src-tauri', 'tauri.conf.json'), 'utf8')) as {
    version: string;
  }
).version;

describe('real app: start-up', () => {
  it('shows the main window without an error banner', async () => {
    await waitForShell();
    // The start-up import re-renders the menus; wait for it so later menu clicks hit what they aim at.
    await waitForEvents();
    await expect(browser).toHaveTitle(/Debrief/);
    const size = await browser.getWindowSize();
    expect(size.width).toBeGreaterThan(0);
    expect(size.height).toBeGreaterThan(0);
    await expect($('[data-testid=crash-banner]')).not.toBeExisting();
    await expect($('[data-testid=safe-mode-banner]')).not.toBeExisting();
    expect(await visibleAlerts()).toEqual([]);
  });

  it('About shows the version of this build', async () => {
    await chooseMenu('Help', 'About Debrief');
    const about = dialog('About Debrief');
    await about.waitForDisplayed();
    await expect(about.$('.version')).toHaveText(version);
    await browser.keys('Escape');
    await about.waitForExist({ reverse: true });
  });
});
