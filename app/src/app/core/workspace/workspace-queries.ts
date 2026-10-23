import { InjectionToken } from '@angular/core';

/**
 * Whether this window fetches the workspace's own data (the timeline) as soon as it knows the workspace. True in the
 * main window and pop-outs; a dialog window turns it off (ADR 0050: it queries only what its dialog needs), so opening
 * a small dialog does not run the host's timeline query again for every dialog window.
 */
export const WORKSPACE_QUERIES = new InjectionToken<boolean>('WORKSPACE_QUERIES', { providedIn: 'root', factory: () => true });

/**
 * Whether this window loads the workspace summary when its store starts. True in the main window and pop-outs; a dialog
 * window loads it only when its dialog names the `summary` (or `workspace`) feature, so a small dialog (Close Debrief?)
 * does not fetch the host's workspace for nothing.
 */
export const WORKSPACE_SUMMARY = new InjectionToken<boolean>('WORKSPACE_SUMMARY', { providedIn: 'root', factory: () => true });
