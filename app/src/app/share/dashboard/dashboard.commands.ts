import { inject, Injectable } from '@angular/core';
import { TileCfg } from '../../core/api.types';
import { ImtCommand } from '../../extensions/tokens';
import { DialogService } from '../../ui/dialog.service';
import { AddTileDialog } from './add-tile-dialog';
import { DashboardStore } from './dashboard.store';

/** Commands for dashboards. */
@Injectable({ providedIn: 'root' })
export class DashboardCommands {
  private readonly ds = inject(DashboardStore);
  private readonly dialogs = inject(DialogService);

  readonly commands: ImtCommand[] = [
    { id: 'dashboards.show', title: 'Show dashboards', category: 'View', run: (ctx) => ctx.showView('dashboards') },
    {
      id: 'dashboards.edit',
      title: 'Dashboard: toggle edit mode',
      category: 'View',
      when: () => !!this.ds.current(),
      run: (ctx) => {
        ctx.showView('dashboards');
        if (this.ds.editing()) this.ds.cancel();
        else this.ds.startEdit();
      },
    },
    {
      id: 'dashboards.addTile',
      title: 'Dashboard: add tile…',
      category: 'View',
      when: () => this.ds.editing(),
      run: async () => {
        const tile = await this.dialogs.open<TileCfg>(AddTileDialog, { title: 'Add tile', width: 520 }).closed;
        if (tile) this.ds.add(tile);
      },
    },
    {
      id: 'dashboards.wall',
      title: 'Dashboard: wall mode',
      category: 'View',
      when: () => !!this.ds.current(),
      run: (ctx) => {
        ctx.showView('dashboards');
        this.ds.wall.set(!this.ds.wall());
      },
    },
  ];
}
