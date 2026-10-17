import { inject, Injectable } from '@angular/core';
import { DialogService } from '../ui/dialog.service';
import { AiStatusStore } from './ai-status.store';
import { ProfileChangeKind, SampleSource } from './ai.types';

/** What a "Propose …" entry point asks for. */
export type ProposeKind = ProfileChangeKind | 'profile';

const TITLES: Record<ProposeKind, string> = {
  pipeline: 'Propose pipeline steps',
  markers: 'Propose markers',
  extractors: 'Propose extractors',
  profile: 'Propose a parsing profile',
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
    this.dialogs.open(m.AiSetupWizard, { title: 'Set up the AI assistant', width: 760, inputs: modelId ? { modelId } : {} });
  }

  async openModels(): Promise<void> {
    const m = await this.lazy();
    this.dialogs.open(m.AiModelManager, { title: 'AI models', width: 820 });
  }

  async explainEvent(recordId: number): Promise<void> {
    if (!(await this.ensureModel())) return;
    const m = await this.lazy();
    this.dialogs.open(m.AiNarrativeDialog, { title: `Explain event ${recordId}`, width: 680, inputs: { kind: 'explain', recordId } });
  }

  async summarize(from: number, to: number): Promise<void> {
    if (!(await this.ensureModel())) return;
    const m = await this.lazy();
    this.dialogs.open(m.AiNarrativeDialog, { title: 'Summarize this range', width: 680, inputs: { kind: 'summarize', from, to } });
  }

  async rootCauses(): Promise<void> {
    if (!(await this.ensureModel())) return;
    const m = await this.lazy();
    this.dialogs.open(m.AiNarrativeDialog, { title: 'Likely root causes', width: 680, inputs: { kind: 'rootCauses' } });
  }

  /** "Ask AI to write a query": a question in plain words becomes a query the user can apply. */
  async askQuery(question = ''): Promise<void> {
    if (!(await this.ensureModel())) return;
    const m = await this.lazy();
    this.dialogs.open(m.AiQueryDialog, { title: 'Ask AI to write a query', width: 640, inputs: { initial: question } });
  }

  /**
   * "Suggest regex": resolves to the pattern the user accepted (for the caller's own pattern field), or
   * null. With `highlight`, accepting offers the pattern as a highlight rule instead.
   */
  async suggestRegex(lines: readonly string[], options: { intent?: string; highlight?: boolean } = {}): Promise<string | null> {
    if (!(await this.ensureModel())) return null;
    const m = await this.lazy();
    const ref = this.dialogs.open<string>(m.AiRegexDialog, {
      title: 'Suggest a regex',
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
    const title = options.chooseKind ? 'Propose profile changes' : TITLES[kind];
    this.dialogs.open(m.AiProposeDialog, { title, width: 760, inputs: { kind, source, initialGoal: goal, chooseKind: !!options.chooseKind } });
  }
}
