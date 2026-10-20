import { inject, Injectable, Injector } from '@angular/core';
import { invoke } from '../../core/activity/ipc';
import { reportBackendFailures } from '../../core/failure-reporter';
import { Logger, withFailureLogging } from '../../core/logger';
import { isTauri } from '../../core/platform';
import { ChannelFactory, tauriChannel } from '../../core/tauri-channel';
import { toAiError } from '../ai-backend';
import { ComposedProfile, Fragment, SurveyProgress, SurveyRequest } from './folder-setup.types';

/**
 * The engine operations behind "Set up analysis for this folder…". Injected as a class token: the Tauri
 * commands in the desktop app, the mock in the browser build (dev and UI tests).
 */
@Injectable({ providedIn: 'root', useFactory: () => createFolderSetupBackend() })
export abstract class FolderSetupBackend {
  /** Starts surveying the open files as a background analysis job and resolves with it at once; progress arrives on
   * `onProgress`, the validated proposals are the job's result (`folderSurvey`). Cancel is the job's. */
  abstract survey(onProgress: (p: SurveyProgress) => void, request?: SurveyRequest): Promise<number>;
  /** The active profile with `fragments` merged in, validated by the engine, with its diff. Saves nothing. */
  abstract compose(fragments: Fragment[]): Promise<ComposedProfile>;
  /** Starts AI proposals (validated by the engine) as a background AI job and resolves with it at once; the result
   * is a `folderSetup` proposal. Rejects at once with an `AiError` (`unavailable`, `noModel`...). */
  abstract refineWithAi(request?: SurveyRequest): Promise<number>;
}

/** Talks to the Rust core through Tauri commands (contract: `folder-setup.types.ts`). */
export class TauriFolderSetupBackend extends FolderSetupBackend {
  /** `call` is Tauri's `invoke` and `channel` makes IPC channels; tests pass fakes. */
  constructor(
    private readonly call: typeof invoke = invoke,
    private readonly channel: ChannelFactory = tauriChannel,
  ) {
    super();
  }

  survey(onProgress: (p: SurveyProgress) => void, request?: SurveyRequest) {
    return this.call<number>('folder_survey', { request: request ?? null, onProgress: this.channel(onProgress) });
  }
  compose(fragments: Fragment[]) {
    return this.call<ComposedProfile>('folder_setup_compose', { fragments });
  }
  async refineWithAi(request?: SurveyRequest) {
    try {
      return await this.call<number>('ai_start', { request: { op: 'folderSetup', request: request ?? null } });
    } catch (e) {
      throw toAiError(e);
    }
  }
}

/** The Tauri backend in the desktop app, the mock (loaded lazily) in a browser; both behind the activity/failure proxy. */
export function createFolderSetupBackend(): FolderSetupBackend {
  const target = isTauri() ? new TauriFolderSetupBackend() : lazyMock(inject(Injector));
  return withFailureLogging<FolderSetupBackend>(target, inject(Logger), { onFailure: reportBackendFailures() });
}

/** Defers loading the mock (and the mock workspace it reads) until the first call. */
function lazyMock(injector: Injector): FolderSetupBackend {
  let loaded: Promise<FolderSetupBackend> | null = null;
  const get = () => (loaded ??= import('./mock-folder-setup-backend').then((m) => m.createMockFolderSetupBackend(injector)));
  return new (class extends FolderSetupBackend {
    survey(onProgress: (p: SurveyProgress) => void, request?: SurveyRequest) {
      return get().then((b) => b.survey(onProgress, request));
    }
    compose(fragments: Fragment[]) {
      return get().then((b) => b.compose(fragments));
    }
    refineWithAi(request?: SurveyRequest) {
      return get().then((b) => b.refineWithAi(request));
    }
  })();
}
