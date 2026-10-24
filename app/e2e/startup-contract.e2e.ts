import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';

// The start-up contract (ADR 0058). A window shows a splash until Angular renders, paints its frame (activity bar,
// status bar) without waiting for any IPC, reports `window_ready` after that first paint, and says calmly when
// Debrief reloaded it. The IPC tests run the real Tauri backends against a fake `__TAURI_INTERNALS__` whose replies
// the test holds and releases: a paused or throttled web content process, or a lost first IPC message.

/** Installs a fake Tauri IPC that records every call and holds every reply until `release` is called. */
async function fakeTauri(page: Page, readyReply: { recovered: string | null }): Promise<void> {
  await page.addInitScript((reply) => {
    type Held = { cmd: string; resolve: (v: unknown) => void; reject: (e: unknown) => void };
    const w = window as unknown as Record<string, unknown>;
    const held: Held[] = [];
    const calls: string[] = [];
    const args: { cmd: string; args: unknown }[] = [];
    let open = false;
    let cb = 1;
    const answer = (h: Held) => (h.cmd === 'window_ready' ? h.resolve(reply) : h.reject('unavailable in this test'));
    w['__ipcCalls'] = calls;
    w['__ipcArgs'] = args;
    w['__ipcRelease'] = () => {
      open = true;
      held.splice(0).forEach(answer);
    };
    w['__TAURI_EVENT_PLUGIN_INTERNALS__'] = { unregisterListener: () => undefined };
    w['__TAURI_INTERNALS__'] = {
      metadata: { currentWindow: { label: 'main' }, currentWebview: { windowLabel: 'main', label: 'main' } },
      transformCallback: (f: unknown) => {
        const id = cb++;
        w[`_${id}`] = f;
        return id;
      },
      unregisterCallback: () => undefined,
      convertFileSrc: (p: string) => p,
      invoke: (cmd: string, a: unknown) =>
        new Promise((resolve, reject) => {
          calls.push(cmd);
          args.push({ cmd, args: a });
          const h = { cmd, resolve, reject };
          if (open) answer(h);
          else held.push(h);
        }),
    };
  }, readyReply);
}

/** The start-up marks the page sent (`startup_mark`), in the order sent. */
const marks = (page: Page) =>
  page.evaluate(() =>
    (window as unknown as { __ipcArgs: { cmd: string; args: { mark: string; atEpochMs: number } }[] }).__ipcArgs
      .filter((c) => c.cmd === 'startup_mark')
      .map((c) => ({ ...c.args, origin: performance.timeOrigin, now: Date.now() })),
  );

const calls = (page: Page) => page.evaluate(() => (window as unknown as { __ipcCalls: string[] }).__ipcCalls.slice());

test.describe('start-up contract', () => {
  test('the window shows a splash until Angular renders the shell', async ({ page }) => {
    let release!: () => void;
    const held = new Promise<void>((r) => (release = r));
    await page.route(/\/main(-[A-Z0-9]+)?\.js$/, async (route) => {
      await held;
      await route.continue();
    });
    await page.goto('/', { waitUntil: 'commit' });
    await expect(page.getByTestId('boot-splash')).toContainText('Starting Debrief…');
    release();
    await expect(page.getByRole('navigation', { name: 'Activity bar' })).toBeVisible();
    await expect(page.getByTestId('boot-splash')).toHaveCount(0);
  });

  test('with every IPC reply withheld (a paused web content process) the frame still paints and reports ready; it recovers on resume', async ({ page }) => {
    await fakeTauri(page, { recovered: null });
    await page.goto('/');
    // Nothing has been answered, yet the frame is there: activity bar, status bar, the dock.
    await expect(page.getByRole('navigation', { name: 'Activity bar' }).getByRole('button', { name: 'Explorer' })).toBeVisible();
    await expect(page.locator('debrief-status-bar')).toBeVisible();
    await expect(page.locator('.dv-groupview').first()).toBeVisible();
    // Ready is reported after the first paint, without waiting for any other reply.
    await expect.poll(() => calls(page)).toContain('window_ready');
    expect(await calls(page)).toContain('workspace_summary');
    // The process resumes: every reply arrives (all but `window_ready` fail here); the window stays usable.
    await page.evaluate(() => (window as unknown as { __ipcRelease: () => void }).__ipcRelease());
    await expect(page.locator('debrief-status-bar')).toContainText('No workspace');
    await expect(page.getByRole('navigation', { name: 'Activity bar' }).getByRole('button', { name: 'Explorer' })).toBeVisible();
    expect((await calls(page)).filter((c) => c === 'window_ready')).toHaveLength(1);
  });

  test('the main window reports its start-up marks once each, as wall-clock times, without waiting for any reply', async ({ page }) => {
    await fakeTauri(page, { recovered: null });
    await page.goto('/');
    // The first paint is reported before any reply arrives (the timings never hold up the start).
    await expect.poll(async () => (await marks(page)).map((m) => m.mark)).toContain('firstPaint');
    await page.evaluate(() => (window as unknown as { __ipcRelease: () => void }).__ipcRelease());
    // The first workspace load (here: a failed one) and the first frame after it.
    await expect.poll(async () => (await marks(page)).map((m) => m.mark).sort()).toEqual(['firstInteractive', 'firstPaint', 'workspaceLoaded']);
    const sent = await marks(page);
    for (const m of sent) {
      expect(m.atEpochMs).toBeGreaterThanOrEqual(Math.floor(m.origin));
      expect(m.atEpochMs).toBeLessThanOrEqual(m.now);
    }
    const at = (name: string) => sent.find((m) => m.mark === name)!.atEpochMs;
    expect(at('firstInteractive')).toBeGreaterThanOrEqual(at('workspaceLoaded'));
    expect(at('workspaceLoaded')).toBeGreaterThanOrEqual(at('firstPaint'));
  });

  for (const [why, text] of [
    ['terminated', "The window's content stopped unexpectedly and was reloaded. Your open files and view are kept."],
    ['unresponsive', 'The window did not finish starting and was reloaded. Your open files and view are kept.'],
  ] as const) {
    test(`a window Debrief reloaded (${why}) says so calmly`, async ({ page }) => {
      await fakeTauri(page, { recovered: why });
      await page.goto('/');
      await page.evaluate(() => (window as unknown as { __ipcRelease: () => void }).__ipcRelease());
      await expect(page.getByText(text)).toBeVisible();
    });
  }

  test('a recovery reload brings back the query, the level filter and the selected event', async ({ page }) => {
    await openApp(page);
    const levels = page.getByRole('toolbar', { name: 'Levels' });
    await levels.getByRole('button', { name: /^ERR/ }).click();
    await expect(levels.getByRole('button', { name: /^ERR/ })).toHaveAttribute('aria-pressed', 'true');
    const rows = page.locator('debrief-events-grid [role=row].row');
    const word = ((await rows.first().locator('.msg').innerText()).trim().split(/\s+/)[0] ?? '').replace(/[^\w]/g, '');
    expect(word.length).toBeGreaterThan(2);
    await page.getByTestId('query-field').fill(word);
    await expect(page.locator('debrief-events-grid .bar .total')).not.toHaveText('0');
    const row = rows.nth(1);
    await row.click();
    await expect(row).toHaveAttribute('aria-selected', 'true');
    const message = (await row.locator('.msg').innerText()).trim();
    // The checkpoint reached the backend (Rust's memory in the app; session storage in the mock).
    const checkpoint = () => page.evaluate(() => sessionStorage.getItem('debrief.mock.uiCheckpoint') ?? '');
    await expect.poll(checkpoint).toContain(`"textInput":"${word}"`);
    await expect.poll(checkpoint).toMatch(/"recordId":\d+/);

    // Debrief reloads the window after its content process terminated.
    await page.evaluate(() => sessionStorage.setItem('debrief.mock.recover', 'terminated'));
    await page.reload();
    await expect(page.getByText("The window's content stopped unexpectedly and was reloaded. Your open files and view are kept.")).toBeVisible();
    await expect(page.getByTestId('query-field')).toHaveValue(word);
    await expect(levels.getByRole('button', { name: /^ERR/ })).toHaveAttribute('aria-pressed', 'true');
    const selected = page.locator('debrief-events-grid [role=row].row[aria-selected=true]');
    await expect(selected).toHaveCount(1);
    await expect(selected.locator('.msg')).toHaveText(message);
  });

  test('a reload the user asks for (no recovery) starts clean', async ({ page }) => {
    await openApp(page);
    await page.getByTestId('query-field').fill('order');
    await expect.poll(() => page.evaluate(() => sessionStorage.getItem('debrief.mock.uiCheckpoint') ?? '')).toContain('"textInput":"order"');
    await page.reload();
    await expect(page.getByRole('grid', { name: 'Events' })).toBeVisible();
    await expect(page.getByTestId('query-field')).toHaveValue('');
  });
});

