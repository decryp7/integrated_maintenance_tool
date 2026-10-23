import { EnvironmentProviders, inject, makeEnvironmentProviders, Provider, provideAppInitializer, signal } from '@angular/core';
import { PANEL_WINDOWS } from '../dock/panel-windows';
import { SessionService } from '../lifecycle/session.service';
import { OpenRequests } from '../projects/open-requests';
import { parseDialogRoute } from '../dialog-windows/dialog-route';
import { parsePopoutRoute } from './popout-route';
import { PopoutService } from './popout.service';

/** A pop-out window never autosaves the session: the host's session is the one (and it would overwrite it). */
export const INERT_SESSION = { enabled: signal(false), lastSaved: signal<number | null>(null), start() {}, flush: async () => false, saveNow: async () => false };
/** A pop-out never takes files queued for its host. */
export const INERT_OPEN_REQUESTS = { start: async () => {}, stop() {}, drain: async () => {}, handle: async () => {} };

/** Providers of a pop-out window (the hash routes it; `main.ts` bootstraps the shell): host-only services made inert. */
export function popoutWindowProviders(): (Provider | EnvironmentProviders)[] {
  return [
    { provide: SessionService, useValue: INERT_SESSION },
    { provide: OpenRequests, useValue: INERT_OPEN_REQUESTS },
  ];
}

/**
 * Panels popped out into real OS windows (Tauri WebviewWindows; `window.open` popups in the browser build) that
 * share the host's UI state. One line in `app.config.ts`, after the other features (it overrides host-only
 * services in a pop-out window).
 */
export function providePopout(hash: string = typeof location === 'undefined' ? '' : location.hash): EnvironmentProviders {
  // A dialog window is a guest of its host like a pop-out: host-only services stay inert and it never hosts.
  const route = parsePopoutRoute(hash) ?? parseDialogRoute(hash);
  return makeEnvironmentProviders([
    { provide: PANEL_WINDOWS, useExisting: PopoutService },
    ...(route ? popoutWindowProviders() : [provideAppInitializer(() => void inject(PopoutService).startHost())]),
  ]);
}
