import { ApplicationRef, createComponent, effect, EnvironmentInjector, EnvironmentProviders, inject, Injectable, makeEnvironmentProviders, provideAppInitializer, untracked } from '@angular/core';
import { DockService } from '../dock/dock.service';
import { IMT_ACTIVITY_ITEMS, IMT_COMMANDS, IMT_VIEWS, ImtCommand, ImtView } from '../extensions/tokens';
import { UI_STATE_CONTRIBUTIONS } from '../lifecycle/ui-state';
import { DialogService } from '../ui/dialog.service';
import { LiveTail } from '../workspace/live-tail';
import { AlertToasts } from './alert-toasts';
import { AlertsStore } from './alerts.store';
import { LiveAlerts } from './live-alerts';
import { LiveAutoOpen } from './live-auto-open';
import { liveActivityItems } from './live-activity';
import { LiveFeed } from './live-feed';
import { LiveFollow } from './live-follow.store';
import { LiveUiState } from './live-ui-state';

export const ALERTS_VIEW = 'live-alerts';
export const TAIL_STATUS_VIEW = 'tail-status';

export const LIVE_VIEWS: ImtView[] = [
  {
    id: ALERTS_VIEW,
    title: $localize`:@@live.views.alerts:Alerts`,
    category: 'Live',
    icon: 'alert',
    defaultPosition: 'bottom',
    component: () => import('./alerts-view').then((m) => m.AlertsView),
  },
  {
    id: TAIL_STATUS_VIEW,
    title: $localize`:@@live.views.tailStatus:Tail status`,
    category: 'Live',
    icon: 'refresh',
    defaultPosition: 'bottom',
    component: () => import('./tail-status-view').then((m) => m.TailStatusView),
  },
];

/** Title of the alerts tab: "Alerts", or "Alerts (3)" with the number of alerts listed. */
export function alertsTitle(count: number): string {
  return count ? $localize`:@@live.views.alertsCount:Alerts (${count > 999 ? '999+' : count}:count:)` : $localize`:@@live.views.alerts:Alerts`;
}

@Injectable({ providedIn: 'root' })
export class LiveCommands {
  private readonly live = inject(LiveTail);
  private readonly follow = inject(LiveFollow);
  private readonly dialogs = inject(DialogService);

  readonly commands: ImtCommand[] = [
    { id: 'live.toggle', title: $localize`:@@live.commands.toggle:Toggle live tail`, category: $localize`:@@live.commands.categoryFile:File`, when: () => this.live.available(), run: () => this.live.toggle() },
    {
      id: 'live.open',
      title: $localize`:@@live.commands.open:Open for live tail…`,
      category: $localize`:@@live.commands.categoryFile:File`,
      run: async () => {
        const { OpenLiveDialog } = await import('./open-live-dialog');
        this.dialogs.open(OpenLiveDialog, { title: $localize`:@@live.commands.openTitle:Open for live tail`, width: 480 });
      },
    },
    { id: 'live.resume', title: $localize`:@@live.commands.resume:Resume following`, category: $localize`:@@live.commands.categoryAnalyze:Analyze`, when: () => this.live.enabled(), run: () => this.follow.resume() },
    { id: 'live.alerts', title: $localize`:@@live.commands.showAlerts:Show live alerts`, category: $localize`:@@live.commands.categoryView:View`, run: (ctx) => ctx.showView(ALERTS_VIEW) },
    { id: 'live.status', title: $localize`:@@live.commands.showStatus:Show tail status`, category: $localize`:@@live.commands.categoryView:View`, run: (ctx) => ctx.showView(TAIL_STATUS_VIEW) },
    {
      id: 'live.rules',
      title: $localize`:@@live.commands.rules:Edit live alert rules…`,
      category: $localize`:@@live.commands.categoryAnalyze:Analyze`,
      run: async () => {
        const { AlertRulesEditor } = await import('./alert-rules-editor');
        this.dialogs.open(AlertRulesEditor, { title: $localize`:@@live.alertsView.dialogTitle:Live alert rules`, width: 760 });
      },
    },
  ];
}

/** Mounts the alert toasts, starts listening to alerts and refreshing on changes, and keeps the alerts tab count. */
export function startLive(): void {
  const injector = inject(EnvironmentInjector);
  const host = document.createElement('imt-alert-toasts');
  document.body.append(host);
  const ref = createComponent(AlertToasts, { environmentInjector: injector, hostElement: host });
  inject(ApplicationRef).attachView(ref.hostView);
  inject(LiveFeed);
  inject(LiveAutoOpen);
  inject(LiveFollow);
  void inject(LiveAlerts).start();
  const alerts = inject(AlertsStore);
  const dock = inject(DockService);
  effect(
    () => {
      const title = alertsTitle(alerts.alerts().length);
      if (!dock.openIds().has(ALERTS_VIEW)) return;
      untracked(() => dock.setTitle(ALERTS_VIEW, title));
    },
    { injector },
  );
}

/**
 * Real-time features on top of live tail: follow now and the live window, coalesced incremental refresh,
 * alerts (toasts, panel, rules editor), the tail status panel, and the saved live settings. Put it AFTER
 * `provideImtBuiltins()` in `app.config.ts`.
 */
export function provideLive(): EnvironmentProviders {
  return makeEnvironmentProviders([
    { provide: IMT_VIEWS, multi: true, useValue: LIVE_VIEWS },
    { provide: IMT_COMMANDS, multi: true, useFactory: () => inject(LiveCommands).commands },
    { provide: IMT_ACTIVITY_ITEMS, multi: true, useFactory: () => liveActivityItems(inject(LiveTail), inject(LiveFollow)) },
    { provide: UI_STATE_CONTRIBUTIONS, multi: true, useFactory: () => inject(LiveUiState).contributions },
    provideAppInitializer(() => startLive()),
  ]);
}
