import { SettingsStore } from '../../settings/settings.store';
import { LayoutModeStore } from '../../core/workspace/layout-mode';
import { SidebarSections } from '../sidebar-sections';
import { afterNextRender, computed, inject, Injectable, InjectionToken, Injector } from '@angular/core';
import { isTauri } from '../../core/platform';
import { WorkspaceStore } from '../../core/workspace.store';
import { DockService } from '../../dock/dock.service';
import { PANEL_WINDOWS } from '../../dock/panel-windows';
import { CommandRegistry } from '../../extensions/command-registry';
import { isMac } from '../../extensions/keybinding';
import { ViewRegistry } from '../../extensions/view-registry';
import { PaletteService } from '../../palette/palette.service';
import { UndoStack } from '../../undo/undo-stack';
import { WindowingService } from '../../windowing/windowing.service';
import { buildMenus, MENU_IDS, MenuCommand, TopMenu } from './menu-model';

/** Where the menus are shown: IMT's own menu bar, or the macOS native menu bar (the in-app bar is then hidden). */
export type MenuMode = 'in-app' | 'native';

/** The menu mode of this run: native in the macOS desktop app, else in-app (Windows, Linux, browser). */
export const MENU_MODE = new InjectionToken<MenuMode>('MENU_MODE', {
  providedIn: 'root',
  factory: () => (isTauri() && isMac() ? 'native' : 'in-app'),
});

/** The application menus (one model for both modes) and what their items do. */
@Injectable({ providedIn: 'root' })
export class AppMenu {
  private readonly registry = inject(CommandRegistry);
  private readonly views = inject(ViewRegistry);
  private readonly dock = inject(DockService);
  private readonly store = inject(WorkspaceStore);
  private readonly undo = inject(UndoStack);
  private readonly windowing = inject(WindowingService);
  private readonly palette = inject(PaletteService);
  private readonly panelWindows = inject(PANEL_WINDOWS, { optional: true });
  private readonly injector = inject(Injector);
  private readonly prefs = inject(SettingsStore);
  private readonly layout = inject(LayoutModeStore);
  private readonly sections = inject(SidebarSections);
  readonly mode = inject(MENU_MODE);

  readonly menus = computed<TopMenu[]>(() => {
    const open = this.dock.openIds();
    const popped = this.panelWindows?.poppedIds() ?? new Set<string>();
    const commands = new Map<string, MenuCommand>(this.registry.available().map((c) => [c.id, { id: c.id, title: c.title, keybinding: c.keybinding }]));
    const label = this.windowing.label();
    return buildMenus({
      commands,
      views: this.views.available().map((v) => ({ id: v.id, title: v.title, category: v.category, plugin: v.plugin, menu: v.menu, open: open.has(v.id) })),
      sidebarOpen: this.store.sidebarOpen(),
      theme: this.store.theme(),
      scheme: this.store.scheme(),
      sourceColors: this.prefs.sourceColors(),
      layoutMode: this.layout.mode(),
      timezone: this.store.timezone(),
      markedOnly: this.store.markedOnly(),
      undoLabel: this.undo.undoLabel(),
      redoLabel: this.undo.redoLabel(),
      windows: this.windowing.windows().map((w) => ({ label: w.label, title: w.title, current: w.label === label })),
      popouts: [...popped].map((id) => ({ id, title: this.views.find(id)?.title ?? id })),
      nativeAppMenu: this.mode === 'native',
    });
  });

  /** Runs a menu item by id (from the in-app bar or the native menu). Unknown ids are ignored. */
  run(id: string): void {
    if (id.startsWith(MENU_IDS.view)) {
      // Open includes auto-hidden and popped-out panels: toggling closes those too.
      this.dock.toggle(id.slice(MENU_IDS.view.length));
    } else if (id.startsWith(MENU_IDS.window)) {
      void this.windowing.focus(id.slice(MENU_IDS.window.length));
    } else if (id.startsWith(MENU_IDS.popout)) {
      this.panelWindows?.focus(id.slice(MENU_IDS.popout.length));
    } else if (id === MENU_IDS.otherWindows) {
      this.palette.open('views');
    } else if (id === MENU_IDS.fields) {
      this.showFields();
    } else {
      void this.registry.run(id);
    }
  }

  /** Shows the sidebar (if hidden) with the Fields section expanded and puts the focus in the fields filter. */
  private showFields(): void {
    if (!this.store.sidebarOpen()) this.store.toggleSidebar();
    this.sections.setFieldsOpen(true);
    afterNextRender(() => document.querySelector<HTMLInputElement>('imt-fields-panel input.filter, imt-fields-panel input[aria-label="Filter fields"]')?.focus(), { injector: this.injector });
  }
}
