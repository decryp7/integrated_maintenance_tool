import { inject, Injectable } from '@angular/core';
import { ImtCommand, ImtView } from '../extensions/tokens';
import { ErrorSnapshotService } from './error-snapshot.service';

/** The view that lists the saved error snapshots. */
export const DIAGNOSTICS_VIEWS: ImtView[] = [
  {
    id: 'error-snapshots',
    title: 'Error snapshots',
    icon: 'file',
    defaultPosition: 'center',
    component: () => import('./error-snapshots-view').then((m) => m.ErrorSnapshotsView),
  },
];

/** Keyboard shortcut of the manual capture. */
export const CAPTURE_KEYBINDING = 'Mod+Shift+F12';

@Injectable({ providedIn: 'root' })
export class DiagnosticsCommands {
  private readonly snapshots = inject(ErrorSnapshotService);

  readonly commands: ImtCommand[] = [
    {
      id: 'diagnostics.capture',
      title: 'Capture diagnostic screenshot',
      category: 'Tools',
      keybinding: CAPTURE_KEYBINDING,
      run: async () => void (await this.snapshots.manual()),
    },
    { id: 'diagnostics.snapshots.show', title: 'Show error snapshots', category: 'Tools', run: (ctx) => ctx.showView('error-snapshots') },
  ];
}
