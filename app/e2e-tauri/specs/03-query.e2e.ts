import { $, browser, expect } from '@wdio/globals';
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
    // The rows are read in one call: a webdriver round trip per row is slow and the rows re-render while it runs.
    // The total drops before the first page of the narrowed result is drawn, so wait for rows with text.
    // Rows are redrawn while the narrowed page loads: take the first reading that has loaded rows, in one call.
    let levels: string[] = [];
    await browser.waitUntil(
      async () => {
        const texts = await browser.execute(() =>
          Array.from(document.querySelectorAll('debrief-events-grid [role=row].row')).map((r) => r.textContent ?? ''),
        );
        levels = texts.filter((t) => t.trim().length > 0);
        return levels.length > 0;
      },
      { timeout: 30_000, timeoutMsg: 'the narrowed rows were not drawn' },
    );
    expect(levels.length).toBeGreaterThan(0);
    expect(levels[0]).toMatch(/ERR|FTL/);
    for (const text of levels) expect(text).not.toMatch(/\b(INF|WRN|DBG|VRB)\b/);

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
