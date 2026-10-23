import { $, browser, expect } from '@wdio/globals';
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { chooseMenu, waitForShell } from '../support/app';
import { RUN_DIR_VAR, runLayout } from '../support/paths';

/** Debrief's own log lines of this run (CLEF, one JSON object per line). */
function debriefLog(): string {
  const dir = join(runLayout(process.env[RUN_DIR_VAR]!).debrief, 'logs');
  if (!existsSync(dir)) return '';
  return readdirSync(dir)
    .filter((f) => f.endsWith('.log'))
    .map((f) => readFileSync(join(dir, f), 'utf8'))
    .join('\n');
}

// Regression: Help ▸ Show activity log opened a white window on Windows (WebView2 never finished loading it). The
// new window's UI must start and open Debrief's activity project. The check reads Debrief's own log instead of switching
// the driver to the new window: WebKitWebDriver cannot switch reliably, and a white window cannot log, so
// "IPC open_project from w-N" proves its page loaded and ran.
describe('real app: activity log', () => {
  it('Help ▸ Show activity log opens a working window with Debrief’s log', async () => {
    await waitForShell();
    // The start-up import settles (it re-renders the menus). The grid, not the timeline: the layout restored from the
    // previous spec can leave the timeline in a background tab, and hidden panels are not rendered.
    await $('debrief-events-grid [role=row].row').waitForDisplayed({ timeout: 60_000 });
    await expect($('debrief-status-bar')).toHaveText(/\d[\d,]* events/, { wait: 30_000 });
    const before = await browser.getWindowHandles();

    await chooseMenu('Help', 'Show activity log');
    await browser.waitUntil(async () => (await browser.getWindowHandles()).length > before.length, {
      timeout: 30_000,
      timeoutMsg: 'no new window appeared',
    });
    await browser.waitUntil(() => /"IPC open_project from w-\d+"/.test(debriefLog()), {
      timeout: 60_000,
      timeoutMsg: 'the activity window never ran its UI (no "IPC open_project from w-N" in Debrief’s log): a white window',
    });
    // The `after` hook closes the activity window through Debrief's menu and quits.
  });
});
