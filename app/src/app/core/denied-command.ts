import { signal } from '@angular/core';
import { CmdError } from './cmd-error';

/** The engine's refusal of a command this window class may not call (`commands::check`). */
const REFUSED = 'is not available in this window';

/** The last command the engine refused for this window, as a plain sentence; a window shows it so the click is never silent. */
export const deniedCommand = signal<string | null>(null);

export function isRefusedCommand(e: unknown): e is CmdError {
  return e instanceof CmdError && e.code === 'denied' && e.message.includes(REFUSED);
}

/** Called by `invoke` for every rejected command. */
export function reportRefusedCommand(cmd: string, e: unknown): void {
  if (isRefusedCommand(e)) deniedCommand.set($localize`:@@dialogWindow.denied:This window is not allowed to do that (${cmd}:command:). Close it and try again from the main window; if it keeps happening, report a problem.`);
}
