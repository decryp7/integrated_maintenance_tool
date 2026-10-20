import { inject, Injectable } from '@angular/core';
import { nativePickOne } from '../../core/native-dialog';
import { isTauri } from '../../core/platform';
import { ImtCommand } from '../../extensions/tokens';
import { DialogService } from '../../ui/dialog.service';

/** Chooses an `.etl` file: the native dialog in Tauri, a demo path in a browser. Replaceable in tests. */
@Injectable({ providedIn: 'root' })
export class EtlPicker {
  pick(): Promise<string | null> {
    /* v8 ignore next 3: the native dialog exists only inside Tauri */
    if (isTauri()) {
      return nativePickOne({ kind: 'openFile', title: $localize`:@@profiling.etl.pickTitle:Open an ETW trace`, filters: [{ name: 'ETW trace', extensions: ['etl'] }] });
    }
    return Promise.resolve('/demo/traces/shop.etl');
  }
}

/** "Profile .NET process…" (Tools) and "Open process from .etl…" (File); the dialogs load lazily. */
@Injectable({ providedIn: 'root' })
export class DotnetCommands {
  private readonly dialogs = inject(DialogService);
  private readonly picker = inject(EtlPicker);

  readonly commands: ImtCommand[] = [
    { id: 'profiling.dotnet', title: $localize`:@@profiling.cmd.dotnet:Profile .NET process…`, category: 'Tools', run: () => this.openCapture() },
    { id: 'profiling.etl', title: $localize`:@@profiling.cmd.etl:Open process from .etl…`, category: 'File', run: () => this.openEtl() },
  ];

  async openCapture(): Promise<void> {
    const { DotnetDialog } = await import('./dotnet-dialog');
    this.dialogs.open(DotnetDialog, { title: $localize`:@@profiling.dotnet.title:Profile .NET process`, width: 720 });
  }

  async openEtl(): Promise<void> {
    const path = await this.picker.pick();
    if (!path) return;
    const { EtlDialog } = await import('./etl-dialog');
    this.dialogs.open(EtlDialog, { title: $localize`:@@profiling.etl.title:Open a process from an ETW trace`, width: 600, inputs: { path } });
  }
}
