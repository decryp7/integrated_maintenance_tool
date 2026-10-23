import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';

async function selectRowWithMessage(page: Page, text: string): Promise<void> {
  await page.getByTestId('query-field').fill(text);
  const row = page.locator('debrief-events-grid [role=row].row').first();
  await expect(row).toContainText(text);
  await row.click();
  await expect(page.locator('debrief-inspector .message')).toContainText(text);
}

async function runCommand(page: Page, title: string): Promise<void> {
  await page.keyboard.press('ControlOrMeta+k');
  const input = page.getByRole('combobox', { name: 'Type a command' });
  await input.fill(title);
  await expect(page.locator('debrief-command-palette').getByRole('option').first()).toContainText(title);
  await page.keyboard.press('Enter');
  await expect(input).toHaveCount(0);
}

test.describe('timeline markers', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('highlight lines with a regex and a badge, then reorder, disable and delete', async ({ page }, testInfo) => {
    test.setTimeout(90_000);
    await selectRowWithMessage(page, 'Slow request');
    await page.locator('debrief-events-grid [role=row].row').first().click({ button: 'right' });
    await page.getByRole('menuitem', { name: 'Highlight lines…' }).click();

    const dialog = page.getByRole('dialog', { name: 'Highlight lines' });
    await expect(dialog).toBeVisible();
    await expect(dialog.getByRole('status')).toContainText(/marks\s*[\d,]+\s*events/);
    await expect(dialog.getByLabel('Badge text (1 to 3 characters)')).toHaveValue('1');

    // Edit the proposed regex: any slow request.
    // Without a selection the template is proposed; switch the condition to a regex.
    await dialog.getByRole('combobox', { name: 'Operator' }).selectOption('matches');
    await dialog.getByRole('textbox', { name: 'Field' }).fill('@message');
    await dialog.getByRole('textbox', { name: 'Value' }).fill('Slow request .* took \\d+ ms');
    await dialog.getByLabel('Label', { exact: true }).fill('Slow requests');
    await dialog.getByLabel(/^Description/).fill('Request slower than the SLA');
    await dialog.getByRole('button', { name: 'Explain' }).click();
    await expect(dialog.getByRole('region', { name: 'Regex explanation' })).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'highlight-dialog');
    await dialog.getByRole('button', { name: 'Add highlight' }).click();
    await expect(dialog).toHaveCount(0);

    // The glyph is on the timeline and the legend explains it.
    const legend = page.locator('debrief-timeline-panel .legend').getByRole('button', { name: 'Slow requests' });
    await expect(legend).toHaveAttribute('title', 'Request slower than the SLA');
    await expect(legend).toBeVisible();
    await expect(legend.locator('debrief-marker-glyph[data-glyph=badge] .badge-text')).toHaveAttribute('data-t', '1');
    await expect(page.locator('debrief-timeline-panel .marker debrief-marker-glyph[data-glyph=badge]').first()).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'timeline-badges');

    // Clicking the legend item filters to the kind.
    await legend.click();
    await expect(legend).toHaveAttribute('aria-pressed', 'true');
    await legend.click();
    await expect(legend).toHaveAttribute('aria-pressed', 'false');

    // Manage: reorder, disable, delete.
    await runCommand(page, 'Manage markers');
    const table = page.getByRole('table', { name: 'Marker rules' });
    await expect(table).toBeVisible();
    const kinds = () => table.locator('tbody tr').evaluateAll((rows) => rows.map((r) => r.getAttribute('data-kind')));
    await expect.poll(kinds).toEqual(['slow-requests', 'fatal', 'start', 'stop', 'exception', 'error', 'warning']);
    await expect(table.locator('tr[data-kind=slow-requests]')).toContainText('Request slower than the SLA');
    await expect(table.locator('tr[data-kind=slow-requests]')).toContainText('message matches regex');
    await expectNoSeriousViolations(page, testInfo, 'markers-manager');

    await table.getByRole('button', { name: 'Move down Slow requests' }).click();
    await expect.poll(kinds).toEqual(['fatal', 'slow-requests', 'start', 'stop', 'exception', 'error', 'warning']);

    await page.getByRole('tab', { name: 'Timeline' }).click();
    await expect(legend).toBeVisible();
    await page.getByRole('tab', { name: 'Markers' }).click();
    await table.getByRole('checkbox', { name: 'Disable Slow requests' }).click();
    await expect(table.getByRole('checkbox', { name: 'Enable Slow requests' })).toBeVisible();
    await expect(page.locator('debrief-timeline-panel .legend').getByRole('button', { name: 'Slow requests' })).toHaveCount(0);

    await table.getByRole('checkbox', { name: 'Enable Slow requests' }).click();
    await page.getByRole('tab', { name: 'Timeline' }).click();
    await expect(page.locator('debrief-timeline-panel .legend').getByRole('button', { name: 'Slow requests' })).toBeVisible();

    await page.getByRole('tab', { name: 'Markers' }).click();
    await table.getByRole('button', { name: 'Delete Slow requests' }).click();
    await table.getByRole('button', { name: 'Confirm delete' }).click();
    await expect.poll(kinds).toEqual(['fatal', 'start', 'stop', 'exception', 'error', 'warning']);
    await page.getByRole('tab', { name: 'Timeline' }).click();
    await expect(page.locator('debrief-timeline-panel .legend').getByRole('button', { name: 'Slow requests' })).toHaveCount(0);
  });

  test('imports custom svg artwork; the engine sanitises scripts away', async ({ page }, testInfo) => {
    await runCommand(page, 'Manage markers');
    await page.getByRole('button', { name: /New marker/ }).click();
    const dialog = page.getByRole('dialog', { name: 'New marker' });
    await dialog.getByRole('textbox', { name: 'Value' }).fill('Slow request');
    await dialog.getByLabel('Label', { exact: true }).fill('Acme slow');
    await dialog.getByRole('tab', { name: 'Custom' }).click();

    const chooser = page.waitForEvent('filechooser');
    await dialog.getByRole('button', { name: /Import SVG or PNG/ }).click();
    (await chooser).setFiles({
      name: 'acme.svg',
      mimeType: 'image/svg+xml',
      buffer: Buffer.from('<svg viewBox="0 0 24 24"><script>window.pwned=1</script><path d="M3 3L21 21M21 3L3 21" stroke="red" stroke-width="3" onclick="window.pwned=1"/></svg>'),
    });
    await expect(dialog.locator('.current debrief-marker-glyph svg path')).toHaveCount(1);
    await expect(dialog.locator('.current debrief-marker-glyph script')).toHaveCount(0);
    await expectNoSeriousViolations(page, testInfo, 'custom-icon');
    await dialog.getByRole('button', { name: 'Add marker' }).click();
    await expect(dialog).toHaveCount(0);

    await page.getByRole('tab', { name: 'Timeline' }).click();
    // The one-line legend keeps most marker kinds in its popover.
    await page.locator('debrief-timeline-panel').getByTestId('legend-more').click();
    const legend = page.locator('debrief-timeline-panel .legend').getByRole('button', { name: /Acme slow/ });
    await expect(legend.locator('debrief-marker-glyph[data-glyph=svg] svg path')).toHaveCount(1);
    expect(await page.evaluate(() => (window as unknown as { pwned?: number }).pwned)).toBeUndefined();
    await expect(page.locator('debrief-timeline-panel .legend script')).toHaveCount(0);
  });

  test('rejects oversized icons before saving', async ({ page }) => {
    await runCommand(page, 'Manage markers');
    await page.getByRole('button', { name: /New marker/ }).click();
    const dialog = page.getByRole('dialog', { name: 'New marker' });
    await dialog.getByRole('tab', { name: 'Custom' }).click();
    const chooser = page.waitForEvent('filechooser');
    await dialog.getByRole('button', { name: /Import SVG or PNG/ }).click();
    (await chooser).setFiles({ name: 'big.svg', mimeType: 'image/svg+xml', buffer: Buffer.from('<svg>' + ' '.repeat(20000) + '</svg>') });
    await expect(dialog.getByRole('alert')).toContainText('16 KB');
  });

  test('infer a pattern from a line and explain it', async ({ page }, testInfo) => {
    await selectRowWithMessage(page, 'Slow request');
    await runCommand(page, 'Infer pattern from selected lines');
    const dialog = page.getByRole('dialog', { name: 'Infer pattern from lines' });
    await expect(dialog.getByLabel('Pattern (editable)')).toHaveValue(/\(\?P<n1>/);
    await expect(dialog.getByRole('table', { name: 'Captured groups' })).toBeVisible();
    await dialog.getByRole('button', { name: 'Explain' }).click();
    await expect(dialog.getByRole('region', { name: 'Regex explanation' })).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'infer-dialog');
    await dialog.getByRole('button', { name: 'Highlight these lines' }).click();
    await expect(page.getByRole('dialog', { name: 'Highlight lines' })).toBeVisible();
  });

  test('highlight command offers the selected text as a regex', async ({ page }) => {
    await selectRowWithMessage(page, 'Slow request');
    await page.evaluate(() => {
      const node = document.querySelector('debrief-inspector .message')!.firstChild as Text;
      const i = node.data.indexOf('took');
      const r = document.createRange();
      r.setStart(node, i);
      r.setEnd(node, node.data.length);
      document.getSelection()!.removeAllRanges();
      document.getSelection()!.addRange(r);
    });
    await page.getByRole('button', { name: 'More actions…' }).click();
    await page.getByRole('menuitem', { name: 'Highlight lines…' }).click();
    const dialog = page.getByRole('dialog', { name: 'Highlight lines' });
    await expect(dialog.getByRole('textbox', { name: 'Value' })).toHaveValue(/took \\d\+/);
  });
});
