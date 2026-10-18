import { inject, Injectable } from '@angular/core';
import { ExportService } from '../export/export.service';
import { WorkspaceStore } from '../core/workspace.store';
import { DockService } from '../dock/dock.service';
import { PaletteService } from '../palette/palette.service';
import { ImtCommand } from '../extensions/tokens';
import { isViewAvailable, ViewRegistry } from '../extensions/view-registry';
import { DIAGNOSTICS_SECTION, SETTINGS_DIAGNOSTICS_COMMAND, SettingsDialogService } from '../settings/settings-dialog';
import { CLEAR_CACHE_COMMAND, STORAGE_ID } from '../settings/storage/storage.types';
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
    { id: 'palette.open', title: $localize`:@@builtin.cmd.palette.open:Show all commands`, category: $localize`:@@builtin.category.view:View`, keybinding: 'Mod+K', run: () => this.palette.open() },
    { id: 'file.openFiles', title: $localize`:@@builtin.cmd.file.openFiles:Open files…`, category: $localize`:@@builtin.category.file:File`, keybinding: 'Mod+O', run: () => this.store.pickAndOpen(false) },
    { id: 'file.openFolder', title: $localize`:@@builtin.cmd.file.openFolder:Open folder…`, category: $localize`:@@builtin.category.file:File`, keybinding: 'Mod+Shift+O', run: () => this.store.pickAndOpen(true) },
    { id: 'file.reimport', title: $localize`:@@builtin.cmd.file.reimport:Re-import all`, category: $localize`:@@builtin.category.file:File`, run: () => this.store.reload() },
    { id: 'file.close', title: $localize`:@@builtin.cmd.file.close:Close workspace`, category: $localize`:@@builtin.category.file:File`, run: () => this.store.clearWorkspace() },
    {
      id: 'file.export',
      title: $localize`:@@builtin.cmd.file.export:Export events…`,
      category: $localize`:@@builtin.category.file:File`,
      keybinding: 'Mod+Shift+E',
      when: (ctx) => !!ctx.summary?.exporters.length,
      run: () => this.exporter.exportCurrent(),
    },
    { id: 'help.welcome', title: $localize`:@@builtin.cmd.help.welcome:Show welcome`, category: $localize`:@@builtin.category.help:Help`, run: () => this.welcome.show() },
    { id: 'help.about', title: $localize`:@@builtin.cmd.help.about:About IMT`, category: $localize`:@@builtin.category.help:Help`, run: () => this.about.open() },
    // The About dialog opens on the third-party notices viewer.
    { id: 'help.notices', title: $localize`:@@builtin.cmd.help.notices:Third-party notices`, category: $localize`:@@builtin.category.help:Help`, run: () => this.about.open() },
    { id: 'settings.open', title: $localize`:@@builtin.cmd.settings.open:Open settings`, category: $localize`:@@builtin.category.view:View`, keybinding: 'Mod+,', run: () => this.settings.open() },
    { id: CLEAR_CACHE_COMMAND, title: $localize`:@@settings.storage.cmd.clearCache:Clear cache…`, category: $localize`:@@builtin.category.help:Help`, run: () => this.settings.open(STORAGE_ID) },
    { id: SETTINGS_DIAGNOSTICS_COMMAND, title: $localize`:@@builtin.cmd.settings.diagnostics:Open settings: Diagnostics`, category: $localize`:@@builtin.category.view:View`, run: () => this.settings.open(DIAGNOSTICS_SECTION) },
    { id: 'view.toggleSidebar', title: $localize`:@@builtin.cmd.view.toggleSidebar:Toggle sidebar`, category: $localize`:@@builtin.category.view:View`, keybinding: 'Mod+B', run: () => this.store.toggleSidebar() },
    { id: 'view.resetLayout', title: $localize`:@@builtin.cmd.view.resetLayout:Reset layout`, category: $localize`:@@builtin.category.view:View`, run: () => this.dock.reset() },
    { id: 'view.theme.system', title: $localize`:@@builtin.cmd.view.theme.system:Theme: system`, category: $localize`:@@builtin.category.view:View`, run: () => this.store.setTheme('system') },
    { id: 'view.theme.dark', title: $localize`:@@builtin.cmd.view.theme.dark:Theme: dark`, category: $localize`:@@builtin.category.view:View`, run: () => this.store.setTheme('dark') },
    { id: 'view.theme.light', title: $localize`:@@builtin.cmd.view.theme.light:Theme: light`, category: $localize`:@@builtin.category.view:View`, run: () => this.store.setTheme('light') },
    { id: 'analyze.search', title: $localize`:@@builtin.cmd.analyze.search:Search events`, category: $localize`:@@builtin.category.analyze:Analyze`, run: () => this.store.focusSearch() },
    { id: 'analyze.markedOnly', title: $localize`:@@builtin.cmd.analyze.markedOnly:Marked events only`, category: $localize`:@@builtin.category.analyze:Analyze`, run: () => this.store.setMarkedOnly(!this.store.markedOnly()) },
    { id: 'analyze.fit', title: $localize`:@@builtin.cmd.analyze.fit:Fit time range`, category: $localize`:@@builtin.category.analyze:Analyze`, run: () => this.zoom.fit() },
    { id: 'analyze.back', title: $localize`:@@builtin.cmd.analyze.back:Previous time range`, category: $localize`:@@builtin.category.analyze:Analyze`, run: () => this.zoom.back() },
    { id: 'analyze.forward', title: $localize`:@@builtin.cmd.analyze.forward:Next time range`, category: $localize`:@@builtin.category.analyze:Analyze`, run: () => this.zoom.forward() },
    { id: 'analyze.clearFilters', title: $localize`:@@builtin.cmd.analyze.clearFilters:Clear all filters`, category: $localize`:@@builtin.category.analyze:Analyze`, run: () => this.store.clearQuery() },
    { id: 'analyze.utc', title: $localize`:@@builtin.cmd.analyze.utc:Time in UTC`, category: $localize`:@@builtin.category.analyze:Analyze`, run: () => this.store.setTimezone('utc') },
    { id: 'analyze.local', title: $localize`:@@builtin.cmd.analyze.local:Time in local zone`, category: $localize`:@@builtin.category.analyze:Analyze`, run: () => this.store.setTimezone('local') },
    {
      id: 'analyze.toggleUtc',
      title: $localize`:@@builtin.cmd.analyze.toggleUtc:Toggle UTC / local time`,
      category: $localize`:@@builtin.category.analyze:Analyze`,
      run: () => this.store.setTimezone(this.store.timezone() === 'utc' ? 'local' : 'utc'),
    },
    // One "Toggle <panel>" command per registered view (built in and from plugins).
    ...this.views.all.map(
      (view): ImtCommand => ({
        id: `view.toggle.${view.id}`,
        title: $localize`:@@builtin.cmd.toggleView:Toggle ${view.title}:view: panel`,
        category: $localize`:@@builtin.category.view:View`,
        when: (ctx) => isViewAvailable(view, ctx.summary),
        run: () => this.dock.toggle(view.id),
      }),
    ),
  ];
}
