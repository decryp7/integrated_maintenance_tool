import { inject, Injectable } from '@angular/core';
import { invoke } from '../../core/activity/ipc';
import { ParserDocDto, ParserDraftRequest, ParserImportCheckDto, ParserLibraryDto, ParserValidationDto } from '../../core/api.types';
import { DebriefBackend } from '../../core/backend';
import { reportBackendFailures } from '../../core/failure-reporter';
import { Logger, withFailureLogging } from '../../core/logger';
import { browserMocks } from '../../core/mock-registry';
import { isTauri } from '../../core/platform';

/** How `save` treats an existing parser with the same id. */
export interface ParserSaveOptions {
  /** The id the edited parser had (renaming or updating it is not a conflict). */
  previousId?: string | null;
  /** Replace a user parser with that id (after the user confirmed). */
  overwrite?: boolean;
}

/**
 * The parser library commands (ADR 0026): parsing, validation, matching and drafting are the engine's; the UI sends
 * TOML text, ids and paths the user chose. Injected as a class token: the Tauri backend in the app, the mock over
 * the mock workspace in a plain browser (dev server, e2e tests).
 */
@Injectable({ providedIn: 'root', useFactory: createParsersBackend })
export abstract class ParsersBackend {
  /** Built-in, user and override parsers, broken user files, and how many of this window's sources are stale. */
  abstract list(): Promise<ParserLibraryDto>;
  /** One parser's TOML as stored. */
  abstract get(id: string): Promise<ParserDocDto>;
  /** Checks a parser TOML (errors with line and column). */
  abstract validate(toml: string): Promise<ParserValidationDto>;
  /** Creates or updates a user parser (a built-in's id saves an override); rejects with code `fileExists` on an id clash (retry with `overwrite`). */
  abstract save(toml: string, options?: ParserSaveOptions): Promise<ParserLibraryDto>;
  /** Saves a copy with a new id and returns it. */
  abstract duplicate(id: string): Promise<ParserDocDto>;
  /** Removes a user parser or an override (the built-in comes back). */
  abstract remove(id: string): Promise<ParserLibraryDto>;
  abstract setEnabled(id: string, enabled: boolean): Promise<ParserLibraryDto>;
  /** Reads and validates a parser file the user picked, and how it relates to the library. */
  abstract importCheck(path: string): Promise<ParserImportCheckDto>;
  /** Writes the parser file as stored; refuses to replace a file without `overwrite` (FILE_EXISTS). */
  abstract exportTo(id: string, path: string, overwrite?: boolean): Promise<void>;
  /** The format wizard's candidate parser (not saved). */
  abstract draft(request: ParserDraftRequest): Promise<ParserDocDto>;
  /** Re-reads the window's stale sources (and retries unrecognised files) as a job; resolves with its id. */
  abstract reread(): Promise<number>;
  /** "Parse with…": re-reads one source with that parser (null: automatic) as a job; resolves with its id. */
  abstract assign(sourceId: number, parserId: string | null): Promise<number>;
}

/** Talks to `app/src-tauri/src/parsers.rs` (see `api.types.ts`, "Parsers"). */
export class TauriParsersBackend extends ParsersBackend {
  /** `call` is Tauri's `invoke`; tests pass a fake. */
  constructor(private readonly call: typeof invoke = invoke) {
    super();
  }
  list() {
    return this.call<ParserLibraryDto>('parsers_list');
  }
  get(id: string) {
    return this.call<ParserDocDto>('parser_get', { id });
  }
  validate(toml: string) {
    return this.call<ParserValidationDto>('parser_validate', { toml });
  }
  save(toml: string, options: ParserSaveOptions = {}) {
    return this.call<ParserLibraryDto>('parser_save', { toml, previousId: options.previousId ?? null, overwrite: options.overwrite ?? false });
  }
  duplicate(id: string) {
    return this.call<ParserDocDto>('parser_duplicate', { id });
  }
  remove(id: string) {
    return this.call<ParserLibraryDto>('parser_remove', { id });
  }
  setEnabled(id: string, enabled: boolean) {
    return this.call<ParserLibraryDto>('parser_set_enabled', { id, enabled });
  }
  importCheck(path: string) {
    return this.call<ParserImportCheckDto>('parser_import_check', { path });
  }
  exportTo(id: string, path: string, overwrite = false) {
    return this.call<void>('parser_export', { id, path, overwrite });
  }
  draft(request: ParserDraftRequest) {
    return this.call<ParserDocDto>('parser_draft', { request });
  }
  reread() {
    return this.call<number>('parsers_reread');
  }
  assign(sourceId: number, parserId: string | null) {
    return this.call<number>('parser_assign', { sourceId, parserId });
  }
}

export function createParsersBackend(): ParsersBackend {
  const backend: ParsersBackend = isTauri() ? new TauriParsersBackend() : new (browserMocks().MockParsersBackend)(inject(DebriefBackend));
  return withFailureLogging<ParsersBackend>(backend, inject(Logger), { onFailure: reportBackendFailures() });
}
