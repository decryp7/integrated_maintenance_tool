import { LOCALE_ID } from '@angular/core';
import { bootstrapApplication } from '@angular/platform-browser';
import { activeLocale } from './app/i18n/locale';
import { parseDialogRoute } from './app/dialog-windows/dialog-route';
import { parsePopoutRoute } from './app/popout/popout-route';
import { loadBrowserMocks } from './app/core/mock-registry';
import { isTauri } from './app/core/platform';

// Loaded by main.ts once the translations are in place: everything imported from here may use `$localize`.
// A pop-out panel window (`#/popout?...`) boots the small pop-out shell; its code and the full app shell's are
// separate chunks, so a pop-out never loads the main window's chrome.
const popout = parsePopoutRoute(location.hash) !== null;
const dialogRoute = parseDialogRoute(location.hash);
const dialog = dialogRoute !== null;
const locale = { provide: LOCALE_ID, useValue: activeLocale() };
if (popout) document.querySelector('debrief-root')?.replaceWith(document.createElement('debrief-popout-shell'));
if (dialog) document.querySelector('debrief-root')?.replaceWith(document.createElement('debrief-dialog-shell'));
// The main window and pop-outs boot the whole app's providers; a dialog window its own small set (ADR 0050), so the
// feature code of the full config is a chunk a dialog never loads.
const configOf = dialogRoute
  ? import('./app/dialog-windows/dialog-bootstrap').then((m) => m.dialogAppConfig(dialogRoute.id))
  : import('./app/app.config').then((m) => m.appConfig);

// Outside Tauri (dev server, e2e) the backends are mocks: loaded first, in their own chunk the desktop app never loads.
const mocks = isTauri() ? Promise.resolve() : loadBrowserMocks();
const started = Promise.all([
  popout
    ? import('./app/popout/popout-shell').then((m) => m.PopoutShell)
    : dialog
      ? import('./app/dialog-windows/dialog-shell').then((m) => m.DialogShell)
      : import('./app/app').then((m) => m.App),
  configOf,
  mocks,
]).then(([component, appConfig]) => bootstrapApplication(component, { ...appConfig, providers: [...appConfig.providers, locale] }));
started.catch((err) => console.error(err));
