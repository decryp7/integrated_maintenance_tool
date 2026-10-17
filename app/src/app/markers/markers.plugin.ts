import { EnvironmentProviders, inject, Injectable, makeEnvironmentProviders } from '@angular/core';
import { RecordActions } from '../context/record-actions';
import { IMT_COMMANDS, ImtCommand } from '../extensions/tokens';
import { provideImtPlugin } from '../extensions/provide';
import { WorkspaceStore } from '../core/workspace.store';

/** Palette commands of the markers feature. */
@Injectable({ providedIn: 'root' })
export class MarkersCommands {
  private readonly store = inject(WorkspaceStore);
  private readonly actions = inject(RecordActions);

  readonly commands: ImtCommand[] = [
    { id: 'markers.manage', title: 'Manage markers', category: 'Markers', run: (ctx) => ctx.showView('markers') },
    {
      id: 'markers.highlight',
      title: 'Highlight lines…',
      category: 'Markers',
      run: (ctx) => {
        const record = this.store.selected();
        if (!record) return ctx.notifyError('Select an event in the grid first, then highlight lines like it.');
        this.actions.highlight(record, document.getSelection()?.toString());
      },
    },
    {
      id: 'markers.infer',
      title: 'Infer pattern from selected lines…',
      category: 'Markers',
      // Every line selected in the grid (shift/ctrl+click), else the event selected elsewhere.
      run: (ctx) => {
        if (!this.actions.inferSelected()) ctx.notifyError('Select one or more events in the grid first.');
      },
    },
  ];
}

/** Registers the Markers view and its commands. One line in app.config.ts: `provideMarkers()`. */
export function provideMarkers(): EnvironmentProviders {
  return makeEnvironmentProviders([
    provideImtPlugin({
      views: [{ id: 'markers', category: 'Analysis', title: 'Markers', icon: 'alert', defaultPosition: 'center', component: () => import('./markers-manager').then((m) => m.MarkersManager) }],
    }),
    { provide: IMT_COMMANDS, multi: true, useFactory: () => inject(MarkersCommands).commands },
  ]);
}
