import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { DockService } from '../dock/dock.service';
import { WorkspaceStore } from '../core/workspace.store';
import { SettingsDialogService } from '../settings/settings-dialog';
import { Icon } from '../shared/icon';

@Component({
  selector: 'imt-activity-bar',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  template: `
    <button class="act" [class.on]="store.sidebarOpen()" aria-label="Explorer" title="Explorer" (click)="store.toggleSidebar()"><imt-icon name="files" [size]="18" /></button>
    <button class="act" aria-label="Search" title="Search (/)" (click)="store.focusSearch()"><imt-icon name="search" [size]="18" /></button>
    <button class="act" [class.on]="dock.isOpen('timeline')" aria-label="Timeline" title="Show Timeline panel" (click)="dock.show('timeline')"><imt-icon name="timeline" [size]="18" [stroke]="1.9" /></button>
    <button class="act" [class.on]="dock.isOpen('metrics')" aria-label="Metrics" title="Show Metrics panel" (click)="dock.show('metrics')"><imt-icon name="metrics" [size]="18" /></button>
    <button class="act" [class.on]="dock.isOpen('profile')" aria-label="Profile editor" title="Show Profile editor panel" (click)="dock.show('profile')"><imt-icon name="profile" [size]="18" /></button>
    <div class="spacer"></div>
    <button class="act" aria-label="Settings" title="Settings" (click)="settings.open()"><imt-icon name="settings" [size]="18" /></button>
  `,
  styles: `
    :host { width: 48px; flex: none; display: flex; flex-direction: column; align-items: center; gap: 6px; padding: 8px 0; background: var(--bg-0); border-right: 1px solid var(--border); }
    .act { width: 36px; height: 36px; border-radius: 8px; border: 0; background: transparent; color: var(--faint); display: flex; align-items: center; justify-content: center; cursor: pointer; }
    .act:hover { color: var(--text); }
    .act.on { background: var(--bg-active); color: var(--text); box-shadow: inset 2px 0 0 var(--accent); }
    .spacer { flex: 1; }
  `,
})
export class ActivityBar {
  protected readonly store = inject(WorkspaceStore);
  protected readonly dock = inject(DockService);
  protected readonly settings = inject(SettingsDialogService);
}
