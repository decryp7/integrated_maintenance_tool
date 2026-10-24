import { readFileSync } from 'node:fs';
import { expect, test } from '@playwright/test';
import { DIALOG_TEST_INPUTS } from '../src/app/dialog-windows/dialog-test-inputs';

// A dialog window is its own small Angular bootstrap (ADR 0050). The browser build can open its route by URL, so this
// boots every dialog of the contract file (generated from the registry, ADR 0054: not a hand-kept list) with only the
// providers of a dialog window and checks, per kind, what the shell guarantees: no missing provider, a body, a visible
// Close/Cancel that works, Escape, and the size rules.
interface Contract {
  kind: 'decision' | 'form' | 'viewer' | 'tool';
  modality: 'modal' | 'modeless';
  requiredChoice: boolean;
  minSize: { width: number; height: number } | null;
}
const CONTRACTS = (JSON.parse(readFileSync('src-tauri/dialog-contracts.json', 'utf8')) as { dialogs: Record<string, Contract> }).dialogs;
const RESULT_KEY = 'debrief.dialog.browserResult';
const CLOSE = /^(close|cancel|not now|dismiss|done|later|got it)\b/i;

const url = (id: string, extra = '') =>
  `/#/dialog?id=${id}&host=main&label=d-main-1&title=${encodeURIComponent(id)}&dialogInput=${encodeURIComponent(JSON.stringify(DIALOG_TEST_INPUTS[id] ?? {}))}${extra}`;

test('the contract file lists every kind', () => {
  const kinds = new Set(Object.values(CONTRACTS).map((c) => c.kind));
  expect([...kinds].sort()).toEqual(['decision', 'form', 'tool', 'viewer']);
  expect(Object.keys(CONTRACTS).length).toBeGreaterThanOrEqual(51);
});

for (const [id, contract] of Object.entries(CONTRACTS)) {
  test(`the ${id} (${contract.kind}) dialog window boots, has a Close that works, and Escape ${contract.requiredChoice ? 'does nothing' : 'closes it'}`, async ({ page }) => {
    const errors: string[] = [];
    page.on('console', (m) => m.type() === 'error' && errors.push(m.text()));
    page.on('pageerror', (e) => errors.push(e.message));
    await page.goto(url(id));
    const shell = page.locator('debrief-dialog-shell');
    await expect(shell).toBeVisible();
    await expect(shell.locator('[data-testid=dialog-loading]')).toHaveCount(0, { timeout: 15_000 });
    await expect(shell.locator('[data-testid=dialog-error]')).toHaveCount(0);
    await expect(shell.locator('main > *').first()).toBeAttached();
    expect(errors.filter((e) => /NG0\d+|No provider|NullInjector/.test(e))).toEqual([]);

    const closeButtons = shell.locator('button').filter({ hasText: CLOSE });
    if (contract.requiredChoice) {
      // A required choice closes with its answer: Escape leaves it open.
      await page.keyboard.press('Escape');
      expect(await page.evaluate((k) => localStorage.getItem(k), RESULT_KEY)).toBeNull();
      return;
    }
    // Every other dialog has a visible Close/Cancel (its own, or the shell's) and Escape does the same.
    await expect(shell.locator('[data-dialog-close]').or(closeButtons).first()).toBeVisible();
    await page.keyboard.press('Escape');
    await expect.poll(() => page.evaluate((k) => localStorage.getItem(k), RESULT_KEY)).toBe('{"result":null}');
  });
}

for (const [id] of Object.entries(CONTRACTS).filter(([, c]) => c.kind === 'decision')) {
  test(`the ${id} decision fits its content: no scrollbar and no gap`, async ({ page }) => {
    await page.setViewportSize({ width: 520, height: 700 });
    await page.goto(url(id));
    const shell = page.locator('debrief-dialog-shell');
    await expect(shell.locator('[data-testid=dialog-loading]')).toHaveCount(0, { timeout: 15_000 });
    // The shell only asks for a taller window; the fit is final when it stops changing (content that loads after the first render).
    let last = '';
    await expect
      .poll(async () => {
        const now = await page.evaluate(() => document.documentElement.dataset['fittedHeight'] ?? '');
        const stable = now !== '' && now === last;
        last = now;
        return stable;
      }, { timeout: 10_000, intervals: [200] })
      .toBe(true);
    const fitted = Number(last);
    await page.setViewportSize({ width: 520, height: fitted });
    const m = await page.evaluate(() => {
      const body = document.querySelector('debrief-dialog-shell main') as HTMLElement;
      return { scroll: body.scrollHeight, client: body.clientHeight };
    });
    expect(m.scroll).toBeLessThanOrEqual(m.client);
  });
}

// The size policy (ADR 0054): a fitted window gets one first fit, after the fonts, and that is its size; a later height
// only comes from content that changed. One dialog of each fitted kind whose content is static at open is checked for a
// second jump (the fit history the browser build records is a single height).
for (const id of ['unsaved-changes', 'paste', 'about']) {
  test(`the ${id} (${CONTRACTS[id]!.kind}) window is fitted once: its first measured height is its size`, async ({ page }) => {
    await page.setViewportSize({ width: 680, height: 560 });
    await page.goto(url(id));
    const shell = page.locator('debrief-dialog-shell');
    await expect(shell.locator('[data-testid=dialog-loading]')).toHaveCount(0, { timeout: 15_000 });
    // The history is final when it stops changing between polls (as in the decision fit above).
    let last = '';
    await expect
      .poll(async () => {
        const now = await page.evaluate(() => document.documentElement.dataset['fitHistory'] ?? '');
        const stable = now !== '' && now === last;
        last = now;
        return stable;
      }, { timeout: 10_000, intervals: [300] })
      .toBe(true);
    const history = (await page.evaluate(() => document.documentElement.dataset['fitHistory'] ?? '')).split(',');
    expect(history, `${id} was fitted more than once: ${history.join(' -> ')}`).toHaveLength(1);
  });
}

for (const [id, c] of Object.entries(CONTRACTS).filter(([, c]) => c.kind === 'viewer')) {
  test(`the ${id} viewer opens at 680x560 at least, keeps its Close in view and does not scroll sideways`, async ({ page }) => {
    expect(c.minSize).toEqual({ width: 680, height: 560 });
    await page.setViewportSize({ width: 680, height: 560 });
    await page.goto(url(id));
    const shell = page.locator('debrief-dialog-shell');
    await expect(shell.locator('[data-testid=dialog-loading]')).toHaveCount(0, { timeout: 15_000 });
    const close = shell.locator('[data-dialog-close]').or(shell.locator('button').filter({ hasText: CLOSE })).first();
    await expect(close).toBeVisible();
    const box = (await close.boundingBox())!;
    expect(box.y + box.height).toBeLessThanOrEqual(560);
    expect(await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)).toBeLessThanOrEqual(0);
  });
}

// A dialog window is a full-height column: the footer sits at the bottom of the window and the body fills the space
// between, at any window size (no sizes left over from the in-page overlay).
for (const size of [{ width: 1021, height: 727 }, { width: 760, height: 560 }]) {
  test(`the settings dialog window fills a ${size.width}x${size.height} window with its footer at the bottom`, async ({ page }) => {
    await page.setViewportSize(size);
    await page.goto('/#/dialog?id=settings&host=main&label=d-main-1&title=Settings&dialogInput=%7B%7D');
    const shell = page.locator('debrief-dialog-shell');
    await expect(shell.locator('[data-testid=dialog-loading]')).toHaveCount(0, { timeout: 15_000 });
    const footer = shell.locator('.footer');
    await expect(footer).toBeVisible();
    const fb = (await footer.boundingBox())!;
    const host = (await shell.locator('debrief-settings-screen').boundingBox())!;
    // The footer ends within the shell's padding of the window bottom; the content spans the window height.
    expect(size.height - (fb.y + fb.height)).toBeLessThanOrEqual(24);
    expect(host.height).toBeGreaterThan(size.height - 48);
  });
}

// A dialog that shows a streamed or loaded answer is not fitted to its first short state: it opens at a usable height
// from the registry, the answer scrolls inside, and the footer stays visible. A small decision dialog still fits exactly.
test('the ai-narrative window with a long answer is at least 480 px tall, scrolls inside and keeps its footer visible', async ({ page }) => {
  const text = Array.from({ length: 60 }, (_, i) => `Paragraph ${i + 1}: the payment gateway timed out after retrying the upstream call [#1].`).join('\n\n');
  const preset = { text, citations: [{ n: 1, id: 1 }], evidence: {}, usage: {} };
  await page.setViewportSize({ width: 680, height: 560 });
  await page.goto(`/#/dialog?id=ai-narrative&host=main&label=d-main-1&title=Explain%20event%201&dialogInput=${encodeURIComponent(JSON.stringify({ preset }))}`);
  const shell = page.locator('debrief-dialog-shell');
  await expect(shell.locator('[data-testid=dialog-loading]')).toHaveCount(0, { timeout: 15_000 });
  const answer = shell.getByTestId('ai-narrative');
  await expect(answer).toContainText('Paragraph 1');
  // A viewer is fitted to its content like any dialog, but the window only ever grows from its 680x560 opening size.
  await expect.poll(() => page.evaluate(() => Number(document.documentElement.dataset['fittedHeight'] ?? 0))).toBeGreaterThanOrEqual(560);
  const m = await answer.evaluate((el) => ({ client: el.clientHeight, scroll: el.scrollHeight, overflowX: el.scrollWidth - el.clientWidth }));
  expect(m.client).toBeGreaterThanOrEqual(300);
  expect(m.scroll).toBeGreaterThan(m.client);
  expect(m.overflowX).toBeLessThanOrEqual(0);
  const close = shell.getByRole('button', { name: 'Close' });
  await expect(close).toBeVisible();
  const box = (await close.boundingBox())!;
  expect(box.y + box.height).toBeLessThanOrEqual(560);
  const win = (await shell.boundingBox())!;
  expect(win.height).toBeGreaterThanOrEqual(480);
});
