import { ApplicationRef, ChangeDetectionStrategy, Component, computed, DestroyRef, EnvironmentInjector, inject } from '@angular/core';
import { mountInToastStack } from './ui/toast-stack';
import { WorkspaceToasts } from './shell/workspace-toasts';
import { WorkspaceStore } from './core/workspace.store';
import { ActivityBar } from './shell/activity-bar';
import { Sidebar } from './shell/sidebar';
import { StatusBar } from './shell/status-bar';
import { TitleBar } from './shell/title-bar';
import { DockHost } from './dock/dock-host';
import { CommandPalette } from './palette/command-palette';
import { FilterBar } from './workspace/filter-bar';
import { ContextMenuHost } from './ui/context-menu-host';
import { DialogHost } from './ui/dialog-host';
import { Welcome, WelcomeService } from './welcome/welcome';
import { LifecycleBanners } from './lifecycle/lifecycle-banners';
import { ImportToast, PartialImportBanner } from './import/import-ui';
import { TourOverlay } from './onboarding/tour-overlay';
import { NarrowWindow } from './shell/narrow-window';

@Component({
  selector: 'debrief-root',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [TitleBar, LifecycleBanners, ActivityBar, Sidebar, StatusBar, DockHost, FilterBar, CommandPalette, DialogHost, ContextMenuHost, Welcome, ImportToast, PartialImportBanner, TourOverlay],
  template: `
    <debrief-title-bar />
    <debrief-lifecycle-banners />
    <debrief-partial-import-banner />
    <div class="body">
      <!-- The regions F6 cycles through (shell/regions.ts): each a named landmark that takes focus. -->
      <debrief-activity-bar data-debrief-region="activity" role="navigation" aria-label="Activity bar" i18n-aria-label="@@app.region.activity" tabindex="-1" />
      <!-- The empty Explorer says nothing the welcome page does not: it appears with the first data. -->
      @if (store.sidebarOpen() && !(welcome.visible() && !hasData())) {
        <debrief-sidebar data-debrief-region="sidebar" role="complementary" aria-label="Side bar" i18n-aria-label="@@app.region.sidebar" tabindex="-1" />
      }
      <main class="center">
        <!-- Nothing to filter until data is open: the welcome page is the whole first screen. -->
        @if (hasData()) {
          <debrief-filter-bar data-debrief-region="filters" role="search" aria-label="Filters" i18n-aria-label="@@app.region.filters" tabindex="-1" />
        }
        <div class="stage">
          <debrief-dock-host />
          @if (welcome.visible()) {
            <debrief-welcome />
          }
        </div>
      </main>
    </div>
    <debrief-status-bar data-debrief-region="status" role="region" aria-label="Status bar" i18n-aria-label="@@app.region.status" tabindex="-1" />
    <debrief-command-palette />
    <debrief-dialog-host />
    <debrief-context-menu-host />
    <debrief-import-toast />
    <debrief-tour-overlay />

    @if (store.dropHover()) {
      <div class="drop" data-debrief-overlay i18n="@@app.drop">Drop files or folders to add them to the workspace</div>
    }
  `,
  styles: `
    :host { display: flex; flex-direction: column; height: 100vh; min-width: 600px; background: var(--bg-1); }
    .body { flex: 1; min-height: 0; display: flex; }
    .center { flex: 1; min-width: 0; display: flex; flex-direction: column; }
    .stage { flex: 1; min-height: 0; position: relative; display: flex; flex-direction: column; }
    /* dockview gives its sashes and floating groups high z-indexes; keep them inside the dock so the
       welcome screen (and anything else stacked over the dock) covers them. */
    .stage > debrief-dock-host { isolation: isolate; }
    .drop {
      position: fixed; inset: 12px; z-index: 90; border: 2px dashed var(--accent); border-radius: 12px;
      background: var(--accent-soft); display: flex; align-items: center; justify-content: center;
      font-size: var(--fs-lg); color: var(--text); pointer-events: none;
    }
  `,
})
export class App {
  protected readonly store = inject(WorkspaceStore);
  protected readonly welcome = inject(WelcomeService);
  protected readonly hasData = computed(() => (this.store.summary()?.sources.length ?? 0) > 0);

  constructor() {
    inject(NarrowWindow).start();
    const toasts = mountInToastStack(WorkspaceToasts, 'debrief-workspace-toasts', inject(EnvironmentInjector), inject(ApplicationRef));
    inject(DestroyRef).onDestroy(() => toasts.destroy());
  }
}
