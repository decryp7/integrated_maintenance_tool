import { $, $$, browser, expect } from '@wdio/globals';
import { gridTotal, waitForEvents, waitForShell } from '../support/app';

/** The one search field of the filter bar: words search messages, a query (`level >= Error`) is applied as a filter. */
const searchField = () => $('[data-testid=query-field]');

describe('real app: query language', () => {
  it('a filter query narrows the grid to errors; clearing it restores the total', async () => {
    await waitForShell();
    await waitForEvents();
    const all = await gridTotal();

    const input = searchField();
    await input.waitForDisplayed();
    await input.click();
    await input.setValue('level >= Error');
    await browser.keys(['Enter']);

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

    await searchField().click();
    await browser.keys(['Control', 'a']);
    await browser.keys(['Backspace']);
    await browser.keys(['Enter']);
    await browser.waitUntil(async () => (await gridTotal()) === all, {
      timeout: 30_000,
      timeoutMsg: `the total did not return to ${all}`,
    });
  });
});
