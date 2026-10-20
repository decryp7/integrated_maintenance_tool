import { inject, Injectable, Injector } from '@angular/core';
import { ImtCommand } from '../extensions/tokens';
import { isMac } from '../extensions/keybinding';
import { UnsavedChangesGuard } from '../projects/unsaved-changes';
import { WindowingService } from './windowing.service';

export const NEW_WINDOW_KEYBINDING = 'Mod+Shift+N';
/** IMT ▸ Quit IMT on macOS (on Windows and Linux, Ctrl+Q and Alt+F4 stay per window: File ▸ Exit has none). */
export const QUIT_KEYBINDING = 'Mod+Q';
/** The quit command (the native macOS menu puts it last in the app menu; see `menu.rs`). */
export const QUIT_COMMAND = 'app.quit';

/** New, close and switch windows, and quit IMT (palette, File and Window menus, shortcut). */
@Injectable({ providedIn: 'root' })
export class WindowCommands {
  private readonly windows = inject(WindowingService);
  // Resolved when quitting: the project service is not needed before.
  private readonly injector = inject(Injector);

  readonly commands: ImtCommand[] = [
    { id: 'window.new', title: $localize`:@@windowing.commands.new:New window`, category: $localize`:Command category@@windowing.commands.category:Window`, keybinding: NEW_WINDOW_KEYBINDING, run: () => this.windows.newWindow() },
    { id: 'window.close', title: $localize`:@@windowing.commands.close:Close window`, category: $localize`:Command category@@windowing.commands.category:Window`, run: () => this.windows.closeWindow() },
    { id: 'window.next', title: $localize`:@@windowing.commands.next:Switch to next window`, category: $localize`:Command category@@windowing.commands.category:Window`, when: () => this.windows.windows().length > 1, run: () => this.windows.focusNext() },
    {
      id: QUIT_COMMAND,
      title: isMac() ? $localize`:@@windowing.commands.quitMac:Quit IMT` : $localize`:@@windowing.commands.exit:Exit`,
      category: $localize`:Command category@@windowing.commands.category:Window`,
      ...(isMac() ? { keybinding: QUIT_KEYBINDING } : {}),
      run: () => this.quit(),
    },
  ];

  /** Asks about this window's unsaved project once (the others keep their autosaved sessions), then quits. */
  private async quit(): Promise<void> {
    const action = $localize`:Completes "Save the changes before you ..."|@@windowing.commands.quitAction:quit IMT`;
    if (await this.injector.get(UnsavedChangesGuard).confirm(action)) await this.windows.quitApp();
  }
}
