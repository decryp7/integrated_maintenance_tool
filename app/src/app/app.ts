import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { WorkspaceStore } from './core/workspace.store';
import { ActivityBar } from './shell/activity-bar';
import { EditorTabs } from './shell/editor-tabs';
import { Sidebar } from './shell/sidebar';
import { StatusBar } from './shell/status-bar';
import { TitleBar } from './shell/title-bar';
import { Inspector } from './inspector/inspector';
import { MetricsView } from './metrics/metrics-view';
import { ProfileEditor } from './profile/profile-editor';
import { TimelineView } from './workspace/timeline-view';
import { Icon } from './shared/icon';

@Component({
  selector: 'app-root',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [TitleBar, ActivityBar, Sidebar, EditorTabs, StatusBar, Inspector, TimelineView, MetricsView, ProfileEditor, Icon],
  template: `
    <imt-title-bar />
    <div class="body">
      <imt-activity-bar />
      @if (store.sidebarOpen()) {
        <imt-sidebar />
      }
      <main class="center">
        <imt-editor-tabs />
        @switch (store.activeTab()) {
          @case ('timeline') { <imt-timeline-view /> }
          @case ('metrics') { <imt-metrics-view /> }
          @case ('profile') { <imt-profile-editor /> }
        }
      </main>
      @if (store.inspectorOpen() && store.activeTab() === 'timeline') {
        <imt-inspector />
      }
    </div>
    <imt-status-bar />

    @if (store.error(); as error) {
      <div class="toast" role="alert">
        <imt-icon name="alert" [size]="15" />
        <span class="ellipsis">{{ error }}</span>
        <button class="icon-btn" aria-label="Dismiss" (click)="store.dismissError()"><imt-icon name="close" [size]="14" /></button>
      </div>
    }
    @if (store.dropHover()) {
      <div class="drop">Drop files or folders to add them to the workspace</div>
    }
  `,
  styles: `
    :host { display: flex; flex-direction: column; height: 100vh; min-width: 1024px; background: var(--bg-1); }
    .body { flex: 1; min-height: 0; display: flex; }
    .center { flex: 1; min-width: 0; display: flex; flex-direction: column; }
    .toast {
      position: fixed; right: 16px; bottom: 36px; max-width: 520px; z-index: 100;
      display: flex; align-items: center; gap: 10px; padding: 8px 8px 8px 12px; border-radius: 8px;
      background: var(--tooltip-bg); border: 1px solid var(--lvl-error); color: var(--text); box-shadow: var(--shadow);
    }
    .toast imt-icon { color: var(--lvl-error); }
    .drop {
      position: fixed; inset: 12px; z-index: 90; border: 2px dashed var(--accent); border-radius: 12px;
      background: var(--accent-soft); display: flex; align-items: center; justify-content: center;
      font-size: 16px; color: var(--text); pointer-events: none;
    }
  `,
})
export class App {
  protected readonly store = inject(WorkspaceStore);
}
