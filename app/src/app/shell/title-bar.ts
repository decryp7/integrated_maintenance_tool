import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { ClickOutside } from '../shared/click-outside';
import { Icon } from '../shared/icon';
import { TimelineZoom } from '../timeline/zoom';

interface MenuItem {
  label: string;
  hint?: string;
  run: () => void;
}

@Component({
  selector: 'imt-title-bar',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, ClickOutside],
  template: `
    <div class="brand">
      <span class="logo" aria-hidden="true">
        <svg width="12" height="12" viewBox="0 0 12 12"><path d="M1 9 L4 5 L7 7 L11 2" stroke="currentColor" stroke-width="1.8" fill="none" stroke-linecap="round" stroke-linejoin="round" /></svg>
      </span>
      <span class="name">IMT</span>
    </div>
    <nav class="menus" (imtClickOutside)="open.set(null)">
      @for (menu of menus; track menu.label) {
        <div class="menu">
          <button class="menu-btn" [class.on]="open() === menu.label" (click)="toggle(menu.label)" [attr.aria-expanded]="open() === menu.label">{{ menu.label }}</button>
          @if (open() === menu.label) {
            <div class="popover dropdown" role="menu">
              @for (item of menu.items; track item.label) {
                <button role="menuitem" class="item" (click)="item.run(); open.set(null)">
                  <span>{{ item.label }}</span>
                  @if (item.hint) { <span class="faint">{{ item.hint }}</span> }
                </button>
              }
            </div>
          }
        </div>
      }
    </nav>
    <div class="palette-wrap">
      <button class="palette" (click)="store.focusSearch()" title="Search events (/)">
        <imt-icon name="search" [size]="13" />
        <span class="ellipsis">{{ store.summary()?.profileName ?? 'IMT' }} — search events, fields, files</span>
        <span class="mono faint kbd">/</span>
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
  private readonly zoom = inject(TimelineZoom);
  protected readonly open = signal<string | null>(null);

  protected readonly menus: { label: string; items: MenuItem[] }[] = [
    {
      label: 'File',
      items: [
        { label: 'Open files…', run: () => void this.store.pickAndOpen(false) },
        { label: 'Open folder…', run: () => void this.store.pickAndOpen(true) },
        { label: 'Re-import all', run: () => void this.store.reload() },
        { label: 'Close workspace', run: () => void this.store.clearWorkspace() },
      ],
    },
    {
      label: 'View',
      items: [
        { label: 'Toggle sidebar', run: () => this.store.toggleSidebar() },
        { label: 'Toggle inspector', run: () => this.store.toggleInspector() },
        { label: 'Timeline', run: () => this.store.setTab('timeline') },
        { label: 'Metrics', run: () => this.store.setTab('metrics') },
        { label: 'Profile editor', run: () => this.store.setTab('profile') },
        { label: 'Theme: system', run: () => this.store.setTheme('system') },
        { label: 'Theme: dark', run: () => this.store.setTheme('dark') },
        { label: 'Theme: light', run: () => this.store.setTheme('light') },
      ],
    },
    {
      label: 'Analyze',
      items: [
        { label: 'Search events', hint: '/', run: () => this.store.focusSearch() },
        { label: 'Marked events only', run: () => this.store.setMarkedOnly(!this.store.markedOnly()) },
        { label: 'Fit time range', run: () => this.zoom.fit() },
        { label: 'Previous time range', hint: 'Alt+←', run: () => this.zoom.back() },
        { label: 'Next time range', hint: 'Alt+→', run: () => this.zoom.forward() },
        { label: 'Clear all filters', run: () => this.store.clearQuery() },
        { label: 'Time in UTC', run: () => this.store.setTimezone('utc') },
        { label: 'Time in local zone', run: () => this.store.setTimezone('local') },
      ],
    },
    {
      label: 'Help',
      items: [{ label: 'IMT 0.1.0 — Integrated Maintenance Tool', run: () => undefined }],
    },
  ];

  protected toggle(label: string): void {
    this.open.update((o) => (o === label ? null : label));
  }

  protected onKey(e: KeyboardEvent): void {
    const target = e.target as HTMLElement | null;
    const typing = !!target && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable);
    if (e.key === '/' && !typing) {
      e.preventDefault();
      this.store.focusSearch();
    }
  }
}
