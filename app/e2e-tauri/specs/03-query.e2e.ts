import { $, $$, browser, expect } from '@wdio/globals';
import { gridTotal, tab, waitForEvents, waitForShell } from '../support/app';

describe('real app: query language', () => {
  it('a filter query narrows the grid to errors; clearing it restores the total', async () => {
    await waitForShell();
    await waitForEvents();
    const all = await gridTotal();

    await tab('Query').click();
    const input = $('[role=combobox][aria-label="Query"]');
    await input.waitForDisplayed();
    await input.click();
    await input.setValue('level >= Error');
    await browser.keys(['Control', 'Enter']);
    await expect($('imt-query-view .applied')).toHaveText(
      expect.stringContaining('level >= Error'),
      { wait: 30_000 },
    );

    await tab('Events').click();
    // Wait for the narrowed total itself (the bar can be between two states).
    await browser.waitUntil(
      async () => {
        const n = await gridTotal();
        return n > 0 && n < all;
      },
      { timeout: 30_000, timeoutMsg: `the total did not drop below ${all}` },
    );
    const rows = $$('imt-events-grid [role=row].row');
    await expect(rows[0]).toHaveText(/ERR|FTL/);
    for (const row of await rows.getElements())
      await expect(row).not.toHaveText(/\b(INF|WRN|DBG|VRB)\b/);

    await tab('Query').click();
    await $('imt-query-view .applied').$(`.//button[normalize-space()='Clear']`).click();
    await tab('Events').click();
    await browser.waitUntil(async () => (await gridTotal()) === all, {
      timeout: 30_000,
      timeoutMsg: `the total did not return to ${all}`,
    });
  });
});
