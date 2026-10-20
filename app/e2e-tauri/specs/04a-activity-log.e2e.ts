import { $, browser, expect } from '@wdio/globals';
import { chooseMenu, waitForShell } from '../support/app';

// Regression: Help ▸ Show activity log opened a white window on Windows (the command that built the window was
// synchronous, and WebView2 never finished loading it). The new window must load IMT and show its own log.
describe('real app: activity log', () => {
  it('Help ▸ Show activity log opens a working window with IMT’s log', async () => {
    await waitForShell();
    // The start-up import settles (it re-renders the menus). The grid, not the timeline: the layout restored from the
    // previous spec can leave the timeline in a background tab, and hidden panels are not rendered.
    await $('imt-events-grid [role=row].row').waitForDisplayed({ timeout: 60_000 });
    await expect($('imt-status-bar')).toHaveText(/\d[\d,]* events/, { wait: 30_000 });
    const main = await browser.getWindowHandle();
    const before = await browser.getWindowHandles();

    await chooseMenu('Help', 'Show activity log');
    await browser.waitUntil(async () => (await browser.getWindowHandles()).length > before.length, {
      timeout: 30_000,
      timeoutMsg: 'no new window appeared',
    });
    const added = (await browser.getWindowHandles()).find((h) => !before.includes(h))!;
    try {
      await browser.switchToWindow(added);
      // The UI loaded (a white window has no menu bar) and the activity project opened IMT's log.
      await $('[role=menubar][aria-label="Application menu"]').waitForDisplayed({ timeout: 60_000 });
      await $('imt-events-grid [role=row].row').waitForDisplayed({ timeout: 60_000 });
      await expect($('imt-status-bar')).toHaveText(/\d[\d,]* events/, { wait: 30_000 });
    } finally {
      // Back to the main window; the `after` hook closes the activity window through IMT's menu (the driver's
      // closeWindow closed the main window instead under WebKitWebDriver) and then quits.
      await browser.switchToWindow(main).catch(() => undefined);
    }
  });
});
