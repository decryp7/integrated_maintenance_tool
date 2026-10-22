import { Injectable } from '@angular/core';
import { nativePickOne } from '../../core/native-dialog';
import { isTauri } from '../../core/platform';

const FILTERS = [{ name: $localize`:File type in the open/save dialog@@settings.parsers.files.filter:IMT parser`, extensions: ['toml'] }];

/** What "Import .toml…" picks in the browser build: a parser file the mock backend knows. */
export const DEMO_PARSER_IMPORT = '/demo/parsers/inventory-csv.toml';
/** Folder "Export…" writes to in the browser build. */
export const DEMO_PARSER_EXPORTS = '/demo/exports';

/** Native file dialogs for Settings ▸ Parsers; the engine records the choice as a grant and re-checks the path. */
@Injectable({ providedIn: 'root' })
export class ParserFiles {
  /* v8 ignore start: thin wrappers over the native dialogs, exercised only in the real app */
  async pickImport(): Promise<string | null> {
    if (!isTauri()) return DEMO_PARSER_IMPORT;
    return nativePickOne({ kind: 'openFile', title: $localize`:@@settings.parsers.files.import:Import a parser`, filters: FILTERS });
  }

  async pickExport(id: string): Promise<string | null> {
    if (!isTauri()) return `${DEMO_PARSER_EXPORTS}/${id}.toml`;
    return nativePickOne({ kind: 'save', title: $localize`:@@settings.parsers.files.export:Export the parser`, defaultName: `${id}.toml`, filters: FILTERS });
  }
  /* v8 ignore stop */
}
