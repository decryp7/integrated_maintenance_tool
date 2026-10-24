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

/**
 * A dialog shown in the page by its title (the dialog host labels `role=dialog` with its heading). In the desktop app
 * every registered dialog is its own OS window (ADR 0050), so this finds none of them: use `withDialogWindow`.
 */
export function inPageDialog(title: string) {
  return $(
    `//*[@role='dialog'][.//*[starts-with(@id,'debrief-dialog-title-')][normalize-space()=${lit(title)}]]`,
  );
}

/** The driver's window handles, empty once the app is gone. */
async function windowHandles(): Promise<string[]> {
  return browser.getWindowHandles().catch(() => [] as string[]);
}

/**
 * Finds the dialog window of a registered dialog (`defineDialog` id, e.g. `settings`, `about`, `close-prompt`) and
 * switches the driver to it. A dialog window loads `index.html#/dialog?id=<id>&host=...&label=...` (dialog_windows.rs
 * `dialog_url`), so the URL names it; the title bar and the page title do not. Returns its handle. Waits for the window,
 * and leaves the driver where it was when none appears.
 */
async function enterDialogWindow(id: string, timeout = 30_000): Promise<string> {
  const origin = await browser.getWindowHandle();
  let found = '';
  try {
    await browser.waitUntil(
      async () => {
        for (const handle of await windowHandles()) {
          if (handle === origin) continue;
          try {
            await browser.switchToWindow(handle);
            if ((await browser.getUrl()).includes(`#/dialog?id=${id}&`)) {
              found = handle;
              return true;
            }
          } catch {
            /* the window closed while it was scanned */
          }
        }
        await browser.switchToWindow(origin);
        return false;
      },
      { timeout, interval: 250, timeoutMsg: `no "${id}" dialog window appeared` },
    );
  } catch (e) {
    await browser.switchToWindow(origin).catch(() => undefined);
    throw e;
  }
  return found;
}

/**
 * Runs `body` with the driver on the `id` dialog window (it waits for the window to open), then switches back to the
 * window the call came from. The body may close the dialog (Escape, a button); `dismissDialogWindow` closes one and
 * waits for it to be gone.
 */
export async function withDialogWindow<T>(id: string, body: (handle: string) => Promise<T>, timeout?: number): Promise<T> {
  const origin = await browser.getWindowHandle();
  const handle = await enterDialogWindow(id, timeout);
  try {
    return await body(handle);
  } finally {
    if ((await windowHandles()).includes(origin)) await browser.switchToWindow(origin).catch(() => undefined);
  }
}

/**
 * Closes the `id` dialog window with Escape and waits until it is gone and the page's modal scrim with it. With
 * `optional`, a dialog that does not open within `timeout` is fine (returns false) instead of an error.
 */
export async function dismissDialogWindow(id: string, opts: { optional?: boolean; timeout?: number } = {}): Promise<boolean> {
  let handle: string;
  try {
    handle = await withDialogWindow(
      id,
      async (h) => {
        await browser.keys('Escape');
        return h;
      },
      opts.timeout,
    );
  } catch (e) {
    if (opts.optional) return false;
    throw e;
  }
  await browser.waitUntil(async () => !(await windowHandles()).includes(handle), {
    timeout: 15_000,
    timeoutMsg: `the "${id}" dialog window did not close`,
  });
  await $('[data-testid=modal-scrim]').waitForExist({ reverse: true, timeout: 15_000 });
  return true;
}

/** A button by its aria-label or its text, optionally inside a scope's XPath. */
export function button(name: string, scope = '') {
  return $(`${scope}//button[@aria-label=${lit(name)} or normalize-space()=${lit(name)}]`);
}

/** A dock tab by its label. */
export function tab(name: string) {
  return $(
    `//*[@role='tab'][.//*[contains(concat(' ',normalize-space(@class),' '),' debrief-tab-label ')][normalize-space()=${lit(name)}]]`,
  );
}

/** Waits for the shell: the in-app menu bar is there and the start-up work is done. */
export async function waitForShell(): Promise<void> {
  await $('[role=menubar][aria-label="Application menu"]').waitForDisplayed({ timeout: 60_000 });
  // The lifecycle extras are mounted into the title bar after start-up (the crash check has run by then).
  await $('debrief-title-extras').waitForExist({ timeout: 30_000 });
  // A build that includes the AI worker offers its set-up once per profile, as a modal dialog window; the main window
  // shows its modal scrim until it is closed, so close it before any click. Later starts of the same run do not offer it.
  await dismissDialogWindow('ai-setup', { optional: true, timeout: 5_000 });
}

/** Waits until the workspace shows events: grid rows, an event count and at least one timeline lane. */
export async function waitForEvents(): Promise<void> {
  await $('debrief-events-grid [role=row].row').waitForDisplayed({ timeout: 60_000 });
  await expect($('debrief-status-bar')).toHaveText(/\d[\d,]* events/, { wait: 30_000 });
  await $('debrief-timeline-panel .lane').waitForDisplayed({ timeout: 30_000 });
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
  const total = $('debrief-events-grid .bar .total');
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
 * Quits Debrief the way a user does: File ▸ Close window on the last window, then Quit in the close prompt (with
 * "Remember my choice" cleared so the user's setting is not changed). Resolves once the process is gone and
 * its running marker removed: a clean exit.
 */
/** Runs a click that may end the app: WebKitWebDriver reports "Session terminated without a reply" when the app
 *  exits before it answers the click (msedgedriver answers first). Whether Debrief really quit is checked afterwards. */
async function mayEndApp(click: () => Promise<unknown>): Promise<void> {
  try {
    await click();
  } catch (e) {
    if (!/session terminated|invalid session id|browser has closed|disconnected/i.test(String(e))) throw e;
  }
}

/** Leaves the driver on Debrief's only window: a spec may have closed the window the driver was on, and an extra window
 *  (e.g. the activity log) keeps Debrief running after its main window's "Close window" ▸ Quit. Extra windows are closed
 *  through Debrief's own menu, never the driver's closeWindow (WebKitWebDriver closed the wrong window). */
async function oneWindowLeft(): Promise<void> {
  let handles = await browser.getWindowHandles().catch(() => [] as string[]);
  const current = await browser.getWindowHandle().catch(() => '');
  if (handles.length && !handles.includes(current)) await browser.switchToWindow(handles[0]!);
  for (let guard = 0; handles.length > 1 && guard < 5; guard++) {
    const keep = await browser.getWindowHandle();
    await browser.switchToWindow(handles.find((h) => h !== keep)!);
    await mayEndApp(() => chooseMenu('File', 'Close window'));
    const before = handles.length;
    await browser
      .waitUntil(async () => (await browser.getWindowHandles()).length < before, { timeout: 10_000 })
      .catch(() => undefined);
    handles = await browser.getWindowHandles();
    if (handles.length) await browser.switchToWindow(handles.includes(keep) ? keep : handles[0]!);
  }
}

export async function quitApp(): Promise<void> {
  await oneWindowLeft();
  await mayEndApp(() => chooseMenu('File', 'Close window'));
  // The close prompt is a dialog window; a remembered "quit" choice closes without asking.
  const asked = await enterDialogWindow('close-prompt', 10_000).then(
    () => true,
    () => false,
  );
  if (asked) {
    const remember = $('.remember input[type=checkbox]');
    await remember.waitForDisplayed();
    if (await remember.isSelected()) await remember.click();
    await mayEndApp(() => $(`//button[normalize-space()='Quit']`).click());
  }
  await browser.waitUntil(async () => !(await isRunning()), {
    timeout: 30_000,
    timeoutMsg: 'Debrief did not exit after Quit',
  });
  await browser.waitUntil(async () => !existsSync(runningMarker()), {
    timeout: 15_000,
    timeoutMsg: `Debrief exited but left ${runningMarker()} behind (an unclean shutdown)`,
  });
}
