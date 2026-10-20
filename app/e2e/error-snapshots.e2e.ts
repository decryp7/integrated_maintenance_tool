import { expect, Page, test } from '@playwright/test';
import { CAPTURE_TIMEOUT, openApp, openView } from './app';
import { openAppWith } from './mock-url';

// Error snapshots: the window is rendered to an image inside the webview (html-to-image), so this runs
// against the real capture code in every project. Under "prod-csp" the production build is served with
// the Tauri CSP, which proves the capture needs no blob:, remote or inline-script permission.

const toast = (page: Page) => page.locator('[data-testid=snapshot-toast]');

/** Raises an uncaught error the way a bug would: from a timer, outside Angular's zone of control. */
async function throwUncaught(page: Page, message: string): Promise<void> {
  await page.evaluate((m) => void setTimeout(() => { throw new Error(m); }, 0), message);
}

async function openSnapshotsView(page: Page): Promise<void> {
  await page.keyboard.press('ControlOrMeta+k');
  const input = page.getByRole('combobox', { name: 'Type a command' });
  await input.fill('Show error snapshots');
  await page.keyboard.press('Enter');
  await expect(page.locator('imt-error-snapshots-view')).toBeVisible();
}

/** Decodes the data URL of the first thumbnail and reports its size and how many distinct colours a region holds. */
async function analyse(page: Page, region?: { x: number; y: number; w: number; h: number }) {
  return page.evaluate(
    async ({ region, vw }) => {
      const src = document.querySelector<HTMLImageElement>('imt-error-snapshots-view .card img')!.src;
      const img = new Image();
      img.src = src;
      await img.decode();
      const scale = img.naturalWidth / vw;
      const r = region ?? { x: 0, y: 0, w: vw, h: img.naturalHeight / scale };
      const canvas = document.createElement('canvas');
      const [sx, sy, sw, sh] = [Math.round(r.x * scale), Math.round(r.y * scale), Math.max(1, Math.round(r.w * scale)), Math.max(1, Math.round(r.h * scale))];
      canvas.width = sw;
      canvas.height = sh;
      const ctx = canvas.getContext('2d')!;
      ctx.drawImage(img, sx, sy, sw, sh, 0, 0, sw, sh);
      const data = ctx.getImageData(0, 0, sw, sh).data;
      const colours = new Set<number>();
      for (let i = 0; i < data.length; i += 4) colours.add(((data[i] >> 4) << 8) | ((data[i + 1] >> 4) << 4) | (data[i + 2] >> 4));
      return { src: src.slice(0, 22), width: img.naturalWidth, height: img.naturalHeight, colours: colours.size };
    },
    { region, vw: page.viewportSize()!.width },
  );
}

test('an uncaught error saves a snapshot, shows the toast and the notice, and the view lists it', async ({ page }) => {
  // Capturing must not trip the CSP (relevant in prod-csp, harmless elsewhere).
  const violations: string[] = [];
  page.on('console', (m) => /Content Security Policy|Refused to/i.test(m.text()) && violations.push(m.text()));
  await page.addInitScript(() => {
    const w = window as unknown as { __csp: string[] };
    w.__csp = [];
    document.addEventListener('securitypolicyviolation', (e) => w.__csp.push(`${e.violatedDirective} ${e.blockedURI}`));
  });
  await openApp(page);
  await throwUncaught(page, 'e2e boom');

  await expect(toast(page)).toContainText('Error captured', { timeout: CAPTURE_TIMEOUT });
  await expect(page.locator('[data-testid=snapshot-notice]')).toContainText('IMT saved a screenshot of this error locally to help diagnose it. Nothing is sent anywhere.');
  await toast(page).getByRole('button', { name: 'View', exact: true }).click();

  const view = page.locator('imt-error-snapshots-view');
  await expect(view.locator('[data-testid=snapshot-card]')).toHaveCount(1);
  await expect(view).toContainText('unhandled-error');
  await expect(view).toContainText('e2e boom');
  await expect(view.locator('.card img')).toBeVisible();

  // A real picture of the app: right type, within the 1600 px limit, and not a blank page.
  const image = await analyse(page);
  expect(image.src).toMatch(/^data:image\/(png|jpeg);base64,/);
  expect(image.width).toBeGreaterThan(200);
  expect(image.width).toBeLessThanOrEqual(1600);
  expect(image.colours).toBeGreaterThan(8);

  // Large view with its context.
  await view.locator('button.thumb').click();
  const detail = view.getByRole('region', { name: 'Snapshot details' });
  await expect(detail).toContainText('e2e boom');
  await expect(detail.getByRole('img', { name: /Screenshot/ })).toBeVisible();

  // Export asks for a destination (the browser build uses a default name); a job writes the zip and its message
  // reports the result.
  await view.getByRole('button', { name: 'Export diagnostics…' }).click();
  await expect(view.getByRole('status')).toContainText('Exporting 1 snapshot');
  const exported = page.getByTestId('notice-toast').filter({ hasText: 'Finished: Export diagnostics to' });
  await expect(exported).toContainText('files zipped.', { timeout: 15_000 });

  await view.getByRole('button', { name: 'Delete', exact: true }).click();
  await expect(view.locator('[data-testid=snapshot-card]')).toHaveCount(0);

  const blocked = await page.evaluate(() => (window as unknown as { __csp: string[] }).__csp);
  expect([...violations, ...blocked]).toEqual([]);
});

test('a second automatic error within 10 seconds does not take another snapshot', async ({ page }) => {
  await openApp(page);
  await throwUncaught(page, 'first');
  await expect(toast(page)).toContainText('Error captured', { timeout: CAPTURE_TIMEOUT });
  // The app logs the error and decides about a snapshot in the same task: once it is logged, it was decided.
  const handled = page.waitForEvent('console', (m) => m.type() === 'error' && m.text().includes('second'));
  await throwUncaught(page, 'second');
  await handled;
  await openSnapshotsView(page);
  await expect(page.locator('imt-error-snapshots-view [data-testid=snapshot-card]')).toHaveCount(1);
});

test('Ctrl/Cmd+Shift+F12 captures a diagnostic screenshot that includes the charts', async ({ page }) => {
  await openApp(page);
  await openView(page, 'metrics');
  const chart = page.locator('imt-chart-card canvas').first();
  await expect(chart).toBeVisible();
  // ECharts draws after the canvas appears: wait until it shows more than a flat background.
  await expect
    .poll(() =>
      chart.evaluate((c: HTMLCanvasElement) => {
        const data = c.getContext('2d')?.getImageData(0, 0, c.width, c.height).data ?? [];
        const colours = new Set<number>();
        for (let i = 0; i < data.length; i += 16) colours.add(((data[i] >> 4) << 8) | ((data[i + 1] >> 4) << 4) | (data[i + 2] >> 4));
        return colours.size;
      }),
    )
    .toBeGreaterThan(3);
  const box = (await chart.boundingBox())!;

  await page.keyboard.press('ControlOrMeta+Shift+F12');
  await expect(toast(page)).toContainText('Error captured', { timeout: CAPTURE_TIMEOUT });
  await toast(page).getByRole('button', { name: 'View', exact: true }).click();
  const view = page.locator('imt-error-snapshots-view');
  await expect(view).toContainText('manual');

  // ECharts draws on a canvas; html-to-image must copy it. A missing canvas leaves one flat colour.
  const inChart = await analyse(page, { x: box.x, y: box.y, w: box.width, h: box.height });
  expect(inChart.colours).toBeGreaterThan(3);
});

test('the capture command is in the palette', async ({ page }) => {
  await openApp(page);
  await page.keyboard.press('ControlOrMeta+k');
  await page.getByRole('combobox', { name: 'Type a command' }).fill('Capture diagnostic');
  await expect(page.locator('imt-command-palette').getByRole('option').first()).toContainText('Capture diagnostic screenshot');
});

test('a panel that fails takes a snapshot of its error card', async ({ page }) => {
  // Rows without `markers` make the events grid throw while rendering.
  await openAppWith(page, '?mockCorrupt=markers', { grid: false });
  await expect(page.locator('.imt-panel-failed')).toBeVisible();
  await expect(toast(page)).toContainText('Error captured', { timeout: CAPTURE_TIMEOUT });
  await openSnapshotsView(page);
  const view = page.locator('imt-error-snapshots-view');
  await expect(view.locator('[data-testid=snapshot-card]')).toHaveCount(1);
  await expect(view).toContainText('panel-failed');
});

test('an unexpected backend failure takes a snapshot naming the command', async ({ page }) => {
  await openAppWith(page, '?mockFail=queryTimeline:1');
  await expect(toast(page)).toContainText('Error captured', { timeout: CAPTURE_TIMEOUT });
  await openSnapshotsView(page);
  const view = page.locator('imt-error-snapshots-view');
  await expect(view.locator('[data-testid=snapshot-card]')).toHaveCount(1);
  await expect(view).toContainText('backend-failed');
  await expect(view).toContainText('queryTimeline');
});
