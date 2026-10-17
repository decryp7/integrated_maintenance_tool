import { invoke } from '../core/activity/ipc';
import { inject, Provider } from '@angular/core';
import { reportBackendFailures } from '../core/failure-reporter';
import { Logger, withFailureLogging } from '../core/logger';
import { isTauri } from '../core/platform';
import { CompareOptions, DiffExportFormat, DiffFormats, DiffOptions, DiffPage, DiffExported, SnapshotDiff } from '../core/config-diff.types';
import { ConfigDiffBackend } from './config-diff-api';

export { ConfigDiffBackend } from './config-diff-api';
import { MockConfigDiffBackend } from './mock/mock-config-diff-backend';

/** Talks to the Rust shell; `call` is Tauri's `invoke` (tests pass a fake). */
export class TauriConfigDiffBackend extends ConfigDiffBackend {
  constructor(private readonly call: typeof invoke = invoke) {
    super();
  }

  formats() {
    return this.call<DiffFormats>('config_diff_formats');
  }
  diff(leftPath: string, rightPath: string, options: DiffOptions) {
    return this.call<DiffPage>('diff_configs', { leftPath, rightPath, options });
  }
  snapshots(leftDir: string, rightDir: string, compare: CompareOptions) {
    return this.call<SnapshotDiff>('diff_snapshots', { leftDir, rightDir, compare });
  }
  exportDiff(leftPath: string, rightPath: string, options: DiffOptions, format: DiffExportFormat, path: string, overwrite = false) {
    return this.call<DiffExported>('export_config_diff', { leftPath, rightPath, options, format, path, ...(overwrite ? { overwrite } : {}) });
  }
}

/** The real backend inside Tauri, the in-memory mock in a plain browser (ng serve, e2e). */
export function provideConfigDiffBackend(): Provider[] {
  return [
    {
      provide: ConfigDiffBackend,
      useFactory: () => {
        const real = isTauri() ? new TauriConfigDiffBackend() : new MockConfigDiffBackend();
        return withFailureLogging<ConfigDiffBackend>(real, inject(Logger), { onFailure: reportBackendFailures() });
      },
    },
  ];
}
