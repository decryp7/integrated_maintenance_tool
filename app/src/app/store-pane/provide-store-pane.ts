import { EnvironmentProviders, inject, makeEnvironmentProviders } from '@angular/core';
import { IMT_COMMANDS, IMT_VIEWS, ImtCommand, ImtView } from '../extensions/tokens';
import { STORE_VIEW } from './store-pane-model';
import { StorePaneStore } from './store-pane.store';

export function storePaneViews(): ImtView[] {
  return [
    {
      id: STORE_VIEW,
      title: $localize`:@@store.view.title:Store`,
      icon: 'metrics',
      category: 'core',
      defaultPosition: 'bottom',
      component: () => import('./store-pane').then((m) => m.StorePane),
    },
  ];
}

export function storePaneCommands(pane: InstanceType<typeof StorePaneStore>): ImtCommand[] {
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
    { provide: IMT_VIEWS, multi: true, useFactory: storePaneViews },
    { provide: IMT_COMMANDS, multi: true, useFactory: () => storePaneCommands(inject(StorePaneStore)) },
  ]);
}
