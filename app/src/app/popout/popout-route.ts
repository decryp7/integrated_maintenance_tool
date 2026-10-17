import { InjectionToken } from '@angular/core';

/**
 * What a pop-out window shows: the app loads with `#/popout?view=<id>&host=<label>&label=<label>`
 * (built by the shell in Rust, or by the browser adapter) and boots into the pop-out shell.
 */
export interface PopoutRoute {
  /** Id of the view the window shows. */
  view: string;
  /** Label of the window the panel belongs to. */
  host: string;
  /** This window's own label (`p-<host>-<n>`). */
  label: string;
}

const VIEW_ID = /^[a-z0-9][a-z0-9._-]{0,63}$/i;
const LABEL = /^[a-z0-9][a-z0-9-]{0,63}$/i;

/** Parses the window's location hash into a pop-out route; null for a normal app window or anything malformed. */
export function parsePopoutRoute(hash: string): PopoutRoute | null {
  const match = /^#\/popout\?(.*)$/.exec(hash ?? '');
  if (!match) return null;
  const params = new URLSearchParams(match[1]);
  const view = params.get('view') ?? '';
  const host = params.get('host') ?? '';
  const label = params.get('label') ?? '';
  if (!VIEW_ID.test(view) || !LABEL.test(host) || !LABEL.test(label) || label === host) return null;
  return { view, host, label };
}

/** The hash that routes a window to the pop-out shell. */
export function popoutHash(route: PopoutRoute): string {
  const params = new URLSearchParams({ view: route.view, host: route.host, label: route.label });
  return `#/popout?${params.toString()}`;
}

/** This window's pop-out route, or null when it is a normal app window. */
export const POPOUT_ROUTE = new InjectionToken<PopoutRoute | null>('POPOUT_ROUTE', {
  providedIn: 'root',
  factory: () => (typeof location === 'undefined' ? null : parsePopoutRoute(location.hash)),
});
