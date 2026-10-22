import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import { formatKeybinding } from '../extensions/keybinding';
import { ProjectService } from '../projects/project.service';
import { WorkspaceStore } from '../core/workspace.store';
import { AppLogo } from '../shared/app-logo';
import { Icon } from '../shared/icon';
import { CommandRegistry } from '../extensions/command-registry';
import { PaletteService } from '../palette/palette.service';
import { TitleExtras } from '../lifecycle/title-extras';
import { MenuBar } from './menu/menu-bar';
import { MENU_MODE } from './menu/app-menu.service';
import { NativeMenuBridge } from './menu/native-menu';

@Component({
  selector: 'imt-title-bar',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AppLogo, Icon, MenuBar, TitleExtras],
  template: `
    <div class="brand">
      <imt-app-logo [size]="20" />
      <span class="name">IMT</span>
    </div>
    @if (mode === 'in-app') {
      <imt-menu-bar class="menus" />
    }
    <div class="palette-wrap">
      <button class="palette" (click)="openPalette()" [title]="paletteTitle">
        <imt-icon name="search" [size]="13" />
        <span class="ellipsis" data-testid="command-centre-name"><span class="where">{{ where() }}</span> <ng-container i18n="@@shell.titleBar.palettePromptShort">— type a command</ng-container></span>
        <span class="mono faint kbd">{{ paletteKey }}</span>
      </button>
    </div>
    <imt-title-extras />
  `,
  host: { '(document:keydown)': 'onKey($event)' },
  styles: `
    :host { height: var(--title-h); flex: none; display: flex; align-items: center; gap: 16px; padding: 0 12px; background: var(--titlebar-bg); border-bottom: 1px solid var(--border); }
    .brand { display: flex; align-items: center; gap: 8px; }
    .name { font-weight: 600; letter-spacing: 0.04em; }
    .palette-wrap { flex: 1; display: flex; justify-content: center; min-width: 0; }
    .palette { width: 440px; max-width: 100%; height: 24px; display: flex; align-items: center; gap: 8px; padding: 0 10px; border-radius: 6px; background: var(--bg-3); border: 1px solid var(--border-2); color: var(--faint); font-size: var(--fs-sm); cursor: text; }
    .palette span:nth-child(2) { flex: 1; text-align: start; }
    .kbd { font-size: var(--fs-xs); }
    .where { color: var(--text-2); }
  `,
})
export class TitleBar {
  protected readonly store = inject(WorkspaceStore);
  private readonly projects = inject(ProjectService);
  protected readonly paletteKey = formatKeybinding('Mod+K');
  protected readonly paletteTitle = $localize`:@@shell.titleBar.paletteTitleKey:Command palette (${this.paletteKey}:key:)`;
  /**
   * What is open, for the command centre: the saved project's name, else the folder of the opened files, else
   * "No folder open". Progress and the parsing profile are in the status bar, not here.
   */
  protected readonly where = computed(() => {
    if (this.projects.path()) return this.projects.name();
    const sources = this.store.summary()?.sources ?? [];
    return sources.length ? openedFolderName(sources.map((s) => s.path)) : $localize`:@@shell.titleBar.noFolder:No folder open`;
  });
  private readonly registry = inject(CommandRegistry);
  private readonly palette = inject(PaletteService);
  /** On macOS the menus are in the native menu bar and the in-app bar is hidden. */
  protected readonly mode = inject(MENU_MODE);

  constructor() {
    if (this.mode === 'native') inject(NativeMenuBridge).start();
  }

  protected openPalette(): void {
    this.palette.open();
  }

  protected onKey(e: KeyboardEvent): void {
    const target = e.target as HTMLElement | null;
    const typing = !!target && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable);
    // A bare '/' focuses the search box; Ctrl/⌘+/ is a command (focus the query bar).
    if (e.key === '/' && !e.ctrlKey && !e.metaKey && !e.altKey && !typing) {
      e.preventDefault();
      this.store.focusSearch();
      return;
    }
    // Inside a text field, Mod+Z / Mod+Shift+Z undo the typing (the browser's own undo), not the last edit.
    if (isTextEntry(target) && (e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === 'z') return;
    this.registry.runForKey(e);
  }
}

const TEXT_INPUTS = new Set(['', 'text', 'search', 'email', 'url', 'tel', 'password', 'number']);

/** Is the element a place where text is typed (so the browser's own undo applies)? */
export function isTextEntry(el: HTMLElement | null): boolean {
  if (!el) return false;
  if (el.tagName === 'TEXTAREA' || el.isContentEditable) return true;
  return el.tagName === 'INPUT' && TEXT_INPUTS.has((el.getAttribute('type') ?? '').toLowerCase());
}

/** The name of the deepest folder holding every path ("logs" for /data/logs/a.log and /data/logs/web/b.log). */
export function openedFolderName(paths: string[]): string {
  const split = (p: string) => p.split(/[\\/]/).filter(Boolean);
  const dirs = paths.map((p) => split(p).slice(0, -1));
  let common = dirs[0] ?? [];
  for (const d of dirs.slice(1)) {
    let i = 0;
    while (i < common.length && i < d.length && common[i] === d[i]) i++;
    common = common.slice(0, i);
  }
  return common.at(-1) ?? (paths.length === 1 ? (split(paths[0] ?? '').at(-1) ?? '') : '/');
}
