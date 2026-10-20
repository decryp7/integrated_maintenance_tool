import { inject, Injectable, Injector } from '@angular/core';
import { AiLauncher } from '../ai/ai-launcher';
import { Narrative, Proposal } from '../ai/ai.types';
import { WorkspaceStore } from '../core/workspace.store';
import { PANEL } from '../dock/dock.service';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { JobsBackend } from './jobs-backend';
import { JobInfo, JobResult } from './jobs.types';
import type { FolderRefinement, SurveyOutcome } from '../ai/folder-setup/folder-setup.types';
import type { StorageClearResult } from '../settings/storage/storage.types';

export const JOBS_VIEW = 'jobs';
const AI_CHAT_VIEW = 'ai-chat';

/** The analysis views a job's `view` result names (the engine's names are the view ids). */
const ANALYSIS_VIEWS = new Set(['patterns', 'exceptions', 'anomalies', 'compare', 'moment', 'query']);

/**
 * View: opens a finished job's result where it belongs — an AI answer with its citations or the proposals to
 * review, the chat of a turn, the analysis view with its query, the workspace after an import, the file of an
 * export in Finder or Explorer, the model manager after a download. Asking the engine for the result marks the
 * job seen.
 */
@Injectable({ providedIn: 'root' })
export class JobViewer {
  private readonly backend = inject(JobsBackend);
  private readonly injector = inject(Injector);

  private ctx() {
    return this.injector.get(COMMAND_CONTEXT);
  }

  /** Opens the job's result; resolves with an error message when it could not (shown inline by the caller). */
  async view(job: Pick<JobInfo, 'id' | 'title' | 'resultType'>): Promise<string | null> {
    try {
      if (job.resultType === 'file') {
        await this.backend.reveal(job.id);
        return null;
      }
      const result = await this.backend.result(job.id);
      if (!result) {
        this.ctx().showView(JOBS_VIEW);
        return null;
      }
      await this.open(result, job.title);
      return null;
    } catch (e) {
      return e instanceof Error ? e.message : typeof e === 'string' ? e : $localize`:@@jobs.viewer.failed:The result could not be opened.`;
    }
  }

  private async open(result: JobResult, title: string): Promise<void> {
    const ctx = this.ctx();
    const ai = this.injector.get(AiLauncher);
    switch (result.type) {
      case 'narrative':
        return ai.showNarrative(result.narrative as Narrative, title);
      case 'proposal': {
        const p = result.proposal as { kind?: string };
        if (p?.kind === 'folderSetup') {
          const [{ FolderSetupLauncher }, { FolderSetupStore }] = await Promise.all([import('../ai/folder-setup/provide-folder-setup'), import('../ai/folder-setup/folder-setup.store')]);
          this.injector.get(FolderSetupStore).addRefinement(result.proposal as FolderRefinement);
          this.injector.get(FolderSetupLauncher).show();
          return;
        }
        return ai.showProposal(result.proposal as Proposal, title);
      }
      case 'chat': {
        ctx.showView(AI_CHAT_VIEW);
        const { ChatStore } = await import('../ai/chat/chat.store');
        await this.injector.get(ChatStore).select(result.sessionId);
        return;
      }
      case 'view':
        if (result.ql !== null) this.injector.get(WorkspaceStore).patchQuery({ ql: result.ql });
        if (result.view === 'folderSetup') {
          const { FolderSetupLauncher } = await import('../ai/folder-setup/provide-folder-setup');
          this.injector.get(FolderSetupLauncher).open();
          return;
        }
        ctx.showView(ANALYSIS_VIEWS.has(result.view) ? result.view : JOBS_VIEW);
        return;
      case 'workspace':
        ctx.showView(PANEL.events);
        return;
      case 'model':
        return ai.openModels();
      case 'folderSurvey': {
        const [{ FolderSetupLauncher }, { FolderSetupStore }] = await Promise.all([import('../ai/folder-setup/provide-folder-setup'), import('../ai/folder-setup/folder-setup.store')]);
        this.injector.get(FolderSetupStore).showSurvey(result.outcome as SurveyOutcome);
        this.injector.get(FolderSetupLauncher).show();
        return;
      }
      case 'storage': {
        const [{ StorageStore }, { SettingsDialogService }, { STORAGE_ID }] = await Promise.all([
          import('../settings/storage/storage.store'),
          import('../settings/settings-dialog'),
          import('../settings/storage/storage.types'),
        ]);
        this.injector.get(StorageStore).showResult(result.result as StorageClearResult);
        this.injector.get(SettingsDialogService).open(STORAGE_ID);
        return;
      }
      case 'file':
        return;
    }
  }
}
