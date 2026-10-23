import { inject, Injectable } from '@angular/core';
import { DialogService } from '../../ui/dialog.service';

/** Opens the diagnostics collector ("Collect diagnostics bundle…"). The dialog is a lazy chunk, loaded on first use (rule 35). */
@Injectable({ providedIn: 'root' })
export class CollectorDialogService {
  private readonly dialogs = inject(DialogService);
  async open(): Promise<void> {
    const { CollectorDialog } = await import('./collector-dialog');
    this.dialogs.open(CollectorDialog, { title: $localize`:@@tools.collector.title:Collect diagnostics bundle`, width: 760 });
  }
}
