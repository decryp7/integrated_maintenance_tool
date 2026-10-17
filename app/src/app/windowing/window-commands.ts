import { inject, Injectable } from '@angular/core';
import { ImtCommand } from '../extensions/tokens';
import { WindowingService } from './windowing.service';

export const NEW_WINDOW_KEYBINDING = 'Mod+Shift+N';

/** New, close and switch windows (palette, Window menu, shortcut). */
@Injectable({ providedIn: 'root' })
export class WindowCommands {
  private readonly windows = inject(WindowingService);

  readonly commands: ImtCommand[] = [
    { id: 'window.new', title: 'New window', category: 'Window', keybinding: NEW_WINDOW_KEYBINDING, run: () => this.windows.newWindow() },
    { id: 'window.close', title: 'Close window', category: 'Window', run: () => this.windows.closeWindow() },
    { id: 'window.next', title: 'Switch to next window', category: 'Window', when: () => this.windows.windows().length > 1, run: () => this.windows.focusNext() },
  ];
}
