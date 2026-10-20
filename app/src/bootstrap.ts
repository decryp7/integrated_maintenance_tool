import { LOCALE_ID } from '@angular/core';
import { bootstrapApplication } from '@angular/platform-browser';
import { appConfig } from './app/app.config';
import { activeLocale } from './app/i18n/locale';
import { parsePopoutRoute } from './app/popout/popout-route';
import { parseAiBrowsingRoute } from './app/browser/ai-browsing-route';

// Loaded by main.ts once the translations are in place: everything imported from here may use `$localize`.
// A pop-out panel window (`#/popout?...`) boots the small pop-out shell; its code and the full app shell's are
// separate chunks, so a pop-out never loads the main window's chrome.
// The "AI is browsing" banner over a browser window (`#/ai-browsing?...`) boots the banner alone, with only the
// backends and logging.
const popout = parsePopoutRoute(location.hash) !== null;
const banner = !popout && parseAiBrowsingRoute(location.hash) !== null;
const locale = { provide: LOCALE_ID, useValue: activeLocale() };
if (popout) document.querySelector('imt-root')?.replaceWith(document.createElement('imt-popout-shell'));
if (banner) document.querySelector('imt-root')?.replaceWith(document.createElement('imt-ai-browsing-banner'));
const config = { ...appConfig, providers: [...appConfig.providers, locale] };

const started = banner
  ? import('./app/browser/ai-browsing-banner').then((m) => bootstrapApplication(m.AiBrowsingBanner, { providers: [...m.aiBrowsingProviders(), locale] }))
  : (popout ? import('./app/popout/popout-shell').then((m) => m.PopoutShell) : import('./app/app').then((m) => m.App)).then((component) =>
      bootstrapApplication(component, config),
    );
started.catch((err) => console.error(err));
