import { inject, Injectable } from '@angular/core';
import { ImtCommand } from '../../extensions/tokens';
import { DialogService } from '../../ui/dialog.service';

/** Edit ▸ "Paste as log…" (Mod+Shift+V). */
@Injectable({ providedIn: 'root' })
export class PasteCommands {
  private readonly dialogs = inject(DialogService);

  /** Opens the dialog, filled from `text` (a drop) or else from the clipboard. */
  async open(text: string | null = null): Promise<void> {
    const { PasteDialog } = await import('./paste-dialog');
    this.dialogs.open(PasteDialog, { title: $localize`:@@sources.paste.dialog.title:Paste as log`, inputs: { text }, width: 640 });
  }

  readonly commands: ImtCommand[] = [
    {
      id: 'edit.pasteLog',
      title: $localize`:@@sources.paste.command:Paste as log…`,
      category: $localize`:@@shell.edit.category:Edit`,
      keybinding: 'Mod+Shift+V',
      run: () => this.open(),
    },
  ];
}
