import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname } from 'node:path';
import { expect, Locator, Page, test } from '@playwright/test';
import { DIALOG_TEST_INPUTS } from '../src/app/dialog-windows/dialog-test-inputs';
import { runCommand } from './analysis-support';
import { openApp } from './app';
import { settleAnimations, settleDockLayout } from './axe';
import { openAppWith } from './mock-url';

/**
 * Shell baseline (Paddock X50, plan package W0): screenshots of the shell and workbench layer taken BEFORE any
 * workbench code moves out of Debrief, so the migration (plan M5) can prove "no behaviour change" by pixel diff.
 * Browser build with the mock backend; every shot runs with reduced motion, animations off and the mock's
 * deterministic demo data. Chromium and WebKit each have their own snapshots (named per project and platform, in
 * `shell-baseline.e2e.ts-snapshots/`); create or refresh them with
 * `npx playwright test shell-baseline --update-snapshots` and commit the PNGs.
 *
 * Matrix per browser (30 screenshots, 60 with Chromium and WebKit; the full cross product is not needed):
 *  - Layouts x sizes: the default layout (Investigate perspective) in dark AND light, the fullest layout and the
 *    Profiling perspective in dark, each at 1440x900, 1920x1080 and 800x600 (12). The old Focused/Full layouts are
 *    gone (changelog 1.1); perspectives replaced them, so "Full" is the Diagnostics perspective (events and timeline
 *    plus Problems, Jobs, ... below them), the layout with the most panels.
 *  - States at 1440x900 in both themes (dark, light), 9 each (18): one dialog per kind (decision `unsaved-changes`,
 *    form `clock`, viewer `cheat-sheet`, tool `settings`; the dialog route of the browser build), the service-health
 *    chip in the down state with its details, a job running (Jobs pane, held at its end by `mockJobsHold=ai`), the
 *    command palette open, the panel menu with "Pop out to window", and a pop-out window (the browser build opens a
 *    real `window.open` popup of the same app, so it is shown as such).
 *
 * The dock's first layout races the load (see `race`), so each dock screenshot also has a `*.layout.txt` next to it
 * with the rounded group boxes; a load whose boxes differ is retried before the pixels are compared.
 *
 * Volatile, masked: the status bar's clock / load-time text and the Jobs pane's elapsed times (see `volatile`).
 */
// A retried load (see `race`) needs more than the default 30 s.
test.setTimeout(120_000);

const SIZES = [
  { name: '1440x900', width: 1440, height: 900 },
  { name: '1920x1080', width: 1920, height: 1080 },
  { name: '800x600', width: 800, height: 600 },
] as const;
const SCHEMES = ['dark', 'light'] as const;
type Scheme = (typeof SCHEMES)[number];
type Size = { width: number; height: number };
const DESKTOP: Size = { width: 1440, height: 900 };

/** Text that changes between runs: drawn over with a mask. */
const volatile = (page: Page): Locator[] => [
  page.locator('debrief-status-bar .clock, debrief-status-bar time, debrief-jobs-panel time, debrief-jobs-panel .elapsed'),
];

async function prepare(page: Page, scheme: Scheme, size: Size): Promise<void> {
  await page.setViewportSize(size);
  await page.addInitScript((s) => {
    localStorage.setItem('debrief.prefs.v1', JSON.stringify({ theme: 'calm', scheme: s }));
  }, scheme);
  await page.emulateMedia({ colorScheme: scheme, reducedMotion: 'reduce' });
}

async function settled(page: Page): Promise<void> {
  await page.evaluate(() => document.fonts.ready);
  await settleAnimations(page);
  // Dialog and pop-out windows have no dock to settle.
  if (await page.locator('.dv-groupview').count()) {
    // The timeline's summary line ("1 crash - 404 errors - Review") arrives after the analysis and makes the panel
    // taller by 18 px, which moves the splitter: wait for it where the timeline is shown.
    if (await page.locator('debrief-timeline-panel').count()) await expect(page.locator('debrief-timeline-panel').getByText(/\d+ errors/)).toBeVisible();
    await settleDockLayout(page);
    // The timeline panel resizes by a few pixels once its lanes have measured (the splitter moves): wait until
    // every dock group and the events grid's header hold still for 30 animation frames in a row.
    await page.waitForFunction(
      () => {
        const w = window as unknown as { __baseBoxes?: string; __baseStable?: number };
        const now = [...document.querySelectorAll('.dv-groupview, debrief-events-grid .row, debrief-timeline-panel .lane')]
          .slice(0, 40)
          .map((e) => {
            const r = e.getBoundingClientRect();
            return `${r.x.toFixed(1)},${r.y.toFixed(1)},${r.width.toFixed(1)},${r.height.toFixed(1)}`;
          })
          .join('|');
        w.__baseStable = now === w.__baseBoxes ? (w.__baseStable ?? 0) + 1 : 0;
        w.__baseBoxes = now;
        return w.__baseStable >= 30;
      },
      undefined,
      { polling: 'raf', timeout: 15_000 },
    );
  }
  await page.mouse.move(0, 0);
}

const pendingLayouts = new Map<string, string>();

/** The dock's group boxes, rounded: what the layout race moves by a few pixels. */
const layoutSignature = (page: Page): Promise<string> =>
  page.evaluate(() =>
    [...document.querySelectorAll('.dv-groupview')]
      .map((g) => {
        const r = g.getBoundingClientRect();
        return [r.x, r.y, r.width, r.height].map(Math.round).join(',');
      })
      .join(' | '),
  );

/**
 * Compares the dock's layout with the one stored next to the screenshot (`*.layout.txt`) and throws a retryable error
 * when it differs. When baselines are updated the signature is written once two loads in a row agree on it.
 */
async function confirmLayout(page: Page, name: string): Promise<void> {
  const info = test.info();
  const file = info.snapshotPath(`${name}.layout.txt`);
  const now = await layoutSignature(page);
  if (info.config.updateSnapshots !== 'none' && (info.config.updateSnapshots !== 'missing' || !existsSync(file))) {
    if (pendingLayouts.get(file) !== now) {
      pendingLayouts.set(file, now);
      throw new Error(`pixels: confirming the layout of ${name} with a second load`);
    }
    mkdirSync(dirname(file), { recursive: true });
    writeFileSync(file, now);
    return;
  }
  const expected = readFileSync(file, 'utf8');
  if (now !== expected) throw new Error(`pixels: layout race in ${name}: ${now} (expected ${expected})`);
}

async function shot(page: Page, name: string, scheme: Scheme, target: Page | Locator = page): Promise<void> {
  await settled(page);
  if (await page.locator('.dv-groupview').count()) await confirmLayout(page, `${name}-${scheme}`);
  await expect(target).toHaveScreenshot(`${name}-${scheme}.png`, { animations: 'disabled', caret: 'hide', mask: volatile(page), maxDiffPixelRatio: 0.005, timeout: 4000 });
}

/**
 * The dock's first layout races the status bar and the load: on a busy machine the Timeline/Events splitter ends 3 px
 * off (an app-level nondeterminism; Chromium 300 or 303 px, noted in shell-baseline.md). A screenshot that differs is
 * therefore taken again from a fresh load, up to 8 times; a real regression differs on every try and still fails.
 */
async function race(page: Page, body: () => Promise<void>): Promise<void> {
  for (let attempt = 1; ; attempt++) {
    try {
      await body();
      return;
    } catch (e) {
      if (attempt >= 8 || !/toHaveScreenshot|pixels/.test(String(e))) throw e;
      // The next load starts from the default layout, not from the layout or perspective this load saved.
      await page.evaluate(() => localStorage.clear()).catch(() => undefined);
      await page.goto('about:blank');
    }
  }
}

async function openThemedApp(page: Page, scheme: Scheme, size: Size, search = ''): Promise<void> {
  await prepare(page, scheme, size);
  if (search) await openAppWith(page, search);
  else await openApp(page);
  await expect(page.locator('html')).toHaveAttribute('data-scheme', scheme);
}

/** Switches perspective from the activity bar's switcher. */
async function perspective(page: Page, name: string): Promise<void> {
  await page.locator('debrief-activity-bar [data-activity-id=perspectives]').click();
  const menu = page.getByRole('menu', { name: 'Perspectives' });
  await menu.getByRole('menuitemcheckbox', { name }).click();
  await expect(menu).toHaveCount(0);
  await expect(page.locator('debrief-activity-bar [data-activity-id=perspectives]')).toBeVisible();
}

test.describe('shell baseline: layouts and sizes', () => {
  for (const size of SIZES) {
    test(`default layout, light, ${size.name}`, async ({ page }) => {
      await race(page, async () => {
        await openThemedApp(page, 'light', size);
        await shot(page, `layout-default-${size.name}`, 'light');
      });
    });
    test(`default layout, dark, ${size.name}`, async ({ page }) => {
      await race(page, async () => {
        await openThemedApp(page, 'dark', size);
        await shot(page, `layout-default-${size.name}`, 'dark');
      });
    });
    test(`fullest layout (Diagnostics), dark, ${size.name}`, async ({ page }) => {
      await race(page, async () => {
        await openThemedApp(page, 'dark', size);
        await perspective(page, 'Diagnostics');
        await expect(page.getByRole('tab', { name: /^Problems/ })).toBeVisible();
        await shot(page, `layout-full-${size.name}`, 'dark');
      });
    });
    test(`profiling perspective, dark, ${size.name}`, async ({ page }) => {
      await race(page, async () => {
        await openThemedApp(page, 'dark', size);
        await perspective(page, 'Profiling');
        await expect(page.getByTestId('captures-landing')).toBeVisible();
        await shot(page, `layout-profiling-${size.name}`, 'dark');
      });
    });
  }
});

for (const scheme of SCHEMES) {
  test.describe(`shell baseline: states, ${scheme}, 1440x900`, () => {
    // One dialog per kind, each on the dialog route of the browser build (the dialog window's own bootstrap).
    for (const [kind, id] of [['decision', 'unsaved-changes'], ['form', 'clock'], ['viewer', 'cheat-sheet'], ['tool', 'settings']] as const) {
      test(`dialog kind ${kind} (${id})`, async ({ page }) => {
        await prepare(page, scheme, DESKTOP);
        const input = encodeURIComponent(JSON.stringify(DIALOG_TEST_INPUTS[id] ?? {}));
        await page.goto(`/#/dialog?id=${id}&host=main&label=d-main-1&title=${encodeURIComponent(id)}&dialogInput=${input}`);
        const shell = page.locator('debrief-dialog-shell');
        await expect(shell).toBeVisible();
        await expect(shell.locator('[data-testid=dialog-loading]')).toHaveCount(0);
        await expect(shell.locator('main > *').first()).toBeAttached();
        await shot(page, `dialog-${kind}`, scheme);
      });
    }

    test('service health chip down, with its details', async ({ page }) => {
      await race(page, async () => {
        await openThemedApp(page, scheme, DESKTOP);
        await page.waitForFunction(() => !!(globalThis as Record<string, unknown>)['__debriefMockHealth']);
        await page.evaluate(() =>
          (globalThis as unknown as { __debriefMockHealth: { fail(...a: unknown[]): void } }).__debriefMockHealth.fail(
            'store',
            { message: "Debrief's log store stopped unexpectedly.", hint: 'Choose Try again.', detail: 'signal: 6' },
            'restart',
            'down',
          ),
        );
        await page.getByTestId('service-health-chip').click();
        await expect(page.getByTestId('service-health-details')).toBeVisible();
        await shot(page, 'health-down', scheme);
      });
    });

    test('a job running (the Jobs pane)', async ({ page }) => {
      await race(page, async () => {
        await openThemedApp(page, scheme, DESKTOP, '?mockAiStepMs=5&mockJobsHold=ai&mockJobsVisibleMs=0&mockJobsAttentionMs=600000');
        await page.locator('debrief-events-grid [role=row].row').first().click();
        await runCommand(page, 'AI: Explain the selected event');
        await runCommand(page, 'View: Jobs');
        const pane = page.locator('debrief-jobs-panel');
        await expect(pane.locator('li').filter({ hasText: 'Explain event' })).toContainText(/Running|Queued/);
        await shot(page, 'jobs-running', scheme);
      });
    });

    test('command palette open', async ({ page }) => {
      await race(page, async () => {
        await openThemedApp(page, scheme, DESKTOP);
        await page.keyboard.press('ControlOrMeta+k');
        await expect(page.getByRole('combobox', { name: 'Type a command' })).toBeFocused();
        await expect(page.locator('debrief-command-palette').getByRole('option').first()).toBeVisible();
        await shot(page, 'palette', scheme);
      });
    });

    test('panel menu with Pop out to window', async ({ page }) => {
      await race(page, async () => {
        await openThemedApp(page, scheme, DESKTOP);
        const group = page.locator('.dv-groupview').filter({ has: page.getByRole('tab', { name: 'Timeline', exact: true }) });
        await group.getByRole('button', { name: 'Panel menu', exact: true }).click();
        await expect(page.getByRole('menu', { name: 'Actions' }).getByRole('menuitem', { name: 'Pop out to window', exact: true })).toBeVisible();
        await shot(page, 'panel-menu', scheme);
      });
    });

    test('a pop-out window', async ({ page, context }) => {
      await openThemedApp(page, scheme, DESKTOP);
      const group = page.locator('.dv-groupview').filter({ has: page.getByRole('tab', { name: 'Timeline', exact: true }) });
      const opened = context.waitForEvent('page');
      await group.getByRole('button', { name: 'Panel menu', exact: true }).click();
      await page.getByRole('menu', { name: 'Actions' }).getByRole('menuitem', { name: 'Pop out to window', exact: true }).click();
      const popup = await opened;
      await popup.setViewportSize({ width: 900, height: 600 });
      await popup.emulateMedia({ colorScheme: scheme, reducedMotion: 'reduce' });
      await expect(popup.locator('debrief-popout-shell')).toBeVisible();
      await expect(popup.getByRole('heading', { level: 1, name: 'Timeline' })).toBeVisible();
      await expect(popup.locator('debrief-timeline-panel .lane').first()).toBeVisible();
      await settled(popup);
      await expect(popup).toHaveScreenshot(`popout-timeline-${scheme}.png`, { animations: 'disabled', caret: 'hide', maxDiffPixelRatio: 0.005 });
      await popup.close();
    });
  });
}
