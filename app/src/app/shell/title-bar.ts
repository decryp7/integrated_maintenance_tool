import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { ClickOutside } from '../shared/click-outside';
import { Icon } from '../shared/icon';
import { DockService } from '../dock/dock.service';
import { CommandRegistry } from '../extensions/command-registry';
import { formatKeybinding } from '../extensions/keybinding';
import { ViewRegistry } from '../extensions/view-registry';
import { PaletteService } from '../palette/palette.service';
import { TitleExtras } from '../lifecycle/title-extras';
import { WindowingService } from '../windowing/windowing.service';

interface MenuItem {
  label: string;
  hint?: string;
  /** Present for toggles (panels): the menu item is a checkbox. */
  checked?: boolean;
  run: () => void;
}

interface Menu {
  label: string;
  items: MenuItem[];
}

@Component({
  selector: 'imt-title-bar',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, ClickOutside, TitleExtras],
  template: `
    <div class="brand">
      <span class="logo" aria-hidden="true">
        <svg width="12" height="12" viewBox="0 0 12 12"><path d="M1 9 L4 5 L7 7 L11 2" stroke="currentColor" stroke-width="1.8" fill="none" stroke-linecap="round" stroke-linejoin="round" /></svg>
      </span>
      <span class="name">IMT</span>
    </div>
    <nav class="menus" (imtClickOutside)="open.set(null)">
      @for (menu of menus(); track menu.label) {
        <div class="menu">
          <button class="menu-btn" [class.on]="open() === menu.label" (click)="toggle(menu.label)" [attr.aria-expanded]="open() === menu.label">{{ menu.label }}</button>
          @if (open() === menu.label) {
            <div class="popover dropdown" role="menu">
              @for (item of menu.items; track item.label) {
                <button
                  [attr.role]="item.checked === undefined ? 'menuitem' : 'menuitemcheckbox'"
                  [attr.aria-checked]="item.checked === undefined ? null : item.checked"
                  class="item"
                  (click)="item.run(); open.set(null)"
                >
                  <span>{{ item.checked === undefined ? '' : item.checked ? '✓ ' : '\u2003' }}{{ item.label }}</span>
                  @if (item.hint) { <span class="faint">{{ item.hint }}</span> }
                </button>
              }
            </div>
          }
        </div>
      }
    </nav>
    <div class="palette-wrap">
      <button class="palette" (click)="openPalette()" title="Command palette (Ctrl/⌘+K)">
        <imt-icon name="search" [size]="13" />
        <span class="ellipsis">{{ store.summary()?.profileName ?? 'IMT' }} — type a command or press Ctrl/⌘+K</span>
        <span class="mono faint kbd">⌘K</span>
      </button>
    </div>
    <div class="state">
      @if (store.busy(); as busy) {
        <span class="spinner" aria-hidden="true"></span><span>{{ busy }}</span>
      } @else if (store.summary(); as s) {
        <span class="dot" style="background: var(--mk-start)"></span>
        <span>Indexed · {{ (s.loadMs / 1000).toFixed(1) }} s</span>
      }
    </div>
    <imt-title-extras />
  `,
  host: { '(document:keydown)': 'onKey($event)' },
  styles: `
    :host { height: 36px; flex: none; display: flex; align-items: center; gap: 16px; padding: 0 12px; background: var(--bg-0); border-bottom: 1px solid var(--border); }
    .brand { display: flex; align-items: center; gap: 8px; }
    .logo { width: 20px; height: 20px; border-radius: 5px; background: var(--accent); color: var(--accent-ink); display: flex; align-items: center; justify-content: center; }
    .name { font-weight: 600; letter-spacing: 0.04em; }
    .menus { display: flex; gap: 2px; }
    .menu { position: relative; }
    .menu-btn { height: 26px; padding: 0 8px; border: 0; border-radius: 5px; background: transparent; color: var(--muted); cursor: pointer; }
    .menu-btn:hover, .menu-btn.on { background: var(--bg-hover); color: var(--text); }
    .dropdown { top: 30px; left: 0; min-width: 220px; padding: 4px; display: flex; flex-direction: column; }
    .item { display: flex; justify-content: space-between; gap: 16px; height: 28px; align-items: center; padding: 0 10px; border: 0; border-radius: 5px; background: transparent; color: var(--text-2); cursor: pointer; text-align: left; }
    .item:hover { background: var(--bg-hover); color: var(--text); }
    .palette-wrap { flex: 1; display: flex; justify-content: center; min-width: 0; }
    .palette { width: 440px; max-width: 100%; height: 24px; display: flex; align-items: center; gap: 8px; padding: 0 10px; border-radius: 6px; background: var(--bg-3); border: 1px solid var(--border-2); color: var(--faint); font-size: 12px; cursor: text; }
    .palette span:nth-child(2) { flex: 1; text-align: left; }
    .kbd { font-size: 11px; }
    .state { display: flex; align-items: center; gap: 8px; color: var(--faint); font-size: 12px; white-space: nowrap; }
    .spinner { width: 10px; height: 10px; border-radius: 50%; border: 2px solid var(--border-3); border-top-color: var(--accent); animation: spin 0.8s linear infinite; }
    @keyframes spin { to { transform: rotate(360deg); } }
  `,
})
export class TitleBar {
  protected readonly store = inject(WorkspaceStore);
  private readonly registry = inject(CommandRegistry);
  private readonly dock = inject(DockService);
  private readonly views = inject(ViewRegistry);
  private readonly palette = inject(PaletteService);
  private readonly windowing = inject(WindowingService);
  protected readonly open = signal<string | null>(null);

  protected readonly menus = computed<Menu[]>(() => {
    // Re-evaluate when panels open/close or the workspace changes.
    this.dock.openIds();
    const cmd = (id: string): MenuItem[] => {
      const c = this.registry.find(id);
      const available = this.registry.available().some((a) => a.id === id);
      return c && available ? [{ label: c.title, hint: c.keybinding ? formatKeybinding(c.keybinding) : undefined, run: () => void this.registry.run(id) }] : [];
    };
    const panels: MenuItem[] = this.views.available().map((v) => ({
      label: v.title,
      checked: this.dock.isOpen(v.id),
      run: () => this.dock.toggle(v.id),
    }));
    return [
      { label: 'File', items: ['window.new', 'file.openFiles', 'file.openFolder', 'file.reimport', 'file.export', 'file.close'].flatMap(cmd) },
      {
        label: 'View',
        items: [
          ...cmd('palette.open'),
          ...cmd('view.toggleSidebar'),
          ...panels,
          ...cmd('view.resetLayout'),
          ...cmd('settings.open'),
          ...['view.theme.system', 'view.theme.dark', 'view.theme.light'].flatMap(cmd),
        ],
      },
      {
        label: 'Analyze',
        items: ['analyze.search', 'analyze.markedOnly', 'analyze.fit', 'analyze.back', 'analyze.forward', 'analyze.clearFilters', 'analyze.utc', 'analyze.local'].flatMap(cmd),
      },
      {
        label: 'Window',
        items: [
          ...['window.new', 'window.close'].flatMap(cmd),
          ...this.windowing.windows().map((w) => ({ label: w.title || w.label, checked: w.label === this.windowing.label(), run: () => void this.windowing.focus(w.label) })),
        ],
      },
      { label: 'Help', items: [...cmd('help.welcome'), ...cmd('help.about'), { label: 'IMT 0.1.0 — Integrated Maintenance Tool', run: () => undefined }] },
    ];
  });

  protected toggle(label: string): void {
    this.open.update((o) => (o === label ? null : label));
  }

  protected openPalette(): void {
    this.palette.open();
  }

  protected onKey(e: KeyboardEvent): void {
    const target = e.target as HTMLElement | null;
    const typing = !!target && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable);
    if (e.key === '/' && !typing) {
      e.preventDefault();
      this.store.focusSearch();
      return;
    }
    this.registry.runForKey(e);
  }
}
