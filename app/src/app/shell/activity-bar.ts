import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { ClickOutside } from '../shared/click-outside';
import { Icon } from '../shared/icon';

@Component({
  selector: 'imt-activity-bar',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, ClickOutside],
  template: `
    <button class="act" [class.on]="store.sidebarOpen()" aria-label="Explorer" title="Explorer" (click)="store.toggleSidebar()"><imt-icon name="files" [size]="18" /></button>
    <button class="act" aria-label="Search" title="Search (/)" (click)="store.focusSearch()"><imt-icon name="search" [size]="18" /></button>
    <button class="act" [class.on]="store.activeTab() === 'timeline'" aria-label="Timeline" title="Timeline" (click)="store.setTab('timeline')"><imt-icon name="timeline" [size]="18" [stroke]="1.9" /></button>
    <button class="act" [class.on]="store.activeTab() === 'metrics'" aria-label="Metrics" title="Metrics" (click)="store.setTab('metrics')"><imt-icon name="metrics" [size]="18" /></button>
    <button class="act" [class.on]="store.activeTab() === 'profile'" aria-label="Profile editor" title="Profile editor" (click)="store.setTab('profile')"><imt-icon name="profile" [size]="18" /></button>
    <div class="spacer"></div>
    <div class="settings" (imtClickOutside)="open.set(false)">
      <button class="act" aria-label="Settings" title="Settings" [attr.aria-expanded]="open()" (click)="open.set(!open())"><imt-icon name="settings" [size]="18" /></button>
      @if (open()) {
        <div class="popover panel" role="dialog" aria-label="Settings">
          <div class="section-title">Theme</div>
          <div class="seg">
            @for (t of themes; track t) {
              <button class="btn" [class.active]="store.theme() === t" (click)="store.setTheme(t)">{{ t }}</button>
            }
          </div>
          <div class="section-title">Time zone</div>
          <div class="seg">
            <button class="btn" [class.active]="store.timezone() === 'utc'" (click)="store.setTimezone('utc')">UTC</button>
            <button class="btn" [class.active]="store.timezone() === 'local'" (click)="store.setTimezone('local')">Local</button>
          </div>
          <label class="row">
            <input type="checkbox" [checked]="store.inspectorOpen()" (change)="store.toggleInspector()" />
            Show event inspector
          </label>
        </div>
      }
    </div>
  `,
  styles: `
    :host { width: 48px; flex: none; display: flex; flex-direction: column; align-items: center; gap: 6px; padding: 8px 0; background: var(--bg-0); border-right: 1px solid var(--border); }
    .act { width: 36px; height: 36px; border-radius: 8px; border: 0; background: transparent; color: var(--faint); display: flex; align-items: center; justify-content: center; cursor: pointer; }
    .act:hover { color: var(--text); }
    .act.on { background: var(--bg-active); color: var(--text); box-shadow: inset 2px 0 0 var(--accent); }
    .spacer { flex: 1; }
    .settings { position: relative; }
    .panel { left: 44px; bottom: 0; width: 240px; display: flex; flex-direction: column; gap: 8px; }
    .seg { display: flex; gap: 4px; }
    .seg .btn { text-transform: capitalize; }
    .row { display: flex; align-items: center; gap: 8px; color: var(--text-2); }
  `,
})
export class ActivityBar {
  protected readonly store = inject(WorkspaceStore);
  protected readonly open = signal(false);
  protected readonly themes = ['system', 'dark', 'light'] as const;
}
