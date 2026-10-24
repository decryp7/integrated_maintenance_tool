import { expect, Page, test } from '@playwright/test';

// The start-up contract (ADR 0057). A window shows a splash until Angular renders, paints its frame (activity bar,
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
    let open = false;
    let cb = 1;
    const answer = (h: Held) => (h.cmd === 'window_ready' ? h.resolve(reply) : h.reject('unavailable in this test'));
    w['__ipcCalls'] = calls;
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
      invoke: (cmd: string) =>
        new Promise((resolve, reject) => {
          calls.push(cmd);
          const h = { cmd, resolve, reject };
          if (open) answer(h);
          else held.push(h);
        }),
    };
  }, readyReply);
}

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

  for (const [why, text] of [
    ['terminated', "The window's content stopped unexpectedly and was reloaded. Your open files are kept."],
    ['unresponsive', 'The window did not finish starting and was reloaded. Your open files are kept.'],
  ] as const) {
    test(`a window Debrief reloaded (${why}) says so calmly`, async ({ page }) => {
      await fakeTauri(page, { recovered: why });
      await page.goto('/');
      await page.evaluate(() => (window as unknown as { __ipcRelease: () => void }).__ipcRelease());
      await expect(page.getByText(text)).toBeVisible();
    });
  }
});
