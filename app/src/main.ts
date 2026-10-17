import { bootstrapApplication } from '@angular/platform-browser';
import { appConfig } from './app/app.config';
import { parsePopoutRoute } from './app/popout/popout-route';

// A pop-out panel window (`#/popout?...`) boots the small pop-out shell; its code and the full app shell's are
// separate chunks, so a pop-out never loads the main window's chrome.
const popout = parsePopoutRoute(location.hash) !== null;
if (popout) document.querySelector('app-root')?.replaceWith(document.createElement('imt-popout-shell'));
const root = popout ? import('./app/popout/popout-shell').then((m) => m.PopoutShell) : import('./app/app').then((m) => m.App);

root
  .then((component) => bootstrapApplication(component, appConfig))
  .catch((err) => console.error(err));
