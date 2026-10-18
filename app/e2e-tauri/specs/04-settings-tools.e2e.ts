import { $, browser, expect } from '@wdio/globals';
import { button, chooseMenu, dialog, waitForEvents, waitForShell } from '../support/app';

type Theme = 'system' | 'dark' | 'light';

describe('real app: settings and tools', () => {
  it('Settings switches the theme and closes', async () => {
    await waitForShell();
    await waitForEvents(); // the start-up import settles (it re-renders the menus)
    await button('Settings').click();
    const settings = dialog('Settings');
    await settings.waitForDisplayed();

    // Remember the theme and switch to the other one (the run has its own profile, but leave it as found).
    const pressed = await settings.$(
      `.//button[@aria-pressed='true'][normalize-space()='system' or normalize-space()='dark' or normalize-space()='light']`,
    );
    const before = (await pressed.getText()).trim().toLowerCase() as Theme;
    const shown = await $('html').getAttribute('data-theme');
    const next: Theme = shown === 'light' ? 'dark' : 'light';
    try {
      await settings.$(`.//button[normalize-space()='${next}']`).click();
      await expect($('html')).toHaveAttribute('data-theme', next);
      await expect(settings.$(`.//button[normalize-space()='${next}']`)).toHaveAttribute(
        'aria-pressed',
        'true',
      );
    } finally {
      await settings.$(`.//button[normalize-space()='${before}']`).click();
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
