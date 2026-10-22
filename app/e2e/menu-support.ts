import { Locator, Page } from '@playwright/test';

/** Debrief's in-app menu bar (Windows, Linux and the browser build). */
export function menubar(page: Page): Locator {
  return page.getByRole('menubar', { name: 'Application menu' });
}

/** An item (plain or checkable) of an open menu, by its exact label. */
export function menuItem(scope: Page | Locator, name: string | RegExp): Locator {
  const opts = { name, exact: typeof name === 'string' };
  return scope.getByRole('menuitem', opts).or(scope.getByRole('menuitemcheckbox', opts));
}

/** Opens a top-level menu (File, Edit, View, ...) and returns it. */
export async function openMenu(page: Page, name: string): Promise<Locator> {
  await menubar(page).getByRole('menuitem', { name, exact: true }).click();
  const menu = page.getByRole('menu', { name, exact: true });
  await menu.waitFor();
  return menu;
}

/** Opens the menus along a path (e.g. "View", "Analysis") and returns the last one, open. */
export async function openMenuPath(page: Page, ...path: string[]): Promise<Locator> {
  let menu = await openMenu(page, path[0]);
  for (const name of path.slice(1)) {
    await menu.getByRole('menuitem', { name, exact: true }).click();
    menu = page.getByRole('menu', { name, exact: true });
    await menu.waitFor();
  }
  return menu;
}

/** Chooses a menu item by path, e.g. `chooseMenu(page, 'View', 'Analysis', 'Anomalies')`. */
export async function chooseMenu(page: Page, ...path: string[]): Promise<void> {
  const menu = await openMenuPath(page, ...path.slice(0, -1));
  await menuItem(menu, path[path.length - 1]).click();
}

/** Opens (or focuses) a view by name with View ▸ Other windows…, which lists every view. */
export async function showView(page: Page, title: string): Promise<void> {
  await chooseMenu(page, 'View', 'Other windows…');
  const picker = page.getByRole('dialog', { name: 'Other windows' });
  await picker.getByRole('combobox', { name: 'Type a window name' }).fill(title);
  await picker.getByRole('option', { name: new RegExp(`${title.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}$`) }).first().click();
}

/** Opens the profile editor (not part of the default layout) from the activity bar. */
export async function showProfileEditor(page: Page): Promise<void> {
  await page.getByRole('button', { name: 'Profile editor' }).click();
  await page.locator('debrief-profile-editor').waitFor();
}
