import { $, $$, browser, expect } from '@wdio/globals';
import { gridTotal, visibleAlerts, waitForEvents, waitForShell } from '../support/app';

// The app is started with samples/serilog-clef open (IMT_OPEN_PATHS, see wdio.conf.ts): the engine imports it.
describe('real app: open the samples folder', () => {
  it('shows the imported events in the grid and on the timeline', async () => {
    await waitForShell();
    await waitForEvents();

    expect(await gridTotal()).toBeGreaterThan(0);
    await expect($('imt-status-bar')).toHaveText(/\d+ sources?/);
    // Both instances of the sample (web01, web02) have a lane.
    await expect($$('imt-timeline-panel .lane')).toBeElementsArrayOfSize({ gte: 1 });
    // On failure, say how the Explorer rows are laid out (a squeezed name once read as missing).
    const rows = await browser.execute(() =>
      Array.from(document.querySelectorAll('imt-sidebar .src')).map((r) => {
        const name = r.querySelector('.name') as HTMLElement | null;
        return `${Math.round(r.getBoundingClientRect().width)}px row, name ${Math.round(name?.getBoundingClientRect().width ?? -1)}px "${name?.textContent ?? ''}"`;
      }),
    );
    await browser.waitUntil(async () => (await $('imt-sidebar').getText()).includes('web01'), {
      timeout: 15_000,
      timeoutMsg: `the Explorer does not show web01: ${rows.join('; ')}`,
    });
    expect(await visibleAlerts()).toEqual([]);
  });
});
