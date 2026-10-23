import { inject, Injectable } from '@angular/core';
import { nativePickOne } from '../../core/native-dialog';
import { isTauri } from '../../core/platform';
import { DebriefCommand } from '../../extensions/tokens';
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

/** "Profile native process…" (Tools) and "Open process from .etl…" (File, .NET and native processes); the dialogs load lazily. */
@Injectable({ providedIn: 'root' })
export class NativeCommands {
  private readonly dialogs = inject(DialogService);
  private readonly picker = inject(EtlPicker);

  readonly commands: DebriefCommand[] = [
    { id: 'profiling.native', title: $localize`:@@profiling.cmd.native:Profile native process…`, category: 'Tools', run: () => this.openCapture() },
    { id: 'profiling.etl', title: $localize`:@@profiling.cmd.etl:Open process from .etl…`, category: 'File', run: () => this.openEtl() },
  ];

  async openCapture(): Promise<void> {
    const { NativeDialog } = await import('./native-dialog');
    this.dialogs.open(NativeDialog, { title: $localize`:@@profiling.native.title:Profile native process`, width: 720 });
  }

  /** The one `.etl` process picker (.NET ones first; managed and native frames, heap events). */
  async openEtl(): Promise<void> {
    const path = await this.picker.pick();
    if (!path) return;
    const { EtlDialog } = await import('./etl-dialog');
    this.dialogs.open(EtlDialog, { title: $localize`:@@profiling.etl.title:Open a process from an ETW trace`, width: 600, inputs: { path } });
  }
}
