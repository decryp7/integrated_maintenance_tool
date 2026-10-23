import { InjectionToken } from '@angular/core';

/**
 * Where a command goes when it is run from a dialog window (ADR 0050): commands act on the Debrief window that opened
 * the dialog (open a view, change the layout), so the dialog window hands them to its host instead of running them in
 * itself. Returns true when the command was handed over. In an ordinary window nothing is forwarded.
 */
export const HOST_COMMAND_FORWARD = new InjectionToken<(commandId: string) => boolean>('HOST_COMMAND_FORWARD', { providedIn: 'root', factory: () => () => false });
