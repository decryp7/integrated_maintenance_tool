import { EnvironmentProviders, inject, Injectable, makeEnvironmentProviders } from '@angular/core';
import { RecordActions } from '../context/record-actions';
import { DEBRIEF_COMMANDS, DebriefCommand } from '../extensions/tokens';
import { provideDebriefPlugin } from '../extensions/provide';
import { WorkspaceStore } from '../core/workspace.store';

/** Palette commands of the markers feature. */
@Injectable({ providedIn: 'root' })
export class MarkersCommands {
  private readonly store = inject(WorkspaceStore);
  private readonly actions = inject(RecordActions);

  readonly commands: DebriefCommand[] = [
    { id: 'markers.manage', title: $localize`:@@markers.cmd.manage.title:Manage markers`, category: $localize`:Command palette category|@@markers.cmd.category:Markers`, run: (ctx) => ctx.showView('markers') },
    {
      id: 'markers.highlight',
      title: $localize`:@@markers.cmd.highlight.title:Highlight lines…`,
      category: $localize`:Command palette category|@@markers.cmd.category:Markers`,
      run: (ctx) => {
        const record = this.store.selected();
        if (!record) return ctx.notifyError($localize`:@@markers.cmd.highlight.selectFirst:Select an event in the grid first, then highlight lines like it.`);
        this.actions.highlight(record, document.getSelection()?.toString());
      },
    },
    {
      id: 'markers.infer',
      title: $localize`:@@markers.cmd.infer.title:Infer pattern from selected lines…`,
      category: $localize`:Command palette category|@@markers.cmd.category:Markers`,
      // Every line selected in the grid (shift/ctrl+click), else the event selected elsewhere.
      run: (ctx) => {
        if (!this.actions.inferSelected()) ctx.notifyError($localize`:@@markers.cmd.infer.selectFirst:Select one or more events in the grid first.`);
      },
    },
  ];
}

/** Registers the Markers view and its commands. One line in app.config.ts: `provideMarkers()`. */
export function provideMarkers(): EnvironmentProviders {
  return makeEnvironmentProviders([
    provideDebriefPlugin({
      views: [{ id: 'markers', category: 'Analysis', title: $localize`:View title|@@markers.view.title:Markers`, icon: 'alert', defaultPosition: 'center', area: 'analysis', component: () => import('./markers-manager').then((m) => m.MarkersManager) }],
    }),
    { provide: DEBRIEF_COMMANDS, multi: true, useFactory: () => inject(MarkersCommands).commands },
  ]);
}
