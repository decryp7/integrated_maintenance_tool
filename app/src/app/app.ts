import { RovingTabindex } from './shared/roving-tabindex';
import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import { WorkspaceStore } from './core/workspace.store';
import { ActivityBar } from './shell/activity-bar';
import { Sidebar } from './shell/sidebar';
import { StatusBar } from './shell/status-bar';
import { TitleBar } from './shell/title-bar';
import { DockHost } from './dock/dock-host';
import { CommandPalette } from './palette/command-palette';
import { FilterBar } from './workspace/filter-bar';
import { ContextMenuHost } from './ui/context-menu-host';
import { ModalGate } from './dialog-windows/modal-gate';
import { DialogHost } from './ui/dialog-host';
import { Welcome, WelcomeService } from './welcome/welcome';
import { LifecycleBanners } from './lifecycle/lifecycle-banners';
import { ImportToast, PartialImportBanner } from './import/import-ui';
import { TourOverlay } from './onboarding/tour-overlay';
import { NarrowWindow } from './shell/narrow-window';
import { RegionNavigator } from './shell/regions';

@Component({
  selector: 'debrief-root',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [RovingTabindex, TitleBar, LifecycleBanners, ActivityBar, Sidebar, StatusBar, DockHost, FilterBar, CommandPalette, DialogHost, ContextMenuHost, Welcome, ImportToast, PartialImportBanner, TourOverlay],
  template: `
    <!-- The first Tab stop: jumps over the activity bar, side bar and filter bar to the events grid (visible on focus). -->
    <button type="button" class="skip-link" (click)="regions.events()" i18n="@@app.skipToEvents">Skip to events</button>
    <debrief-title-bar />
    <debrief-lifecycle-banners />
    @defer (on idle) {
      <debrief-partial-import-banner />
    }
    <div class="body">
      <!-- The regions F6 cycles through (shell/regions.ts): each a named landmark that takes focus. -->
      <debrief-activity-bar debriefRoving=".act" rovingAxis="vertical" data-debrief-region="activity" role="navigation" aria-label="Activity bar" i18n-aria-label="@@app.region.activity" tabindex="-1" />
      <!-- The empty Explorer says nothing the welcome page does not: it appears with the first data. -->
      @if (store.sidebarOpen() && !(welcome.visible() && !hasData())) {
        @defer (on immediate) {
          <debrief-sidebar data-debrief-region="sidebar" role="complementary" aria-label="Side bar" i18n-aria-label="@@app.region.sidebar" tabindex="-1" />
        } @placeholder {
          <!-- Its room until its code arrives: the dock does not move when it appears. -->
          <div class="sidebar-placeholder" data-testid="sidebar-placeholder" aria-hidden="true"></div>
        }
      }
      <main class="center">
        <!-- Nothing to filter until data is open: the welcome page is the whole first screen. -->
        @if (hasData()) {
          @defer (on immediate) {
            <debrief-filter-bar data-debrief-region="filters" role="search" aria-label="Filters" i18n-aria-label="@@app.region.filters" tabindex="-1" />
          } @placeholder {
            <div class="filter-bar-placeholder" data-testid="filter-bar-placeholder" aria-hidden="true"></div>
          }
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
    @if (modalGate.blocked()) {
      <!-- A modal dialog window is open over this one: dim and block it (ADR 0050). -->
      <div class="modal-scrim" data-debrief-overlay data-testid="modal-scrim" aria-hidden="true"></div>
    }
    <!-- Opened by the user or by chrome that appears after the first paint: their code arrives when the browser is idle (Paddock rule 35). -->
    @defer (on idle) {
      <debrief-command-palette />
    }
    <debrief-dialog-host />
    <debrief-context-menu-host />
    @defer (on idle) {
      <debrief-import-toast />
      <debrief-tour-overlay />
    }

    @if (store.dropHover()) {
      <div class="drop" data-debrief-overlay i18n="@@app.drop">Drop files or folders to add them to the workspace</div>
    }
  `,
  styles: `
    .skip-link {
      position: fixed; top: 6px; left: 6px; z-index: 300; padding: 6px 12px; border-radius: 6px;
      border: 1px solid var(--accent); background: var(--bg-1); color: var(--text); font: inherit;
      transform: translateY(-200%);
    }
    .skip-link:focus-visible { transform: none; }
    .modal-scrim { position: fixed; inset: 0; z-index: 205; background: var(--scrim); }
    :host { display: flex; flex-direction: column; height: 100vh; min-width: 600px; background: var(--bg-1); }
    .body { flex: 1; min-height: 0; display: flex; }
    .center { flex: 1; min-width: 0; display: flex; flex-direction: column; }
    /* The deferred side bar's and filter bar's room (their host sizes: shell/sidebar.ts, workspace/filter-bar.ts). */
    .sidebar-placeholder { width: 272px; flex: none; background: var(--bg-2); border-inline-end: 1px solid var(--border); }
    .filter-bar-placeholder { flex: none; height: 30px; padding: 10px 14px; border-bottom: 1px solid var(--border); background: var(--bg-1); }
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
  protected readonly modalGate = inject(ModalGate);
  protected readonly regions = inject(RegionNavigator);
  protected readonly hasData = computed(() => (this.store.summary()?.sources.length ?? 0) > 0);

  constructor() {
    inject(NarrowWindow).start();
  }
}
