import { inject, Injectable } from '@angular/core';
import { ImtCommand } from '../../extensions/tokens';
import { DialogService } from '../../ui/dialog.service';

/** "Profile .NET process…" (Tools); the dialog loads lazily. `.etl` traces are the native profiler's commands. */
@Injectable({ providedIn: 'root' })
export class DotnetCommands {
  private readonly dialogs = inject(DialogService);

  readonly commands: ImtCommand[] = [
    { id: 'profiling.dotnet', title: $localize`:@@profiling.cmd.dotnet:Profile .NET process…`, category: 'Tools', run: () => this.openCapture() },
  ];

  async openCapture(): Promise<void> {
    const { DotnetDialog } = await import('./dotnet-dialog');
    this.dialogs.open(DotnetDialog, { title: $localize`:@@profiling.dotnet.title:Profile .NET process`, width: 720 });
  }
}
