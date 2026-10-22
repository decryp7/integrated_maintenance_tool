import { inject, Injectable } from '@angular/core';
import { ImtCommand } from '../extensions/tokens';
import { DialogService } from '../ui/dialog.service';
import { DockService } from './dock.service';
import { PANEL_WINDOWS } from './panel-windows';
import { ManageLayoutsDialog, SaveLayoutDialog } from './layout-dialogs';
import { FullScreenMode, FullScreenPrefs } from './os-window';
import { WindowLayouts } from './window-layouts';

const CATEGORY = $localize`:Command category@@dock.commands.category:Window`;

/** Window commands of the dock (palette, Window menu, shortcuts), as in Visual Studio. */
@Injectable({ providedIn: 'root' })
export class DockCommands {
  private readonly dock = inject(DockService);
  private readonly layouts = inject(WindowLayouts);
  private readonly dialogs = inject(DialogService);
  private readonly windows = inject(PANEL_WINDOWS, { optional: true });
  private readonly prefs = inject(FullScreenPrefs);

  private mode(mode: FullScreenMode, title: string): ImtCommand {
    return { id: `window.fullScreenMode.${mode}`, title, category: CATEGORY, when: () => this.prefs.mode() !== mode, run: () => this.prefs.set(mode) };
  }

  private applyAt(n: number): ImtCommand {
    return {
      id: `window.layout.apply${n}`,
      title: $localize`:@@dock.commands.applyN:Apply window layout ${n}:n:`,
      category: CATEGORY,
      keybinding: `Ctrl+Alt+${n}`,
      when: () => this.layouts.has(n),
      run: () => void this.layouts.applyAt(n),
    };
  }

  readonly commands: ImtCommand[] = [
    { id: 'window.popOut', title: $localize`:@@dock.commands.popOut:Pop out to window`, category: CATEGORY, keybinding: 'Ctrl+Alt+O', when: () => !!this.windows?.canPopOut(), run: () => void this.dock.popOutActive() },
    { id: 'window.float', title: $localize`:@@dock.commands.float:Float panel`, category: CATEGORY, when: () => this.dock.canFloatActive(), run: () => void this.dock.floatActive() },
    { id: 'window.panelMenu', title: $localize`:@@dock.commands.panelMenu:Panel menu (window position)`, category: CATEGORY, keybinding: 'Alt+-', run: () => this.dock.openActivePanelMenu() },
    { id: 'window.autoHide', title: $localize`:@@dock.commands.autoHide:Auto hide / pin tool window`, category: CATEGORY, keybinding: 'Ctrl+Alt+H', run: () => this.dock.toggleAutoHideActive() },
    { id: 'window.autoHideAll', title: $localize`:@@dock.commands.autoHideAll:Auto hide all tool windows`, category: CATEGORY, run: () => this.dock.autoHideAll() },
    { id: 'window.pinAll', title: $localize`:@@dock.commands.pinAll:Pin all tool windows`, category: CATEGORY, when: () => this.dock.autoHide().items.length > 0, run: () => this.dock.pinAll() },
    { id: 'window.closeAllDocuments', title: $localize`:@@dock.commands.closeAllDocuments:Close all documents`, category: CATEGORY, run: () => this.dock.closeAllDocuments() },
    { id: 'window.switcher', title: $localize`:@@dock.commands.switcher:Switch panel (Ctrl+Tab)`, category: CATEGORY, run: () => this.dock.switch() },
    { id: 'window.fullScreen', title: $localize`:@@dock.commands.fullScreen:Full screen`, category: CATEGORY, keybinding: 'Shift+Alt+Enter', run: () => this.dock.toggleFullScreen() },
    this.mode('both', $localize`:@@dock.commands.fullScreenMode.both:Full screen does: hide tool windows and chrome, and OS full screen`),
    this.mode('chrome', $localize`:@@dock.commands.fullScreenMode.chrome:Full screen does: hide tool windows and chrome only`),
    this.mode('os', $localize`:@@dock.commands.fullScreenMode.os:Full screen does: OS full screen only`),
    {
      id: 'window.layout.save',
      title: $localize`:@@dock.commands.layout.save:Save window layout…`,
      category: CATEGORY,
      run: () => void this.dialogs.open(SaveLayoutDialog, { title: $localize`:@@dock.commands.layout.saveDialog:Save window layout`, width: 420 }),
    },
    { id: 'window.layout.manage', title: $localize`:@@dock.commands.layout.manage:Manage window layouts…`, category: CATEGORY, run: () => void this.dialogs.open(ManageLayoutsDialog, { title: $localize`:@@dock.commands.layout.dialogTitle:Window layouts`, width: 620 }) },
    { id: 'window.layout.apply', title: $localize`:@@dock.commands.layout.apply:Apply window layout…`, category: CATEGORY, run: () => void this.dialogs.open(ManageLayoutsDialog, { title: $localize`:@@dock.commands.layout.dialogTitle:Window layouts`, width: 620 }) },
    this.applyAt(1),
    this.applyAt(2),
    this.applyAt(3),
    this.applyAt(4),
    { id: 'window.layout.reset', title: $localize`:@@dock.commands.layout.reset:Reset window layout`, category: CATEGORY, run: () => this.dock.reset() },
  ];
}
