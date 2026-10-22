import { inject, Injectable } from '@angular/core';
import { ExportService } from '../export/export.service';
import { WorkspaceStore } from '../core/workspace.store';
import { DockService } from '../dock/dock.service';
import { PaletteService } from '../palette/palette.service';
import { ImtCommand, VIEW_OPEN_PREFIX } from '../extensions/tokens';
import { isViewAvailable, ViewRegistry } from '../extensions/view-registry';
import { DIAGNOSTICS_SECTION, SETTINGS_DIAGNOSTICS_COMMAND, SettingsDialogService } from '../settings/settings-dialog';
import { CLEAR_CACHE_COMMAND, STORAGE_ID } from '../settings/storage/storage.types';
import { APPEARANCE_ID } from '../settings/settings-model';
import { AboutService } from '../about/about-dialog';
import { TimelineZoom } from '../timeline/zoom';
import { WelcomeService } from '../welcome/welcome';
import { SettingsStore } from '../settings/settings.store';

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
  private readonly prefs = inject(SettingsStore);

  private showOverWelcome(id: string): void {
    this.welcome.stepAside();
    this.dock.show(id);
  }

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
    { id: 'settings.appearance', title: $localize`:@@builtin.cmd.settings.appearance:Open settings: Appearance`, category: $localize`:@@builtin.category.view:View`, run: () => this.settings.open(APPEARANCE_ID) },
    { id: SETTINGS_DIAGNOSTICS_COMMAND, title: $localize`:@@builtin.cmd.settings.diagnostics:Open settings: Diagnostics`, category: $localize`:@@builtin.category.view:View`, run: () => this.settings.open(DIAGNOSTICS_SECTION) },
    { id: 'view.toggleSidebar', title: $localize`:@@builtin.cmd.view.toggleSidebar:Toggle sidebar`, category: $localize`:@@builtin.category.view:View`, keybinding: 'Mod+B', run: () => this.store.toggleSidebar() },
    { id: 'view.resetLayout', title: $localize`:@@builtin.cmd.view.resetLayout:Reset layout`, category: $localize`:@@builtin.category.view:View`, run: () => this.dock.reset() },
    { id: 'view.theme.system', title: $localize`:@@builtin.cmd.view.theme.system:Theme: Follow system`, category: $localize`:@@builtin.category.view:View`, run: () => this.store.setTheme(null) },
    { id: 'view.theme.calm', title: $localize`:@@builtin.cmd.view.theme.calm:Theme: Calm`, category: $localize`:@@builtin.category.view:View`, run: () => this.store.setTheme('calm') },
    { id: 'view.theme.console', title: $localize`:@@builtin.cmd.view.theme.console:Theme: Console`, category: $localize`:@@builtin.category.view:View`, run: () => this.store.setTheme('console') },
    { id: 'view.theme.precision', title: $localize`:@@builtin.cmd.view.theme.precision:Theme: Precision`, category: $localize`:@@builtin.category.view:View`, run: () => this.store.setTheme('precision') },
    { id: 'view.theme.contrast', title: $localize`:@@builtin.cmd.view.theme.contrast:Theme: High contrast`, category: $localize`:@@builtin.category.view:View`, run: () => this.store.setTheme('contrast') },
    { id: 'view.scheme.system', title: $localize`:@@builtin.cmd.view.scheme.system:Colour scheme: system`, category: $localize`:@@builtin.category.view:View`, run: () => this.store.setScheme('system') },
    { id: 'view.scheme.dark', title: $localize`:@@builtin.cmd.view.scheme.dark:Colour scheme: dark`, category: $localize`:@@builtin.category.view:View`, run: () => this.store.setScheme('dark') },
    { id: 'view.scheme.light', title: $localize`:@@builtin.cmd.view.scheme.light:Colour scheme: light`, category: $localize`:@@builtin.category.view:View`, run: () => this.store.setScheme('light') },
    { id: 'view.sourceColors.source', title: $localize`:@@builtin.cmd.view.sourceColors.source:Source colours: by file`, category: $localize`:@@builtin.category.view:View`, run: () => this.prefs.setSourceColors('source') },
    { id: 'view.sourceColors.instance', title: $localize`:@@builtin.cmd.view.sourceColors.instance:Source colours: by instance`, category: $localize`:@@builtin.category.view:View`, run: () => this.prefs.setSourceColors('instance') },
    { id: 'view.sourceColors.off', title: $localize`:@@builtin.cmd.view.sourceColors.off:Source colours: off`, category: $localize`:@@builtin.category.view:View`, run: () => this.prefs.setSourceColors('off') },
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
    // One "Open <view>" command per registered view (built in and from plugins); the View menu toggles them.
    ...this.views.all.map(
      (view): ImtCommand => ({
        id: `${VIEW_OPEN_PREFIX}${view.id}`,
        title: $localize`:@@builtin.cmd.openView:Open ${view.title}:view:`,
        category: $localize`:@@builtin.category.view:View`,
        view: view.id,
        when: (ctx) => isViewAvailable(view, ctx.summary),
        // Behind the welcome screen a docked panel is open but unseen: step the welcome page aside too.
        run: () => (this.welcome.visible() ? this.showOverWelcome(view.id) : this.dock.show(view.id)),
      }),
    ),
  ];
}
