import { EnvironmentProviders, inject, makeEnvironmentProviders } from '@angular/core';
import { DEBRIEF_COMMANDS, DEBRIEF_VIEWS, DebriefCommand, DebriefView } from '../extensions/tokens';
import { STORE_VIEW } from './store-pane-model';
import { StorePaneStore } from './store-pane.store';

export function storePaneViews(): DebriefView[] {
  return [
    {
      id: STORE_VIEW,
      title: $localize`:@@store.view.title:Store`,
      icon: 'metrics',
      category: 'core',
      defaultPosition: 'bottom',
      area: 'status',
      component: () => import('./store-pane').then((m) => m.StorePane),
    },
  ];
}

export function storePaneCommands(pane: InstanceType<typeof StorePaneStore>): DebriefCommand[] {
  const category = $localize`:Command palette category@@store.category:View`;
  return [
    {
      id: 'store.show',
      title: $localize`:@@store.cmd.show:Open Store`,
      category,
      run: (ctx) => {
        void pane.show(pane.tab() === 'sql' ? 'overview' : pane.tab());
        ctx.showView(STORE_VIEW);
      },
    },
    {
      id: 'store.sql',
      title: $localize`:@@store.cmd.sql:Open SQL console`,
      category,
      run: (ctx) => {
        void pane.show('sql');
        ctx.showView(STORE_VIEW);
      },
    },
  ];
}

/** One line in `app.config.ts`: View ▸ Store (the Store pane and its SQL console) and its palette commands. */
export function provideStorePane(): EnvironmentProviders {
  return makeEnvironmentProviders([
    { provide: DEBRIEF_VIEWS, multi: true, useFactory: storePaneViews },
    { provide: DEBRIEF_COMMANDS, multi: true, useFactory: () => storePaneCommands(inject(StorePaneStore)) },
  ]);
}
