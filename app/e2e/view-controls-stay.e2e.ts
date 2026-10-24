import { expect, Page, test } from '@playwright/test';
import { openAiApp } from './ai-support';
import { showProfileEditor } from './menu-support';

// ADR 0061: a view's controls do not move when its data arrives after it opened. A click that starts on a control
// just after the view opened must end on the same control: if the layout shifts between the press and the release,
// the click is lost (ai-entry-points: "Propose…" opened nothing about 1 run in 8 on WebKit). The mock holds the
// profile (`mockHold`) so the press always happens before it arrives, and the release always after.

const release = (page: Page) => page.evaluate(() => (globalThis as unknown as { __debriefMockRelease: () => void }).__debriefMockRelease());

test('the profile editor keeps Propose… in place while its profile loads, and a click across the load opens the dialog', async ({ page }) => {
  await openAiApp(page, 'mockHold=getProfile');
  await showProfileEditor(page);
  const editor = page.locator('debrief-profile-editor');
  const name = editor.locator('.bar .name');
  await expect(name).toHaveText('…');
  const propose = page.getByTestId('ai-profile-propose');
  await expect(propose).toBeEnabled();
  const before = await propose.boundingBox();
  if (!before) throw new Error('Propose… has no box');

  // Press on Propose… while the profile is still on its way, let it arrive, then release the button.
  await page.mouse.move(before.x + before.width / 2, before.y + before.height / 2);
  await page.mouse.down();
  await release(page);
  await expect(name).toHaveText('OrderService (Serilog) — demo');
  await expect(editor.getByText('Valid', { exact: true })).toBeVisible();
  expect(await propose.boundingBox()).toEqual(before);
  await page.mouse.up();

  await expect(page.getByRole('dialog', { name: 'Propose profile changes' })).toBeVisible();
});
