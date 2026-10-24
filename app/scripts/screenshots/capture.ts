import { expect, Page, test } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { ensureInspector, openApp } from '../../e2e/app';

/**
 * Captures the README screenshots from the browser build (mock backend) into docs/screenshots.
 * Each shot is independent and resilient: if a view is missing or fails to render, the shot is
 * skipped with a warning and the others still run. Navigation uses roles, labels and the
 * command palette (command titles) instead of CSS structure.
 */
// DEBRIEF_SHOT_OUT redirects a trial run elsewhere; the default is the committed set. Run from app/ (npm run screenshots).
const OUT = process.env['DEBRIEF_SHOT_OUT'] ? resolve(process.env['DEBRIEF_SHOT_OUT']) : resolve(process.cwd(), '../docs/screenshots');
const THEMES = (process.env['DEBRIEF_SHOT_THEMES'] ?? 'dark,light').split(',') as ('dark' | 'light')[];
let suffix = '';
const skipped: string[] = [];

/** Runs a palette command by (part of) its title. Returns false if no such command exists. */
async function command(page: Page, title: string, retry = true): Promise<boolean> {
  const box = page.getByRole('combobox', { name: 'Type a command' });
  for (let i = 0; i < 3 && !(await box.isVisible()); i++) {
    await page.keyboard.press('ControlOrMeta+k');
    await page.waitForTimeout(250);
  }
  await box.fill(title);
  const match = page.getByRole('option').filter({ hasText: title }).first();
  try {
    await match.waitFor({ state: 'visible', timeout: 2500 });
  } catch {
    await page.keyboard.press('Escape');
    return retry ? command(page, title, false) : false;
  }
  await match.click();
  await page.waitForTimeout(400);
  return true;
}

async function need(page: Page, title: string): Promise<void> {
  if (!(await command(page, title))) throw new Error(`command "${title}" not found`);
}

async function settle(page: Page, ms = 600): Promise<void> {
  await page.mouse.move(0, 0);
  await page.waitForTimeout(ms);
}

/**
 * Captures one shot; a failure skips it with a warning instead of failing the run.
 * With `maximize`, the dock group holding that tab fills the window for the capture.
 */
async function shot(page: Page, name: string, prepare: () => Promise<void>, maximize?: string): Promise<void> {
  // DEBRIEF_SHOT_ONLY=home,light restricts a run to the named shots.
  const only = process.env['DEBRIEF_SHOT_ONLY'];
  if (only && !only.split(',').includes(name)) return;
  const file = `${name}${suffix}`;
  let restore: (() => Promise<void>) | undefined;
  try {
    await prepare();
    if (maximize) {
      const group = page.locator('.dv-groupview').filter({ has: page.getByRole('tab', { name: maximize, exact: true }) }).first();
      const button = group.getByRole('button', { name: 'Maximize panel' });
      await button.click();
      restore = async () => void (await page.getByRole('button', { name: 'Restore panel size' }).first().click());
    }
    await settle(page);
    await page.screenshot({ path: join(OUT, `${file}.png`) });
    console.log(`captured ${file}`);
  } catch (e) {
    const why = e instanceof Error ? e.message.split('\n').slice(0, 4).join(' ') : String(e);
    skipped.push(`${file}: ${why}`);
    console.warn(`SKIPPED ${name}: ${why}`);
  } finally {
    await restore?.().catch(() => undefined);
  }
}

/**
 * Brings a panel to the front: clicks its tab if it is open, else its activity-bar button, else the first palette
 * command (by title) that exists.
 */
async function showPanel(page: Page, tab: string, ...commands: string[]): Promise<void> {
  const t = page.getByRole('tab', { name: tab, exact: true });
  if (!(await t.count())) {
    const button = page.getByRole('button', { name: tab, exact: true });
    if (await button.count()) await button.first().click();
    else for (const c of commands) if ((await page.getByRole('button', { name: c, exact: true }).count()) ? (await page.getByRole('button', { name: c, exact: true }).first().click(), true) : await command(page, c)) break;
  }
  await page.getByRole('tab', { name: tab, exact: true }).first().click();
}

/**
 * Selects the first row with this text that is on screen (the grid renders a few rows beyond its viewport, which a
 * click cannot reach); the first row when none is.
 */
async function clickRow(page: Page, text: string): Promise<void> {
  const grid = (await page.locator('debrief-events-grid').boundingBox())!;
  const rows = page.locator('debrief-events-grid [role=row].row');
  for (let i = 0; i < (await rows.count()); i++) {
    const row = rows.nth(i);
    const b = await row.boundingBox();
    if (b && b.y >= grid.y && b.y + b.height <= grid.y + grid.height && (await row.innerText()).includes(text)) return row.click();
  }
  await rows.first().click();
}

/** Gives the timeline more height by dragging the horizontal sash between it and the events grid. */
async function growTimeline(page: Page, by = 150): Promise<void> {
  const sashes = page.locator('.dv-sash');
  for (let i = 0; i < (await sashes.count()); i++) {
    const b = await sashes.nth(i).boundingBox();
    if (b && b.width > b.height * 3 && b.y > 300 && b.y < 700) {
      await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2);
      await page.mouse.down();
      await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2 + by, { steps: 8 });
      await page.mouse.up();
      return;
    }
  }
  console.warn('timeline sash not found; leaving the layout as is');
}

async function fresh(page: Page, theme: 'dark' | 'light' = 'dark'): Promise<void> {
  await page.goto('/');
  await page.evaluate(() => localStorage.clear()); // forget the saved dock layout and theme of earlier shots
  await openApp(page);
  await command(page, `Colour scheme: ${theme}`);
  await command(page, 'Reset layout');
  // The demo layout docks the registry viewer beside the timeline: close it so shots show the timeline and grid at a useful size.
  await page.locator('.debrief-tab[data-panel-id="registry"] .debrief-tab-close').click({ timeout: 2000 }).catch(() => undefined);
  await page.waitForTimeout(500);
}

/** Every shot of one colour scheme; light shots get a `-light` suffix. */
async function captureAll(page: Page, theme: 'dark' | 'light'): Promise<void> {
  suffix = theme === 'light' ? '-light' : '';
  await fresh(page, theme);

  await shot(page, 'welcome', async () => {
    await need(page, 'Close workspace');
    await expect(page.locator('debrief-welcome')).toBeVisible();
  });
  await fresh(page, theme);
  await growTimeline(page, 100); // every lane whole (the demo has three instances), with the first grid rows below
  await shot(page, 'workspace', async () => {
    // The default layout keeps the inspector auto-hidden; the README shot shows it docked with an event selected.
    await clickRow(page, 'HTTP POST');
    await ensureInspector(page).catch(() => undefined); // an inspector that stays auto-hidden is fine for the shot
    await expect(page.locator('debrief-timeline-panel .lane').first()).toBeVisible();
  });

  await shot(page, 'inspector', async () => {
    await clickRow(page, 'HTTP POST');
    await ensureInspector(page).catch(() => undefined);
    await expect(page.locator('debrief-inspector')).toBeVisible();
  });

  await shot(page, 'format-wizard', async () => {
    await page.getByRole('button', { name: /Set up format for/ }).first().click();
    await page.waitForTimeout(1000);
  });
  await page.keyboard.press('Escape');

  await fresh(page, theme); // the same Explorer in every shot (the format wizard leaves a file selected)
  // Dialogs are OS windows on the desktop (ADR 0050); the browser build shows the same dialogs in the page: a modeless
  // tool beside the work, and a modal question with the page dimmed behind it.
  await shot(page, 'dialog-modeless', async () => {
    await need(page, 'About Debrief');
    await expect(page.getByRole('dialog', { name: 'About Debrief' })).toHaveAttribute('data-modality', 'modeless');
  });
  await page.keyboard.press('Escape');
  await shot(page, 'dialog-modal', async () => {
    await need(page, 'Paste as log');
    await expect(page.getByRole('dialog', { name: 'Paste as log' })).toHaveAttribute('data-modality', 'modal');
    // Invented lines, never a real log.
    await page.getByTestId('paste-text').fill(
      [
        '2026-10-14T10:41:07.120Z INF checkout order=A-1042 total=59.90 ms=212',
        '2026-10-14T10:41:07.480Z WRN checkout retry order=A-1042 attempt=2 reason=timeout',
        '2026-10-14T10:41:08.015Z ERR payments declined order=A-1042 code=insufficient_funds',
        '2026-10-14T10:41:08.300Z INF checkout order=A-1043 total=12.00 ms=187',
      ].join('\n'),
    );
    await page.getByTestId('paste-text').blur();
  });
  await page.keyboard.press('Escape');

  await shot(page, 'dialog-redacted-copy', async () => {
    await need(page, 'Share redacted copy');
    await expect(page.getByTestId('rc-scanned')).toContainText('Scanned');
  });
  await page.keyboard.press('Escape');
  await shot(page, 'dialog-export', async () => {
    await need(page, 'Export events');
    await expect(page.getByRole('dialog', { name: 'Export events' })).toBeVisible();
  });
  await page.keyboard.press('Escape');

  await fresh(page, theme); // both themes start from the same layout, with no event selected (so no inspector)
  await shot(page, 'timeline-zoom', async () => {
    const zoomIn = page.getByRole('button', { name: 'Zoom in' });
    for (let i = 0; i < 3; i++) await zoomIn.click();
    await page.waitForTimeout(600);
  });
  await page.getByRole('button', { name: 'Fit', exact: true }).click().catch(() => undefined);

  // The query shots: two panes (events and results) so every row is whole; the query bar shows what produced the results.
  await fresh(page, theme);
  await page.locator('.debrief-tab[data-panel-id="timeline"] .debrief-tab-close').click({ timeout: 2000 }).catch(() => undefined);
  // The Explorer closed (Ctrl+B): the search field gets the width to show the whole one-line query.
  await page.keyboard.press('ControlOrMeta+b');
  await shot(page, 'query', async () => {
    await need(page, 'Query: focus the search field');
    // One line that fits the bar: unfocused, the field shows the whole query that produced the rows (and covers nothing).
    await page.getByTestId('query-field').fill('level >= debug | stats count() by SourceContext, level');
    await page.keyboard.press('ControlOrMeta+Enter');
    await page.getByTestId('query-field').blur();
    await page.waitForTimeout(800);
  });

  await shot(page, 'query-results', async () => {
    await need(page, 'Query: focus the search field');
    await page.getByTestId('query-field').fill('level >= debug | stats count() by SourceContext, level');
    await page.getByTestId('query-field').press('ControlOrMeta+Enter');
    await page.waitForTimeout(1200);
    await need(page, 'Query: show results');
    await expect(page.getByText('Results of the previous query')).toHaveCount(0);
    await page.waitForTimeout(800);
  }, 'Query results');

  await page.keyboard.press('ControlOrMeta+b'); // the Explorer back for the later shots
  // The query shots leave their text in the search field: clear it so later shots show an empty field.
  await need(page, 'Query: focus the search field').catch(() => undefined);
  await page.getByTestId('query-field').fill('').catch(() => undefined);
  await page.keyboard.press('Enter');

  await fresh(page, theme); // a clean tab strip: only this view
  await shot(page, 'metrics', async () => {
    await showPanel(page, 'Metrics', 'Show Metrics');
    await page.waitForTimeout(1200);
  }, 'Metrics');

  for (const [name, show, tab] of [
    ['patterns', 'Show patterns', 'Patterns'],
    ['exceptions', 'Show exceptions', 'Exceptions'],
    ['anomalies', 'Show anomalies', 'Anomalies'],
    ['compare', 'Compare time ranges', 'Compare time ranges'],
  ] as const) {
    await fresh(page, theme);
    await shot(
      page,
      name,
      async () => {
        await need(page, show);
        if (name === 'exceptions') {
          await page.waitForTimeout(1200);
          // Expand the groups (the first click re-renders the list, so re-query each time).
          for (let i = 0; i < 4; i++) await page.locator('debrief-exceptions-view button.head[aria-expanded=false]').first().click().catch(() => undefined);
        }
        if (name === 'compare') await page.getByRole('button', { name: 'Before / after selected time' }).click();
        await page.waitForTimeout(1500);
      },
      tab,
    );
  }

  await fresh(page, theme); // a clean tab strip: only this view
  await shot(page, 'moment', async () => {
    // A sub-range after the quiet start, so the metrics have an earlier window to compare with (the whole range has none).
    const plot = page.locator('debrief-timeline-panel .plot').first();
    const b = (await plot.boundingBox())!;
    await page.mouse.move(b.x + b.width * 0.45, b.y + 40);
    await page.mouse.down();
    await page.mouse.move(b.x + b.width * 0.7, b.y + 40, { steps: 8 });
    await page.mouse.up();
    await need(page, 'Moment: selected range');
    await page.waitForTimeout(1500);
  }, 'Moment');

  await fresh(page, theme); // a clean tab strip: only this view
  await shot(page, 'markers', async () => {
    await need(page, 'Manage markers');
    await page.waitForTimeout(800);
  }, 'Markers');

  await fresh(page, theme); // a clean tab strip: only this view
  await shot(page, 'profile', async () => {
    await showPanel(page, 'Profile', 'Profile editor', 'Show Profile editor');
    await page.locator('debrief-profile-form .nav-item').filter({ hasText: 'Extractors' }).click();
    await page.waitForTimeout(800);
  }, 'Profile');


  await fresh(page, theme); // a clean tab strip: only this view
  await shot(page, 'perfmon', async () => {
    await showPanel(page, 'Performance Monitor', 'Open Performance Monitor');
    await page.waitForTimeout(800);
  }, 'Performance Monitor');

  await fresh(page, theme); // a clean tab strip: only this view
  await shot(page, 'process-monitor', async () => {
    await showPanel(page, 'Process Monitor', 'Open Process Monitor');
    // A running monitor (mock) with its live charts, as the intro describes.
    const panel = page.locator('debrief-monitor-view');
    await panel.getByLabel('Target value 1').fill('OrderService*');
    await panel.getByRole('button', { name: 'Start' }).click();
    await expect(page.getByTestId('monitor-live')).toBeVisible();
    await page.waitForTimeout(2500);
    // Opening the samples with live tail brings the timeline to the front of this group: back to the monitor.
    await page.getByRole('tab', { name: 'Process Monitor', exact: true }).click();
    await expect(page.getByTestId('monitor-live')).toBeVisible();
  }, 'Process Monitor');

  await fresh(page, theme); // a clean tab strip: only this view
  await shot(page, 'registry', async () => {
    await showPanel(page, 'Registry', 'Show registry viewer');
    await page.waitForTimeout(800);
  }, 'Registry');

  await fresh(page, theme);
  await shot(page, 'floating-panel', async () => {
    await need(page, 'Float panel');
    await page.waitForTimeout(600);
  });


  // ---- views added for 1.0: outliers, pattern values, time-range import, watched folders, lenses, plugins, jobs ----
  await fresh(page, theme);
  await shot(page, 'outliers', async () => {
    await need(page, 'Show error outliers');
    await page.waitForTimeout(1500);
  }, 'Error outliers');

  await fresh(page, theme);
  await shot(page, 'pattern-values', async () => {
    await need(page, 'Show patterns');
    await page.getByRole('button', { name: /^Show the values behind the placeholders/ }).first().click();
    await page.waitForTimeout(1200);
  }, 'Patterns');

  await fresh(page, theme);
  await shot(page, 'time-window', async () => {
    await page.getByTestId('open-range').click();
    const dialog = page.getByRole('dialog', { name: 'Open a time range' });
    await expect(dialog).toBeVisible();
    await dialog.getByTestId('range-from').fill('2026-10-14 10:30');
    await dialog.getByTestId('range-to').fill('2026-10-14 11:00');
    await dialog.getByText('Only events inside this range').click(); // take the focus ring off the date field
    await page.waitForTimeout(500);
  });
  await page.keyboard.press('Escape');

  await fresh(page, theme);
  await shot(page, 'watched-folders', async () => {
    await need(page, 'Show watched folders');
    // The browser build's folder picker answers with ?mockOpen=<path>: pick three invented folders in turn.
    for (const folder of ['/demo/extra-folder', '/demo/exports/nightly', '/demo/support-cases']) {
      await page.evaluate((f) => history.replaceState(null, '', `/?mockOpen=${encodeURIComponent(f)}`), folder);
      await page.getByTestId('watch-add').click();
      await page.waitForTimeout(500);
    }
    await page.waitForTimeout(800);
  }, 'Watched folders');

  await fresh(page, theme);
  await shot(page, 'lenses', async () => {
    const pick = async (item: string | RegExp) => {
      await page.getByTestId('lens-button').click();
      await page.getByRole('menuitem', { name: item, exact: typeof item === 'string' }).click();
    };
    await pick('Warnings and worse');
    await page.getByRole('button', { name: '+ Add filter' }).click();
    await page.getByLabel('Field', { exact: true }).fill('MachineName');
    await page.getByLabel('Value', { exact: true }).fill('WEB01');
    await page.getByRole('button', { name: 'Add', exact: true }).click();
    await pick('Filters → set');
    await page.getByTestId('lens-button').click(); // leave the menu open
    await page.waitForTimeout(500);
  });
  await page.keyboard.press('Escape');

  await fresh(page, theme);
  await shot(page, 'plugins', async () => {
    await need(page, 'Open Plugins');
    await page.getByRole('button', { name: /^Pin/ }).first().click(); // dock the sheet so it can fill the window
    await page.waitForTimeout(800);
  }, 'Plugins');

  // Jobs: a slow AI answer and a slow export run side by side, a root-cause request waits behind them.
  await page.goto('/');
  await page.evaluate(() => localStorage.clear()); // forget the layout the Plugins shot saved
  await page.goto('/?mockAiStepMs=60000&mockExportMs=60000&mockJobsVisibleMs=0&mockJobsAttentionMs=600000');
  await expect(page.locator('debrief-status-bar')).toContainText('events', { timeout: 20_000 });
  await command(page, `Colour scheme: ${theme}`);
  await page.locator('.debrief-tab[data-panel-id="registry"] .debrief-tab-close').click({ timeout: 2000 }).catch(() => undefined);
  await shot(page, 'jobs', async () => {
    await page.locator('debrief-events-grid [role=row].row').nth(8).click(); // a real event, not the very first row (event 0)
    await need(page, 'AI: Explain the selected event');
    await need(page, 'AI: Find likely root causes');
    await need(page, 'New incident report');
    const view = page.locator('debrief-report-view');
    await view.getByRole('textbox', { name: 'Report title' }).fill('Incident 2026-10-14 checkout failures');
    await view.getByRole('textbox', { name: 'Report title' }).press('Tab');
    await need(page, 'Export report');
    const dialog = page.getByRole('dialog', { name: 'Export report' });
    await dialog.getByRole('button', { name: 'Export…' }).click();
    await need(page, 'View: Jobs');
    await expect(page.locator('debrief-jobs-panel')).toBeVisible();
    // Toasts would cover the pane, and the report editor behind it is not the point of this shot.
    for (const x of await page.getByTestId('notification-dismiss').all()) await x.click().catch(() => undefined);
    await page.locator('.debrief-tab[data-panel-id="report"] .debrief-tab-close').click({ timeout: 2000 }).catch(() => undefined);
    // The timeline is not the point of this shot either: closing it leaves the events grid its rows above the Jobs panel.
    await page.locator('.debrief-tab[data-panel-id="timeline"] .debrief-tab-close').click({ timeout: 2000 }).catch(() => undefined);
    await need(page, 'View: Jobs').catch(() => undefined);
    await page.waitForTimeout(800);
  });

  // The first-start AI set-up window (modeless: the app stays usable while the model downloads).
  await page.goto('/?mockAi=setup&mockAiStepMs=60000');
  await expect(page.locator('debrief-status-bar')).toContainText('events', { timeout: 20_000 });
  await command(page, `Colour scheme: ${theme}`);
  await shot(page, 'dialog-ai-setup', async () => {
    await need(page, 'Set up AI assistant');
    await expect(page.getByTestId('ai-hardware')).toBeVisible();
    await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur()); // no focus ring on the close button
  });
  await page.keyboard.press('Escape');

  // Home: the dashboard (tiles, history by level, findings, activity).
  await fresh(page, theme);
  // Home fits the default window: its first screen is whole, Recent activity collapsed below it.
  await shot(page, 'home', async () => {
    await command(page, 'Show Home');
    await expect(page.locator('debrief-home-view [data-testid=home-history]')).toBeVisible();
    await page.waitForTimeout(800);
  }, 'Home');
}

test('capture README screenshots', async ({ page }) => {
  mkdirSync(OUT, { recursive: true });
  for (const theme of THEMES) await captureAll(page, theme);
  if (skipped.length) console.warn(`\nSkipped ${skipped.length} shot(s):\n  ${skipped.join('\n  ')}`);
});
