import { inject, Injectable } from '@angular/core';
import { DialogService } from '../ui/dialog.service';

/** Opens the About dialog (Help → About Debrief, the "About Debrief" palette command). */
@Injectable({ providedIn: 'root' })
export class AboutService {
  private readonly dialogs = inject(DialogService);

  open(): void {
    this.dialogs.openLazy(() => import('./about-dialog').then((m) => m.AboutDialog), { title: $localize`:@@about.title:About Debrief`, width: 720 });
  }
}
