import { inject, Injectable } from '@angular/core';
import { DebriefCommand } from '../../extensions/tokens';
import { DialogService } from '../../ui/dialog.service';
import { DashboardStore } from './dashboard.store';

/** Commands for dashboards. */
@Injectable({ providedIn: 'root' })
export class DashboardCommands {
  private readonly ds = inject(DashboardStore);
  private readonly dialogs = inject(DialogService);

  readonly commands: DebriefCommand[] = [
    { id: 'dashboards.show', title: $localize`:@@share.dashboardCommands.show:Show dashboards`, category: $localize`:@@share.commands.categoryView:View`, view: 'dashboards', run: (ctx) => ctx.showView('dashboards') },
    {
      id: 'dashboards.edit',
      title: $localize`:@@share.dashboardCommands.edit:Dashboard: toggle edit mode`,
      category: $localize`:@@share.commands.categoryView:View`,
      when: () => !!this.ds.current(),
      run: (ctx) => {
        ctx.showView('dashboards');
        if (this.ds.editing()) this.ds.cancel();
        else this.ds.startEdit();
      },
    },
    {
      id: 'dashboards.addTile',
      title: $localize`:@@share.dashboardCommands.addTile:Dashboard: add tile…`,
      category: $localize`:@@share.commands.categoryView:View`,
      when: () => this.ds.editing(),
      run: async () => {
        const tile = await this.dialogs.openLazy(() => import('./add-tile-dialog').then((m) => m.AddTileDialog), { title: $localize`:@@share.addTile.dialogTitle:Add tile`, width: 520 }).closed;
        if (tile) this.ds.add(tile);
      },
    },
    {
      id: 'dashboards.wall',
      title: $localize`:@@share.dashboardCommands.wall:Dashboard: wall mode`,
      category: $localize`:@@share.commands.categoryView:View`,
      when: () => !!this.ds.current(),
      run: (ctx) => {
        ctx.showView('dashboards');
        this.ds.wall.set(!this.ds.wall());
      },
    },
  ];
}
