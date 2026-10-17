import { Page, test } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { openApp } from '../../e2e/app';
import { chooseMenu, menubar, openMenu } from '../../e2e/menu-support';
import { layoutProblems, LayoutProblem } from './layout-check';

/**
 * Visual QA sweep (see playwright.config.ts): for each theme and size, screenshots the default layout, every top-level
 * menu, every view (maximised in its dock group) and the main dialogs, and writes the layout problems found in each
 * state to report.json next to the images.
 */
const OUT = resolve(process.cwd(), process.env['IMT_QA_OUT'] ?? '../target/visual-qa');
const SIZES = (process.env['IMT_QA_SIZES'] ?? '1600x1000,1024x700').split(',').map((s) => s.split('x').map(Number) as [number, number]);
const THEMES = (process.env['IMT_QA_THEMES'] ?? 'dark,light').split(',') as ('dark' | 'light')[];
const ONLY = process.env['IMT_QA_ONLY']?.toLowerCase();

const slug = (s: string) => s.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');

async function settle(page: Page, ms = 500): Promise<void> {
  await page.mouse.move(0, 0);
  await page.waitForTimeout(ms);
}

/** Titles of every view, as View ▸ Other windows… lists them ("Category: Title" → title). */
async function viewTitles(page: Page): Promise<string[]> {
  await chooseMenu(page, 'View', 'Other windows…');
  const picker = page.getByRole('dialog', { name: 'Other windows' });
  await picker.getByRole('option').first().waitFor();
  const titles = await picker.locator('[role=option] .title').allInnerTexts();
  await page.keyboard.press('Escape');
  return titles.map((t) => t.trim()).filter(Boolean);
}

for (const theme of THEMES) {
  for (const [width, height] of SIZES) {
    test(`sweep ${theme} ${width}x${height}`, async ({ page }) => {
      const dir = join(OUT, `${theme}-${width}x${height}`);
      mkdirSync(dir, { recursive: true });
      const report: Record<string, LayoutProblem[] | string> = {};
      await page.setViewportSize({ width, height });
      await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
      await openApp(page);

      const capture = async (name: string, prepare: () => Promise<void>, cleanup?: () => Promise<void>) => {
        if (ONLY && !name.includes(ONLY)) return;
        try {
          await prepare();
          await settle(page);
          await page.screenshot({ path: join(dir, `${name}.png`) });
          const problems = await layoutProblems(page);
          if (problems.length) report[name] = problems;
        } catch (e) {
          report[name] = `FAILED: ${e instanceof Error ? e.message.split('\n')[0] : String(e)}`;
        } finally {
          await cleanup?.().catch(() => undefined);
          await page.keyboard.press('Escape').catch(() => undefined);
        }
      };

      await capture('00-default-layout', async () => undefined);

      const menus = await menubar(page).getByRole('menuitem').allInnerTexts();
      for (const m of menus.map((t) => t.trim()).filter(Boolean)) {
        await capture(`01-menu-${slug(m)}`, async () => void (await openMenu(page, m)));
      }

      for (const title of await viewTitles(page)) {
        await capture(
          `10-view-${slug(title)}`,
          async () => {
            await chooseMenu(page, 'View', 'Other windows…');
            const picker = page.getByRole('dialog', { name: 'Other windows' });
            await picker.getByRole('combobox', { name: 'Type a window name' }).fill(title);
            await picker.getByRole('option').filter({ has: page.locator('.title', { hasText: new RegExp(`^${title.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}$`) }) }).first().click();
            await settle(page, 700);
            const tab = page.getByRole('tab', { name: title, exact: true }).first();
            const group = page.locator('.dv-groupview').filter({ has: tab }).first();
            const max = group.getByRole('button', { name: 'Maximize panel' });
            if (await max.count()) await max.click();
          },
          async () => {
            const restore = page.getByRole('button', { name: 'Restore panel size' }).first();
            if (await restore.count()) await restore.click();
          },
        );
      }

      await capture('20-palette', async () => void (await page.keyboard.press('ControlOrMeta+k')));
      writeFileSync(join(dir, 'report.json'), JSON.stringify(report, null, 2));
    });
  }
}
