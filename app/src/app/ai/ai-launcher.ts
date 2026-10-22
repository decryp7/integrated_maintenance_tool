import { inject, Injectable, Injector } from '@angular/core';
import { DialogService } from '../ui/dialog.service';
import { AiStatusStore } from './ai-status.store';
import { Narrative, NarrativeRequest, ProfileChangeKind, Proposal, ResearchArgs, SampleSource } from './ai.types';
import { JobFollow } from '../jobs/job-follow';
import { AiBackend, toAiError } from './ai-backend';
import { Platform } from '../core/platform';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { imageFilter } from './vision/image-files';

/** The image view's id (ADR 0025). */
export const AI_IMAGE_VIEW = 'ai-image';
import type { QuerySource, Segment } from '../browser/browser.types';

/** What a "Propose …" entry point asks for. */
export type ProposeKind = ProfileChangeKind | 'profile';

const TITLES: Record<ProposeKind, string> = {
  pipeline: $localize`:@@ai.launcher.title.proposePipeline:Propose pipeline steps`,
  markers: $localize`:@@ai.launcher.title.proposeMarkers:Propose markers`,
  extractors: $localize`:@@ai.launcher.title.proposeExtractors:Propose extractors`,
  profile: $localize`:@@ai.launcher.title.proposeParser:Propose a parser`,
};

/**
 * Opens the AI dialogs from anywhere (palette, menus, buttons in other views). Small and eager; the
 * dialogs themselves load lazily with the rest of the AI UI. Without a model, every use case opens the
 * set-up wizard instead.
 */
@Injectable({ providedIn: 'root' })
export class AiLauncher {
  private readonly dialogs = inject(DialogService);
  private readonly status = inject(AiStatusStore);
  private readonly injector = inject(Injector);

  private lazy() {
    return import('./ai-lazy');
  }

  /** True when the use case can run; otherwise the set-up wizard was opened. */
  private async ensureModel(): Promise<boolean> {
    if (this.status.hasModel()) return true;
    await this.openSetup();
    return false;
  }

  /** The set-up wizard; `modelId` preselects a catalog model instead of the recommended one. */
  async openSetup(modelId?: string): Promise<void> {
    const m = await this.lazy();
    this.dialogs.open(m.AiSetupWizard, { title: $localize`:@@ai.launcher.title.setup:Set up the AI assistant`, width: 760, inputs: modelId ? { modelId } : {} });
  }

  async openModels(): Promise<void> {
    const m = await this.lazy();
    this.dialogs.open(m.AiModelManager, { title: $localize`:@@ai.launcher.title.models:AI models`, width: 820 });
  }

  /** "Explain this event" runs in the background: a toast says so, "Explanation ready · View" opens the answer. */
  explainEvent(recordId: number): Promise<void> {
    return this.startNarrative({ op: 'explain', recordId }, $localize`:@@ai.launcher.started.explain:Explaining event #${recordId}:id: — running in Jobs`);
  }

  summarize(from: number, to: number): Promise<void> {
    return this.startNarrative({ op: 'summarize', from, to }, $localize`:@@ai.launcher.started.summarize:Summarizing the range — running in Jobs`);
  }

  rootCauses(): Promise<void> {
    return this.startNarrative({ op: 'rootCauses' }, $localize`:@@ai.launcher.started.rootCauses:Looking for root causes — running in Jobs`);
  }

  /** Queues the request as an AI job and returns at once; the user keeps working. */
  private async startNarrative(request: NarrativeRequest, started: string): Promise<void> {
    if (!(await this.ensureModel())) return;
    const follow = this.injector.get(JobFollow);
    try {
      await this.injector.get(AiBackend).start(request);
      follow.started(started);
    } catch (e) {
      follow.notStarted($localize`:@@ai.launcher.startFailed:The AI request did not start`, toAiError(e).message);
    }
  }

  /**
   * "Research this": the AI drafts the search text from the question and the event, and the Research dialog shows it
   * (with what the leak guard found) for the user to edit before anything is searched. A search text the user already
   * saw (the chat's offer card) starts the research at once, unless it has workspace data in it. Resolves with an error
   * message when nothing could start.
   */
  async research(args: ResearchArgs): Promise<string | null> {
    if (!(await this.ensureModel())) return null;
    const backend = this.injector.get(AiBackend);
    try {
      if (args.query?.trim()) {
        const r = await backend.research({ ...args, confirmed: false });
        if (r.status === 'confirm') await this.askResearch({ ...args, question: r.question, query: r.checked.query, segments: r.checked.segments });
        return null;
      }
      const d = await backend.researchDraft({ question: args.question ?? null, recordId: args.recordId ?? null });
      await this.askResearch({ ...args, question: d.question, query: d.checked.query, segments: d.checked.needsConfirm ? d.checked.segments : [], source: d.source });
      return null;
    } catch (e) {
      return toAiError(e).message;
    }
  }

  /** The Research dialog: the question, the search text (with what the leak guard found and who wrote it) and how many
   * results to read. */
  async askResearch(args: ResearchArgs & { segments?: Segment[]; source?: QuerySource | null }): Promise<void> {
    if (!(await this.ensureModel())) return;
    const { ResearchDialog } = await import('../browser/research-dialog');
    this.dialogs.open(ResearchDialog, {
      title: $localize`:@@ai.launcher.title.research:Research the web`,
      width: 600,
      inputs: {
        question: args.question ?? '',
        query: args.query ?? '',
        segments: args.segments ?? [],
        source: args.source ?? null,
        sessionId: args.sessionId ?? null,
        recordId: args.recordId ?? null,
        maxResults: args.maxResults ?? null,
      },
    });
  }

  /**
   * Opens an image for the AI in the image view (ADR 0025): `path` (an Explorer file), else one the user picks.
   * With `findRelated`, "Find related logs" starts at once. Resolves with an error message when it did not open.
   */
  async openImage(path?: string | null, options: { findRelated?: boolean } = {}): Promise<string | null> {
    const chosen = path ?? (await this.injector.get(Platform).pickOpenPath($localize`:@@ai.vision.pickTitle:Open an image`, imageFilter()));
    if (!chosen) return null;
    const { AiImageStore } = await import('./vision/ai-image.store');
    const store = this.injector.get(AiImageStore);
    this.injector.get(COMMAND_CONTEXT).showView(AI_IMAGE_VIEW);
    const img = await store.openPath(chosen);
    if (!img) return store.openError();
    if (options.findRelated) await store.findRelated(img.id);
    return null;
  }

  /** A finished request's answer with its citations (View of an AI job). */
  async showNarrative(narrative: Narrative, title: string): Promise<void> {
    const m = await this.lazy();
    this.dialogs.open(m.AiNarrativeDialog, { title, width: 680, inputs: { preset: narrative } });
  }

  /** A finished request's proposal, to review and apply (View of an AI job). */
  async showProposal(proposal: Proposal, title: string): Promise<void> {
    const m = await this.lazy();
    this.dialogs.open(m.AiProposalResultDialog, { title, width: 760, inputs: { proposal } });
  }

  /** "Ask AI to write a query": a question in plain words becomes a query the user can apply. */
  async askQuery(question = ''): Promise<void> {
    if (!(await this.ensureModel())) return;
    const m = await this.lazy();
    this.dialogs.open(m.AiQueryDialog, { title: $localize`:@@ai.launcher.title.askQuery:Ask AI to write a query`, width: 640, inputs: { initial: question } });
  }

  /**
   * "Suggest regex": resolves to the pattern the user accepted (for the caller's own pattern field), or
   * null. With `highlight`, accepting offers the pattern as a highlight rule instead.
   */
  async suggestRegex(lines: readonly string[], options: { intent?: string; highlight?: boolean } = {}): Promise<string | null> {
    if (!(await this.ensureModel())) return null;
    const m = await this.lazy();
    const ref = this.dialogs.open<string>(m.AiRegexDialog, {
      title: $localize`:@@ai.launcher.title.suggestRegex:Suggest a regex`,
      width: 720,
      inputs: { lines: [...lines], intent: options.intent ?? '', highlight: !!options.highlight },
    });
    return (await ref.closed) ?? null;
  }

  /**
   * "Propose pipeline/markers/extractors/profile" from a sample (selected event or given text). With
   * `chooseKind` the user picks pipeline, markers or extractors in the dialog.
   */
  async propose(kind: ProposeKind, source: SampleSource, goal = '', options: { chooseKind?: boolean } = {}): Promise<void> {
    if (!(await this.ensureModel())) return;
    const m = await this.lazy();
    const title = options.chooseKind ? $localize`:@@ai.launcher.title.proposeChanges:Propose profile changes` : TITLES[kind];
    this.dialogs.open(m.AiProposeDialog, { title, width: 760, inputs: { kind, source, initialGoal: goal, chooseKind: !!options.chooseKind } });
  }
}
