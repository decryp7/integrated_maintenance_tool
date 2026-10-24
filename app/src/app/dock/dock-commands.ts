import { inject, Injectable } from '@angular/core';
import { DebriefCommand } from '../extensions/tokens';
import { DialogService } from '../ui/dialog.service';
import { DockService } from './dock.service';
import { PANEL_WINDOWS } from './panel-windows';
import { FullScreenMode, FullScreenPrefs } from './os-window';
import { Perspectives, PERSPECTIVE_KEYS } from './perspectives';

const CATEGORY = $localize`:Command category@@dock.commands.category:Window`;

/** Window commands of the dock (palette, Window menu, shortcuts), as in Visual Studio. */
@Injectable({ providedIn: 'root' })
export class DockCommands {
  private readonly dock = inject(DockService);
  private readonly perspectives = inject(Perspectives);
  private readonly dialogs = inject(DialogService);
  private readonly windows = inject(PANEL_WINDOWS, { optional: true });
  private readonly prefs = inject(FullScreenPrefs);

  private mode(mode: FullScreenMode, title: string): DebriefCommand {
    return { id: `window.fullScreenMode.${mode}`, title, category: CATEGORY, when: () => this.prefs.mode() !== mode, run: () => this.prefs.set(mode) };
  }

  /** Ctrl+n switches to perspective n; the title follows the perspective's (current) name. */
  private perspectiveAt(n: number): DebriefCommand {
    const perspectives = this.perspectives;
    return {
      id: `window.perspective.${n}`,
      get title() {
        const name = perspectives.at(n)?.name ?? String(n);
        return $localize`:Command title|@@dock.commands.perspective:Perspective: ${name}:name:`;
      },
      category: CATEGORY,
      keybinding: `Mod+${n}`,
      when: () => !!this.perspectives.at(n),
      run: () => void this.perspectives.switchToAt(n),
    };
  }

  readonly commands: DebriefCommand[] = [
    { id: 'window.popOut', title: $localize`:@@dock.commands.popOut:Pop out to window`, category: CATEGORY, keybinding: 'Ctrl+Alt+O', when: () => !!this.windows?.canPopOut(), run: () => void this.dock.popOutActive() },
    { id: 'window.float', title: $localize`:@@dock.commands.float:Float panel`, category: CATEGORY, when: () => this.dock.canFloatActive(), run: () => void this.dock.floatActive() },
    { id: 'window.panelMenu', title: $localize`:@@dock.commands.panelMenu:Panel menu (window position)`, category: CATEGORY, keybinding: 'Alt+-', run: () => this.dock.openActivePanelMenu() },
    { id: 'window.autoHide', title: $localize`:@@dock.commands.autoHide:Auto hide / pin tool window`, category: CATEGORY, keybinding: 'Ctrl+Alt+H', run: () => this.dock.toggleAutoHideActive() },
    { id: 'window.autoHideAll', title: $localize`:@@dock.commands.autoHideAll:Auto hide all tool windows`, category: CATEGORY, run: () => void this.dock.autoHideAll() },
    { id: 'window.pinAll', title: $localize`:@@dock.commands.pinAll:Pin all tool windows`, category: CATEGORY, when: () => this.dock.autoHide().items.length > 0, run: () => this.dock.pinAll() },
    { id: 'window.closeAllDocuments', title: $localize`:@@dock.commands.closeAllDocuments:Close all documents`, category: CATEGORY, run: () => this.dock.closeAllDocuments() },
    { id: 'window.switcher', title: $localize`:@@dock.commands.switcher:Switch panel (Ctrl+Tab)`, category: CATEGORY, run: () => this.dock.switch() },
    { id: 'window.fullScreen', title: $localize`:@@dock.commands.fullScreen:Full screen`, category: CATEGORY, keybinding: 'Shift+Alt+Enter', run: () => this.dock.toggleFullScreen() },
    this.mode('both', $localize`:@@dock.commands.fullScreenMode.both:Full screen does: hide tool windows and chrome, and OS full screen`),
    this.mode('chrome', $localize`:@@dock.commands.fullScreenMode.chrome:Full screen does: hide tool windows and chrome only`),
    this.mode('os', $localize`:@@dock.commands.fullScreenMode.os:Full screen does: OS full screen only`),
    {
      id: 'window.layout.save',
      title: $localize`:@@dock.commands.perspective.save:Save current as perspective…`,
      category: CATEGORY,
      run: () => void this.dialogs.openLazy(() => import('./layout-dialogs').then((m) => m.SaveLayoutDialog), { title: $localize`:@@dock.commands.perspective.saveDialog:Save as perspective`, width: 420 }),
    },
    { id: 'window.layout.manage', title: $localize`:@@dock.commands.perspective.manage:Manage perspectives…`, category: CATEGORY, run: () => void this.dialogs.openLazy(() => import('./layout-dialogs').then((m) => m.ManageLayoutsDialog), { title: $localize`:@@dock.commands.perspective.dialogTitle:Perspectives`, width: 620 }) },
    ...Array.from({ length: PERSPECTIVE_KEYS }, (_, i) => this.perspectiveAt(i + 1)),
    { id: 'window.layout.reset', title: $localize`:@@dock.commands.perspective.reset:Reset perspective`, category: CATEGORY, run: () => this.perspectives.reset() },
  ];
}
