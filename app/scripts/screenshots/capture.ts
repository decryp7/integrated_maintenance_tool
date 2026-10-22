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
const OUT = resolve(process.cwd(), '../docs/screenshots') // run from app/ (npm run screenshots);
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
  // IMT_SHOT_ONLY=home,light restricts a run to the named shots.
  const only = process.env['IMT_SHOT_ONLY'];
  if (only && !only.split(',').includes(name)) return;
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
    await page.screenshot({ path: join(OUT, `${name}.png`) });
    console.log(`captured ${name}`);
  } catch (e) {
    const why = e instanceof Error ? e.message.split('\n').slice(0, 4).join(' ') : String(e);
    skipped.push(`${name}: ${why}`);
    console.warn(`SKIPPED ${name}: ${why}`);
  } finally {
    await restore?.().catch(() => undefined);
  }
}

/** Brings a panel to the front: clicks its tab if it is open, otherwise toggles it from the palette. */
async function showPanel(page: Page, tab: string, toggleCommand: string): Promise<void> {
  const t = page.getByRole('tab', { name: tab, exact: true });
  if (!(await t.count())) await need(page, toggleCommand);
  await page.getByRole('tab', { name: tab, exact: true }).first().click();
}

/**
 * Selects the first row with this text that is on screen (the grid renders a few rows beyond its viewport, which a
 * click cannot reach); the first row when none is.
 */
async function clickRow(page: Page, text: string): Promise<void> {
  const grid = (await page.locator('imt-events-grid').boundingBox())!;
  const rows = page.locator('imt-events-grid [role=row].row');
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
  await page.waitForTimeout(500);
}

test('capture README screenshots', async ({ page }) => {
  mkdirSync(OUT, { recursive: true });

  await fresh(page);

  await shot(page, 'welcome', async () => {
    await need(page, 'Close workspace');
    await expect(page.locator('imt-welcome')).toBeVisible();
  });
  await fresh(page);
  await growTimeline(page);

  await shot(page, 'workspace', async () => {
    // The default layout keeps the inspector auto-hidden; the README shot shows it docked with an event selected.
    await clickRow(page, 'HTTP POST');
    await ensureInspector(page);
    await expect(page.locator('imt-timeline-panel .lane').first()).toBeVisible();
  });

  await shot(page, 'format-wizard', async () => {
    await page.getByRole('button', { name: /Set up format for/ }).first().click();
    await page.waitForTimeout(1000);
  });
  await page.keyboard.press('Escape');

  await shot(page, 'timeline-zoom', async () => {
    const zoomIn = page.getByRole('button', { name: 'Zoom in' });
    for (let i = 0; i < 3; i++) await zoomIn.click();
    await page.waitForTimeout(600);
  });
  await page.getByRole('button', { name: 'Fit', exact: true }).click().catch(() => undefined);

  await shot(page, 'inspector', async () => {
    await clickRow(page, 'HTTP POST');
    await expect(page.locator('imt-inspector')).toContainText(/Properties|RequestPath/i);
  });

  await shot(page, 'query', async () => {
    await need(page, 'Query: focus query bar');
    const input = page.getByRole('textbox', { name: 'Query' }).or(page.getByRole('combobox', { name: 'Query' })).first();
    await input.fill('level >= warning | stats count() as n by SourceContext | sort n desc | head 10');
    await page.keyboard.press('ControlOrMeta+Enter');
    await page.waitForTimeout(800);
  });

  await shot(page, 'query-results', async () => {
    await need(page, 'Query: show results');
    await page.waitForTimeout(800);
  }, 'Query results');

  await shot(page, 'metrics', async () => {
    await showPanel(page, 'Metrics', 'Toggle Metrics panel');
    await page.waitForTimeout(1200);
  }, 'Metrics');

  for (const [name, show, tab] of [
    ['patterns', 'Show patterns', 'Patterns'],
    ['exceptions', 'Show exceptions', 'Exceptions'],
    ['anomalies', 'Show anomalies', 'Anomalies'],
    ['compare', 'Show compare', 'Compare'],
  ] as const) {
    await shot(
      page,
      name,
      async () => {
        await need(page, show);
        if (name === 'compare') await page.getByRole('button', { name: 'Current range vs previous' }).click();
        await page.waitForTimeout(1500);
      },
      tab,
    );
  }

  await shot(page, 'moment', async () => {
    await showPanel(page, 'Events', 'Toggle Events panel');
    await page.locator('imt-events-grid [role=row].row').nth(8).click();
    await need(page, 'Analyse this moment');
    await page.waitForTimeout(1500);
  }, 'Moment');

  await shot(page, 'markers', async () => {
    await need(page, 'Manage markers');
    await page.waitForTimeout(800);
  }, 'Markers');

  await shot(page, 'profile', async () => {
    await showPanel(page, 'Profile', 'Toggle Profile panel');
    await page.waitForTimeout(800);
  }, 'Profile');


  await shot(page, 'perfmon', async () => {
    await showPanel(page, 'Performance Monitor', 'Toggle Performance Monitor panel');
    await page.waitForTimeout(800);
  }, 'Performance Monitor');

  await shot(page, 'process-monitor', async () => {
    await need(page, 'Toggle Process Monitor panel');
    await page.waitForTimeout(800);
  }, 'Process Monitor');

  await shot(page, 'registry', async () => {
    await showPanel(page, 'Registry', 'Toggle Registry panel');
    await page.waitForTimeout(800);
  }, 'Registry');

  await fresh(page);
  await shot(page, 'floating-panel', async () => {
    await page.getByRole('button', { name: 'Float panel' }).first().click();
    await page.waitForTimeout(600);
  });

  await fresh(page, 'light');
  await growTimeline(page);
  await shot(page, 'light-theme', async () => {
    await clickRow(page, 'HTTP POST');
  });

  await fresh(page, 'dark');
  await command(page, 'Theme: Console');
  await growTimeline(page);
  await shot(page, 'theme-console', async () => {
    await clickRow(page, 'HTTP POST');
  });

  // Home: the dashboard (tiles, history by level, findings, activity) in both themes.
  for (const theme of ['dark', 'light'] as const) {
    await fresh(page, theme);
    await growTimeline(page);
    await shot(page, theme === 'dark' ? 'home' : 'home-light', async () => {
      await command(page, 'Show Home');
      await expect(page.locator('imt-home-view [data-testid=home-history]')).toBeVisible();
      await page.waitForTimeout(800);
    }, 'Home');
  }

  if (skipped.length) console.warn(`\nSkipped ${skipped.length} shot(s):\n  ${skipped.join('\n  ')}`);
});
