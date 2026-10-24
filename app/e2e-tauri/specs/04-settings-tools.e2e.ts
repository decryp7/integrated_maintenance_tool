import { $, browser, expect } from '@wdio/globals';
import { button, chooseMenu, dismissDialogWindow, waitForEvents, waitForShell, withDialogWindow } from '../support/app';

type Scheme = 'system' | 'dark' | 'light';

describe('real app: settings and tools', () => {
  it('Settings switches the colour scheme and closes', async () => {
    await waitForShell();
    await waitForEvents(); // the start-up import settles (it re-renders the menus)
    await button('Settings').click();
    // Settings is a dialog window of its own (ADR 0050); the colour scheme it sets reaches the main window through UiSync.
    const scheme = (s: Scheme) =>
      withDialogWindow('settings', async () => {
        const schemes = $('[data-testid=scheme-setting]');
        await $(`//button[normalize-space()='Appearance']`).click();
        await schemes.waitForDisplayed();
        await schemes.$(`.//button[translate(normalize-space(),'DLS','dls')='${s}']`).click();
        await expect(schemes.$(`.//button[translate(normalize-space(),'DLS','dls')='${s}'][@aria-pressed='true']`)).toExist();
      });
    const current = () =>
      withDialogWindow('settings', async () => {
        await $(`//button[normalize-space()='Appearance']`).click();
        const schemes = $('[data-testid=scheme-setting]');
        await schemes.waitForDisplayed();
        return (await schemes.$(`.//button[@aria-pressed='true']`).getText()).trim().toLowerCase() as Scheme;
      });

    // Remember the scheme and switch to the other one (the run has its own profile, but leave it as found).
    const before = await current();
    const shown = await $('html').getAttribute('data-scheme');
    const next: Scheme = shown === 'light' ? 'dark' : 'light';
    try {
      await scheme(next);
      await expect($('html')).toHaveAttribute('data-scheme', next);
    } finally {
      await scheme(before);
    }
    await dismissDialogWindow('settings');
  });

  it('View ▸ Tools ▸ Process Monitor opens the view', async () => {
    await chooseMenu('View', 'Tools', 'Process Monitor');
    const monitor = $('debrief-monitor-view');
    await monitor.waitForDisplayed({ timeout: 30_000 });
    await expect(monitor.$('[data-testid=monitor-status]')).toHaveText(
      expect.stringContaining('Stopped'),
      { wait: 30_000 },
    );
  });
});
