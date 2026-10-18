import { inject, Injectable } from '@angular/core';
import { ImtCommand } from '../extensions/tokens';
import { WindowingService } from './windowing.service';

export const NEW_WINDOW_KEYBINDING = 'Mod+Shift+N';

/** New, close and switch windows (palette, Window menu, shortcut). */
@Injectable({ providedIn: 'root' })
export class WindowCommands {
  private readonly windows = inject(WindowingService);

  readonly commands: ImtCommand[] = [
    { id: 'window.new', title: $localize`:@@windowing.commands.new:New window`, category: $localize`:Command category@@windowing.commands.category:Window`, keybinding: NEW_WINDOW_KEYBINDING, run: () => this.windows.newWindow() },
    { id: 'window.close', title: $localize`:@@windowing.commands.close:Close window`, category: $localize`:Command category@@windowing.commands.category:Window`, run: () => this.windows.closeWindow() },
    { id: 'window.next', title: $localize`:@@windowing.commands.next:Switch to next window`, category: $localize`:Command category@@windowing.commands.category:Window`, when: () => this.windows.windows().length > 1, run: () => this.windows.focusNext() },
  ];
}
