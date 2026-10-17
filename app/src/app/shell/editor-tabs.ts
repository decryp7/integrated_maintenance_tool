import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { EditorTab, WorkspaceStore } from '../core/workspace.store';
import { Icon } from '../shared/icon';

@Component({
  selector: 'imt-editor-tabs',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  template: `
    <div role="tablist" class="tabs">
      @for (t of tabs; track t.id) {
        <button role="tab" class="tab" [class.on]="store.activeTab() === t.id" [attr.aria-selected]="store.activeTab() === t.id" (click)="store.setTab(t.id)">
          <imt-icon [name]="t.icon" [size]="13" [stroke]="2" />
          <span>{{ t.label }}</span>
        </button>
      }
    </div>
  `,
  styles: `
    :host { height: 36px; flex: none; display: flex; align-items: flex-end; background: var(--bg-0); border-bottom: 1px solid var(--border); }
    .tabs { display: flex; height: 100%; }
    .tab { height: 36px; display: flex; align-items: center; gap: 8px; padding: 0 14px; border: 0; border-right: 1px solid var(--border); background: transparent; color: var(--muted); cursor: pointer; }
    .tab:hover { color: var(--text); }
    .tab.on { background: var(--bg-1); color: var(--text); box-shadow: inset 0 2px 0 var(--accent); }
  `,
})
export class EditorTabs {
  protected readonly store = inject(WorkspaceStore);
  protected readonly tabs: { id: EditorTab; label: string; icon: string }[] = [
    { id: 'timeline', label: 'Timeline · All sources', icon: 'timeline' },
    { id: 'metrics', label: 'Metrics', icon: 'metrics' },
    { id: 'profile', label: 'Profile', icon: 'profile' },
  ];
}
