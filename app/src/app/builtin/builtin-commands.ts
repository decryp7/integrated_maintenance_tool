import { inject, Injectable } from '@angular/core';
import { ExportService } from '../export/export.service';
import { WorkspaceStore } from '../core/workspace.store';
import { DockService } from '../dock/dock.service';
import { PaletteService } from '../palette/palette.service';
import { ImtCommand } from '../extensions/tokens';
import { isViewAvailable, ViewRegistry } from '../extensions/view-registry';
import { DIAGNOSTICS_SECTION, SETTINGS_DIAGNOSTICS_COMMAND, SettingsDialogService } from '../settings/settings-dialog';
import { AboutService } from '../about/about-dialog';
import { TimelineZoom } from '../timeline/zoom';
import { WelcomeService } from '../welcome/welcome';

/** Built-in commands: everything the menus offer, available from the palette and key bindings. */
@Injectable({ providedIn: 'root' })
export class BuiltinCommands {
  private readonly store = inject(WorkspaceStore);
  private readonly zoom = inject(TimelineZoom);
  private readonly dock = inject(DockService);
  private readonly views = inject(ViewRegistry);
  private readonly palette = inject(PaletteService);
  private readonly exporter = inject(ExportService);
  private readonly welcome = inject(WelcomeService);
  private readonly settings = inject(SettingsDialogService);
  private readonly about = inject(AboutService);

  readonly commands: ImtCommand[] = [
    { id: 'palette.open', title: 'Show all commands', category: 'View', keybinding: 'Mod+K', run: () => this.palette.open() },
    { id: 'file.openFiles', title: 'Open files…', category: 'File', keybinding: 'Mod+O', run: () => this.store.pickAndOpen(false) },
    { id: 'file.openFolder', title: 'Open folder…', category: 'File', keybinding: 'Mod+Shift+O', run: () => this.store.pickAndOpen(true) },
    { id: 'file.reimport', title: 'Re-import all', category: 'File', run: () => this.store.reload() },
    { id: 'file.close', title: 'Close workspace', category: 'File', run: () => this.store.clearWorkspace() },
    {
      id: 'file.export',
      title: 'Export events…',
      category: 'File',
      keybinding: 'Mod+Shift+E',
      when: (ctx) => !!ctx.summary?.exporters.length,
      run: () => this.exporter.exportCurrent(),
    },
    { id: 'help.welcome', title: 'Show welcome', category: 'Help', run: () => this.welcome.show() },
    { id: 'help.about', title: 'About IMT', category: 'Help', run: () => this.about.open() },
    // The About dialog opens on the third-party notices viewer.
    { id: 'help.notices', title: 'Third-party notices', category: 'Help', run: () => this.about.open() },
    { id: 'settings.open', title: 'Open settings', category: 'View', keybinding: 'Mod+,', run: () => this.settings.open() },
    { id: SETTINGS_DIAGNOSTICS_COMMAND, title: 'Open settings: Diagnostics', category: 'View', run: () => this.settings.open(DIAGNOSTICS_SECTION) },
    { id: 'view.toggleSidebar', title: 'Toggle sidebar', category: 'View', keybinding: 'Mod+B', run: () => this.store.toggleSidebar() },
    { id: 'view.resetLayout', title: 'Reset layout', category: 'View', run: () => this.dock.reset() },
    { id: 'view.theme.system', title: 'Theme: system', category: 'View', run: () => this.store.setTheme('system') },
    { id: 'view.theme.dark', title: 'Theme: dark', category: 'View', run: () => this.store.setTheme('dark') },
    { id: 'view.theme.light', title: 'Theme: light', category: 'View', run: () => this.store.setTheme('light') },
    { id: 'analyze.search', title: 'Search events', category: 'Analyze', run: () => this.store.focusSearch() },
    { id: 'analyze.markedOnly', title: 'Marked events only', category: 'Analyze', run: () => this.store.setMarkedOnly(!this.store.markedOnly()) },
    { id: 'analyze.fit', title: 'Fit time range', category: 'Analyze', run: () => this.zoom.fit() },
    { id: 'analyze.back', title: 'Previous time range', category: 'Analyze', run: () => this.zoom.back() },
    { id: 'analyze.forward', title: 'Next time range', category: 'Analyze', run: () => this.zoom.forward() },
    { id: 'analyze.clearFilters', title: 'Clear all filters', category: 'Analyze', run: () => this.store.clearQuery() },
    { id: 'analyze.utc', title: 'Time in UTC', category: 'Analyze', run: () => this.store.setTimezone('utc') },
    { id: 'analyze.local', title: 'Time in local zone', category: 'Analyze', run: () => this.store.setTimezone('local') },
    {
      id: 'analyze.toggleUtc',
      title: 'Toggle UTC / local time',
      category: 'Analyze',
      run: () => this.store.setTimezone(this.store.timezone() === 'utc' ? 'local' : 'utc'),
    },
    // One "Toggle <panel>" command per registered view (built in and from plugins).
    ...this.views.all.map(
      (view): ImtCommand => ({
        id: `view.toggle.${view.id}`,
        title: `Toggle ${view.title} panel`,
        category: 'View',
        when: (ctx) => isViewAvailable(view, ctx.summary),
        run: () => this.dock.toggle(view.id),
      }),
    ),
  ];
}
