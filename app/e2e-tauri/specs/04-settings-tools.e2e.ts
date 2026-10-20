import { $, browser, expect } from '@wdio/globals';
import { button, chooseMenu, dialog, waitForEvents, waitForShell } from '../support/app';

type Scheme = 'system' | 'dark' | 'light';

describe('real app: settings and tools', () => {
  it('Settings switches the colour scheme and closes', async () => {
    await waitForShell();
    await waitForEvents(); // the start-up import settles (it re-renders the menus)
    await button('Settings').click();
    const settings = dialog('Settings');
    await settings.waitForDisplayed();
    await settings.$(`.//button[normalize-space()='Appearance']`).click();
    const schemes = settings.$('[data-testid=scheme-setting]');
    await schemes.waitForDisplayed();

    // Remember the scheme and switch to the other one (the run has its own profile, but leave it as found).
    const before = (await schemes.$(`.//button[@aria-pressed='true']`).getText()).trim().toLowerCase() as Scheme;
    const shown = await $('html').getAttribute('data-scheme');
    const next: Scheme = shown === 'light' ? 'dark' : 'light';
    const option = (s: Scheme) => schemes.$(`.//button[translate(normalize-space(),'DLS','dls')='${s}']`);
    try {
      await option(next).click();
      await expect($('html')).toHaveAttribute('data-scheme', next);
      await expect(option(next)).toHaveAttribute('aria-pressed', 'true');
    } finally {
      await option(before).click();
    }
    await browser.keys('Escape');
    await settings.waitForExist({ reverse: true });
  });

  it('View ▸ Tools ▸ Process Monitor opens the view', async () => {
    await chooseMenu('View', 'Tools', 'Process Monitor');
    const monitor = $('imt-monitor-view');
    await monitor.waitForDisplayed({ timeout: 30_000 });
    await expect(monitor.$('[data-testid=monitor-status]')).toHaveText(
      expect.stringContaining('Stopped'),
      { wait: 30_000 },
    );
  });
});
