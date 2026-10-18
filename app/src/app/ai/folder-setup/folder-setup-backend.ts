import { inject, Injectable, Injector } from '@angular/core';
import { invoke } from '../../core/activity/ipc';
import { reportBackendFailures } from '../../core/failure-reporter';
import { Logger, withFailureLogging } from '../../core/logger';
import { isTauri } from '../../core/platform';
import { ChannelFactory, tauriChannel } from '../../core/tauri-channel';
import { toAiError } from '../ai-backend';
import { ComposedProfile, FolderRefinement, Fragment, SurveyOutcome, SurveyProgress, SurveyRequest } from './folder-setup.types';

/**
 * The engine operations behind "Set up analysis for this folder…". Injected as a class token: the Tauri
 * commands in the desktop app, the mock in the browser build (dev and UI tests).
 */
@Injectable({ providedIn: 'root', useFactory: () => createFolderSetupBackend() })
export abstract class FolderSetupBackend {
  /** Surveys the open files and returns validated heuristic proposals; cancellable with `cancelSurvey(surveyId)`. */
  abstract survey(surveyId: string, onProgress: (p: SurveyProgress) => void, request?: SurveyRequest): Promise<SurveyOutcome>;
  /** Asks the survey `surveyId` to stop; false when it is not running (any more). */
  abstract cancelSurvey(surveyId: string): Promise<boolean>;
  /** The active profile with `fragments` merged in, validated by the engine, with its diff. Saves nothing. */
  abstract compose(fragments: Fragment[]): Promise<ComposedProfile>;
  /** AI proposals (validated by the engine). Rejects with an `AiError` (`unavailable`, `noModel`, `cancelled`...). */
  abstract refineWithAi(request?: SurveyRequest): Promise<FolderRefinement>;
  /** Stops a running AI refinement. */
  abstract cancelAi(): Promise<void>;
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

  survey(surveyId: string, onProgress: (p: SurveyProgress) => void, request?: SurveyRequest) {
    return this.call<SurveyOutcome>('folder_survey', { surveyId, request: request ?? null, onProgress: this.channel(onProgress) });
  }
  cancelSurvey(surveyId: string) {
    return this.call<boolean>('folder_survey_cancel', { surveyId });
  }
  compose(fragments: Fragment[]) {
    return this.call<ComposedProfile>('folder_setup_compose', { fragments });
  }
  async refineWithAi(request?: SurveyRequest) {
    try {
      return await this.call<FolderRefinement>('ai_refine_folder_setup', { request: request ?? null });
    } catch (e) {
      throw toAiError(e);
    }
  }
  async cancelAi() {
    await this.call<number>('ai_cancel');
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
    survey(id: string, onProgress: (p: SurveyProgress) => void, request?: SurveyRequest) {
      return get().then((b) => b.survey(id, onProgress, request));
    }
    cancelSurvey(id: string) {
      return get().then((b) => b.cancelSurvey(id));
    }
    compose(fragments: Fragment[]) {
      return get().then((b) => b.compose(fragments));
    }
    refineWithAi(request?: SurveyRequest) {
      return get().then((b) => b.refineWithAi(request));
    }
    cancelAi() {
      return get().then((b) => b.cancelAi());
    }
  })();
}
