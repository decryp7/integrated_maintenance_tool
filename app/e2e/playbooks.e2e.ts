import { expect, test } from '@playwright/test';
import { runCommand } from './ai-support';
import { expectNoSeriousViolations } from './axe';
import { openAppWith } from './mock-url';

// Investigation playbooks (ADR 0039) against the mock backend: copy a template, start a run, run a check, mark a step done.

test('a template becomes a playbook, a run records a check and a manual step with a note', async ({ page }, testInfo) => {
  await openAppWith(page, '');
  await runCommand(page, 'View: Playbooks');
  const panel = page.locator('debrief-playbooks-panel');
  await expect(panel).toBeVisible();

  await panel.getByTestId('pb-template-service-crashed').click();
  await panel.getByTestId('pb-use-template').click();
  await expect(panel.getByTestId('pb-item-service-crashed')).toBeVisible();
  await panel.getByTestId('pb-start').click();
  await expect(panel.getByTestId('pb-progress')).toContainText('0 of 4 steps');

  await panel.getByTestId('pb-run-fatal').click();
  await expect(panel.getByTestId('pb-status-fatal')).toContainText(/Pass|Fail/);
  await expect(panel.getByTestId('pb-summary-fatal')).toContainText('Expected');
  await expect(panel.getByTestId('pb-progress')).toContainText('1 of 4 steps');

  await panel.getByTestId('pb-note-before').fill('Opened the first fatal event');
  await panel.getByTestId('pb-note-before').blur();
  await panel.getByTestId('pb-done-before').click();
  await expect(panel.getByTestId('pb-status-before')).toContainText('Pass');
  await expect(panel.getByTestId('pb-note-before')).toHaveValue('Opened the first fatal event');
  await expect(panel.getByTestId('pb-progress')).toContainText('2 of 4 steps');

  await expectNoSeriousViolations(page, testInfo, 'playbooks', 'debrief-playbooks-panel');
});
