import { inject, Injectable, Injector } from '@angular/core';
import { ImtCommand } from '../../extensions/tokens';
import { DialogService } from '../../ui/dialog.service';
import { EtlPicker } from '../dotnet/dotnet-commands';
import { NativeBackend } from './native-backend';
import { NativeOpener } from './native-capture.store';

/** "Profile native process…" (Tools) and "Open native process from .etl…" (File); the dialogs load lazily. */
@Injectable({ providedIn: 'root' })
export class NativeCommands {
  private readonly dialogs = inject(DialogService);
  private readonly picker = inject(EtlPicker);
  private readonly injector = inject(Injector);

  readonly commands: ImtCommand[] = [
    { id: 'profiling.native', title: $localize`:@@profiling.cmd.native:Profile native process…`, category: 'Tools', run: () => this.openCapture() },
    { id: 'profiling.native.etl', title: $localize`:@@profiling.cmd.nativeEtl:Open native process from .etl…`, category: 'File', run: () => this.openEtl() },
  ];

  async openCapture(): Promise<void> {
    const { NativeDialog } = await import('./native-dialog');
    this.dialogs.open(NativeDialog, { title: $localize`:@@profiling.native.title:Profile native process`, width: 720 });
  }

  /** The `.etl` process picker with the native profiler's conversion (symbols, heap events). */
  async openEtl(): Promise<void> {
    const path = await this.picker.pick();
    if (!path) return;
    const { EtlDialog } = await import('../dotnet/etl-dialog');
    this.dialogs.open(EtlDialog, {
      title: $localize`:@@profiling.native.etlTitle:Open a native process from an ETW trace`,
      width: 600,
      inputs: { path, source: this.injector.get(NativeBackend), opener: this.injector.get(NativeOpener) },
    });
  }
}
