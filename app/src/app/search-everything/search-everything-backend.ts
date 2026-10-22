import { inject, Injectable } from '@angular/core';
import { invoke } from '../core/activity/ipc';
import { SearchAllReply, SearchAllRequest } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { reportBackendFailures } from '../core/failure-reporter';
import { Logger, withFailureLogging } from '../core/logger';
import { browserMocks } from '../core/mock-registry';
import { isTauri } from '../core/platform';

/**
 * Search everything as the UI sees it: the engine searches the window's events, sources, fields, marker kinds, notes and
 * knowledge files and answers a few marked results of each kind. Injected as a class token: the `search_everything`
 * command in the app (`app/src-tauri/src/search_everything.rs`), a mock over the mock workspace in a plain browser.
 * Rejects with a `CmdError`.
 */
@Injectable({ providedIn: 'root', useFactory: createSearchEverythingBackend })
export abstract class SearchEverythingBackend {
  /** The results for `request`; null when the text has no words or a newer search of the window replaced this one. */
  abstract searchEverything(request: SearchAllRequest): Promise<SearchAllReply | null>;
}

/** Talks to the `search_everything` command. */
export class TauriSearchEverythingBackend extends SearchEverythingBackend {
  /** `call` is Tauri's `invoke`; tests pass a fake. */
  constructor(private readonly call: typeof invoke = invoke) {
    super();
  }
  searchEverything(request: SearchAllRequest) {
    return this.call<SearchAllReply | null>('search_everything', { request });
  }
}

export function createSearchEverythingBackend(): SearchEverythingBackend {
  const backend: SearchEverythingBackend = isTauri() ? new TauriSearchEverythingBackend() : new (browserMocks().MockSearchEverythingBackend)(inject(ImtBackend));
  return withFailureLogging<SearchEverythingBackend>(backend, inject(Logger), { category: 'ui.search', onFailure: reportBackendFailures() });
}
