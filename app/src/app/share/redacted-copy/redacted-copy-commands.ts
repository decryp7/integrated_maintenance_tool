import { inject, Injectable } from '@angular/core';
import { DebriefCommand } from '../../extensions/tokens';
import { DialogService } from '../../ui/dialog.service';

/** File ▸ "Share redacted copy…": a project copy with the sensitive values removed. */
@Injectable({ providedIn: 'root' })
export class RedactedCopyCommands {
  private readonly dialogs = inject(DialogService);

  async open(): Promise<void> {
    const { RedactedCopyDialog } = await import('./redacted-copy-dialog');
    this.dialogs.open(RedactedCopyDialog, { title: $localize`:@@share.redactedCopy.dialog.title:Share redacted copy`, width: 760 });
  }

  readonly commands: DebriefCommand[] = [
    {
      id: 'share.redactedCopy',
      title: $localize`:@@share.redactedCopy.command:Share redacted copy…`,
      category: $localize`:@@share.commands.categoryFile:File`,
      when: (ctx) => (ctx.summary?.sources.length ?? 0) > 0,
      run: () => this.open(),
    },
  ];
}
