import { LOCALE_ID } from '@angular/core';
import { bootstrapApplication } from '@angular/platform-browser';
import { appConfig } from './app/app.config';
import { activeLocale } from './app/i18n/locale';
import { parsePopoutRoute } from './app/popout/popout-route';
import { loadBrowserMocks } from './app/core/mock-registry';
import { isTauri } from './app/core/platform';

// Loaded by main.ts once the translations are in place: everything imported from here may use `$localize`.
// A pop-out panel window (`#/popout?...`) boots the small pop-out shell; its code and the full app shell's are
// separate chunks, so a pop-out never loads the main window's chrome.
const popout = parsePopoutRoute(location.hash) !== null;
if (popout) document.querySelector('imt-root')?.replaceWith(document.createElement('imt-popout-shell'));
const root = popout ? import('./app/popout/popout-shell').then((m) => m.PopoutShell) : import('./app/app').then((m) => m.App);
const config = { ...appConfig, providers: [...appConfig.providers, { provide: LOCALE_ID, useValue: activeLocale() }] };

// Outside Tauri (dev server, e2e) the backends are mocks: loaded first, in their own chunk the desktop app never loads.
const mocks = isTauri() ? Promise.resolve() : loadBrowserMocks();
Promise.all([root, mocks])
  .then(([component]) => bootstrapApplication(component, config))
  .catch((err) => console.error(err));
