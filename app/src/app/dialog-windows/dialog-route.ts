import { InjectionToken } from '@angular/core';

/**
 * What a dialog window shows: the app loads with `#/dialog?id=<dialog>&host=<label>&label=<label>` (built by the
 * shell in Rust) and boots into the dialog shell.
 */
export interface DialogRoute {
  /** Id of the dialog (`ai-setup`). */
  id: string;
  /** Label of the Debrief window the dialog belongs to. */
  host: string;
  /** This window's own label (`d-<host>-<n>`). */
  label: string;
}

const DIALOG_ID = /^[a-z0-9][a-z0-9._-]{0,63}$/i;
const LABEL = /^[a-z0-9][a-z0-9-]{0,63}$/i;

/** Parses the window's location hash into a dialog route; null for any other window or anything malformed. */
export function parseDialogRoute(hash: string): DialogRoute | null {
  const match = /^#\/dialog\?(.*)$/.exec(hash ?? '');
  if (!match) return null;
  const params = new URLSearchParams(match[1]);
  const id = params.get('id') ?? '';
  const host = params.get('host') ?? '';
  const label = params.get('label') ?? '';
  if (!DIALOG_ID.test(id) || !LABEL.test(host) || !LABEL.test(label) || !label.startsWith('d-')) return null;
  return { id, host, label };
}

/** The hash that routes a window to the dialog shell. */
export function dialogHash(route: DialogRoute): string {
  return `#/dialog?${new URLSearchParams({ id: route.id, host: route.host, label: route.label }).toString()}`;
}

/** This window's dialog route, or null when it is not a dialog window. */
export const DIALOG_ROUTE = new InjectionToken<DialogRoute | null>('DIALOG_ROUTE', {
  providedIn: 'root',
  factory: () => (typeof location === 'undefined' ? null : parseDialogRoute(location.hash)),
});
