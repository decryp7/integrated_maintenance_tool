import { Injectable } from '@angular/core';
import { nativePickOne } from '../core/native-dialog';
import { isTauri } from '../core/platform';

/** What the browser build (mock backend) uses instead of the native save dialog. */
export const DEMO_CSV_FILE = '/demo/result.csv';

/** The save dialog of the console's Export CSV; the engine re-checks the chosen path. */
@Injectable({ providedIn: 'root' })
export class SqlExportFiles {
  /* v8 ignore start: a thin wrapper over the native dialog, exercised only in the real app */
  async pickCsv(): Promise<string | null> {
    if (!isTauri()) return DEMO_CSV_FILE;
    return nativePickOne({
      kind: 'save',
      title: $localize`:@@store.sql.exportTitle:Export the result as CSV`,
      defaultName: 'result.csv',
      filters: [{ name: 'CSV', extensions: ['csv'] }],
    });
  }
  /* v8 ignore stop */
}
