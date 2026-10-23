import { $, $$, browser, expect } from '@wdio/globals';
import { gridTotal, visibleAlerts, waitForEvents, waitForShell } from '../support/app';

// The app is started with samples/serilog-clef open (DEBRIEF_OPEN_PATHS, see wdio.conf.ts): the engine imports it.
describe('real app: open the samples folder', () => {
  it('shows the imported events in the grid and on the timeline', async () => {
    await waitForShell();
    await waitForEvents();

    expect(await gridTotal()).toBeGreaterThan(0);
    await expect($('debrief-status-bar')).toHaveText(/\d[\d,]* events/);
    // Both instances of the sample (web01, web02) have a lane.
    await expect($$('debrief-timeline-panel .lane')).toBeElementsArrayOfSize({ gte: 1 });
    // The Explorer rows: width and text of each file name.
    const rows = await browser.execute(() =>
      Array.from(document.querySelectorAll('debrief-sidebar .src')).map((r) => {
        const name = r.querySelector('.name') as HTMLElement | null;
        return `${Math.round(r.getBoundingClientRect().width)}px row, name ${Math.round(name?.getBoundingClientRect().width ?? -1)}px "${name?.textContent ?? ''}"`;
      }),
    );
    // The names themselves (text content): WebKitWebDriver leaves ellipsis-truncated text out of getText, although
    // the names are on screen (the rows report their widths).
    // web01 is listed and its name has room on screen (it once collapsed to 0 px).
    expect(rows.filter((r) => r.includes('"web01/') && !r.includes(', name 0px'))).toHaveLength(1);
    expect(await visibleAlerts()).toEqual([]);
  });
});
