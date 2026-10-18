import { $, browser, expect } from '@wdio/globals';
import { existsSync } from 'node:fs';
import { quitApp, visibleAlerts, waitForEvents, waitForShell } from '../support/app';
import { runningMarker } from '../support/paths';

describe('real app: quit', () => {
  it('quits cleanly and the next start reports no crash', async () => {
    await waitForShell();
    // Let the start-up import settle first: the menus change while it runs, and a click on File ▸ Close window
    // could land on File ▸ Close (which only clears the workspace).
    await waitForEvents();
    expect(existsSync(runningMarker())).toBe(true);

    // File ▸ Close window ▸ Quit; resolves once the process is gone and its marker removed.
    await quitApp();

    // A fresh app process: no "IMT didn't close properly last time".
    await browser.reloadSession();
    await waitForShell();
    await waitForEvents(); // before the `after` hook quits this instance through the menus
    await expect($('[data-testid=crash-banner]')).not.toBeExisting();
    expect(await visibleAlerts()).toEqual([]);
    // The config's `after` hook quits this second instance cleanly too.
  });
});
