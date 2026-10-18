import { $, $$, browser, expect } from '@wdio/globals';
import { existsSync } from 'node:fs';
import { runningMarker } from './paths';

/**
 * UI helpers for the real-app smoke tests. They locate things the way the Playwright suite does (roles and
 * accessible names, plus the few component selectors it uses) and always wait for a condition, never a sleep.
 */

/** XPath string literal for any text (handles both quote kinds). */
function lit(text: string): string {
  if (!text.includes("'")) return `'${text}'`;
  if (!text.includes('"')) return `"${text}"`;
  return `concat('${text.split("'").join(`', "'", '`)}')`;
}

/** A dialog by its title (the dialog host labels `role=dialog` with its heading). */
export function dialog(title: string) {
  return $(
    `//*[@role='dialog'][.//*[starts-with(@id,'imt-dialog-title-')][normalize-space()=${lit(title)}]]`,
  );
}

/** A button by its aria-label or its text, optionally inside a scope's XPath. */
export function button(name: string, scope = '') {
  return $(`${scope}//button[@aria-label=${lit(name)} or normalize-space()=${lit(name)}]`);
}

/** A dock tab by its label. */
export function tab(name: string) {
  return $(
    `//*[@role='tab'][.//*[contains(concat(' ',normalize-space(@class),' '),' imt-tab-label ')][normalize-space()=${lit(name)}]]`,
  );
}

/** Waits for the shell: the in-app menu bar is there and the start-up work is done. */
export async function waitForShell(): Promise<void> {
  await $('[role=menubar][aria-label="Application menu"]').waitForDisplayed({ timeout: 60_000 });
  // The lifecycle extras are mounted into the title bar after start-up (the crash check has run by then).
  await $('imt-title-extras').waitForExist({ timeout: 30_000 });
}

/** Waits until the workspace shows events: grid rows, an event count and at least one timeline lane. */
export async function waitForEvents(): Promise<void> {
  await $('imt-events-grid [role=row].row').waitForDisplayed({ timeout: 60_000 });
  await expect($('imt-status-bar')).toHaveText(/\d[\d,]* events/, { wait: 30_000 });
  await $('imt-timeline-panel .lane').waitForDisplayed({ timeout: 30_000 });
}

/** Visible error surfaces with a message: toasts, banners and inline errors all use `role=alert` (empty live regions don't count). */
export async function visibleAlerts(): Promise<string[]> {
  const out: string[] = [];
  for (const a of await $$('[role=alert]')) {
    if (!(await a.isDisplayed())) continue;
    const text = (await a.getText()).trim();
    if (text) out.push(text);
  }
  return out;
}

/** Chooses a menu item by path in the in-app menu bar, e.g. `chooseMenu('View', 'Tools', 'Process Monitor')`. */
export async function chooseMenu(...path: string[]): Promise<void> {
  const [top, ...rest] = path;
  await $(`//*[@role='menubar']//button[normalize-space()=${lit(top!)}]`).click();
  let menu = $(`//*[@role='menu'][@aria-label=${lit(top!)}]`);
  await menu.waitForDisplayed();
  for (const [i, name] of rest.entries()) {
    const item = menu.$(
      `.//*[starts-with(@role,'menuitem')][.//*[contains(@class,'label')][normalize-space()=${lit(name)}]]`,
    );
    await item.waitForClickable();
    await item.click();
    if (i < rest.length - 1) {
      menu = $(`//*[@role='menu'][@aria-label=${lit(name)}]`);
      await menu.waitForDisplayed();
    }
  }
}

/** Parses a count like "40,123" shown in the UI. */
export function parseCount(text: string): number {
  return Number(text.replace(/[^\d]/g, ''));
}

/** The grid's total of matching events. */
export async function gridTotal(): Promise<number> {
  const total = $('imt-events-grid .bar .total');
  await expect(total).toHaveText(/^\d[\d,]*$/, { wait: 30_000 });
  return parseCount(await total.getText());
}

/** Whether the app's webview still answers (false once the process has exited). */
export async function isRunning(): Promise<boolean> {
  try {
    await browser.getTitle();
    return true;
  } catch {
    return false;
  }
}

/**
 * Quits IMT the way a user does: File ▸ Close window on the last window, then Quit in the close prompt (with
 * "Remember my choice" cleared so the user's setting is not changed). Resolves once the process is gone and
 * its running marker removed: a clean exit.
 */
export async function quitApp(): Promise<void> {
  await chooseMenu('File', 'Close window');
  const prompt = dialog('Close IMT?');
  // A remembered "quit" choice closes without asking.
  const asked = await prompt.waitForDisplayed({ timeout: 10_000 }).then(
    () => true,
    () => false,
  );
  if (asked) {
    const remember = prompt.$('.remember input[type=checkbox]');
    if (await remember.isSelected()) await remember.click();
    await prompt.$(`.//button[normalize-space()='Quit']`).click();
  }
  await browser.waitUntil(async () => !(await isRunning()), {
    timeout: 30_000,
    timeoutMsg: 'IMT did not exit after Quit',
  });
  await browser.waitUntil(async () => !existsSync(runningMarker()), {
    timeout: 15_000,
    timeoutMsg: `IMT exited but left ${runningMarker()} behind (an unclean shutdown)`,
  });
}
