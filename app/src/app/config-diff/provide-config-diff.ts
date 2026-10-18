import { EnvironmentProviders, makeEnvironmentProviders } from '@angular/core';
import { IMT_COMMANDS, IMT_VIEWS, ImtCommand, ImtView } from '../extensions/tokens';
import { provideConfigDiffBackend } from './config-diff-backend';

/** The configuration diff panel. Same extension point as plugin views. */
export const CONFIG_DIFF_VIEWS: ImtView[] = [
  { id: 'config-diff', category: 'Tools', title: $localize`:@@configDiff.provide.view.title:Configuration diff`, icon: 'columns', defaultPosition: 'center', component: () => import('./config-diff-view').then((m) => m.ConfigDiffView) },
];

export const CONFIG_DIFF_COMMANDS: ImtCommand[] = [
  { id: 'tools.configDiff.show', title: $localize`:@@configDiff.provide.command.compare:Compare configuration files…`, category: $localize`:@@configDiff.provide.command.category:Tools`, run: (ctx) => ctx.showView('config-diff') },
];

/** Registers the configuration diff (view, command, backend). One line in `app.config.ts`. */
export function provideConfigDiff(): EnvironmentProviders {
  return makeEnvironmentProviders([
    ...provideConfigDiffBackend(),
    { provide: IMT_VIEWS, multi: true, useValue: CONFIG_DIFF_VIEWS },
    { provide: IMT_COMMANDS, multi: true, useValue: CONFIG_DIFF_COMMANDS },
  ]);
}
