import { $, browser, expect } from '@wdio/globals';
import { existsSync } from 'node:fs';
import { quitApp, visibleAlerts, waitForShell } from '../support/app';
import { runningMarker } from '../support/paths';

describe('real app: quit', () => {
  it('quits cleanly and the next start reports no crash', async () => {
    await waitForShell();
    // Let the start-up import settle first: the menus change while it runs, and a click on File ▸ Close window
    // could land on File ▸ Close (which only clears the workspace). The grid is enough here: the restored layout of
    // the previous spec can leave the timeline in a background tab (the open-samples spec covers the timeline).
    await $('debrief-events-grid [role=row].row').waitForDisplayed({ timeout: 60_000 });
    expect(existsSync(runningMarker())).toBe(true);

    // File ▸ Close window ▸ Quit; resolves once the process is gone and its marker removed.
    await quitApp();

    // A fresh app process: no "Debrief didn't close properly last time".
    await browser.reloadSession();
    await waitForShell();
    // The restored workspace is back (the grid shows events) before the `after` hook quits this instance through
    // the menus. The timeline is covered by the open-samples spec; on a slow runner its lanes can lag the grid here.
    await $('debrief-events-grid [role=row].row').waitForDisplayed({ timeout: 60_000 });
    await expect($('[data-testid=crash-banner]')).not.toBeExisting();
    expect(await visibleAlerts()).toEqual([]);
    // The config's `after` hook quits this second instance cleanly too.
  });
});
