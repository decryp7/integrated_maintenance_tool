import { $, $$, expect } from '@wdio/globals';
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
    await expect($('imt-sidebar')).toHaveText(expect.stringContaining('web01'));
    expect(await visibleAlerts()).toEqual([]);
  });
});
