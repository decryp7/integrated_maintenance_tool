import { inject, Injectable, Injector } from '@angular/core';
import { ProfileFinding } from '../../core/api.types';
import { COMMAND_CONTEXT } from '../../extensions/command-registry';

/** The AI chat view (provide-ai.ts `AI_CHAT_VIEW`). */
const AI_CHAT_VIEW = 'ai-chat';
/** Most findings and characters sent: the summary stays compact. */
const MAX_FINDINGS = 10;
const MAX_CHARS = 4000;

/**
 * The question "Explain this" sends: the engine's findings (already computed and worded by the engine), compact.
 * Pure, so it is tested without the AI.
 */
export function explainPrompt(profileName: string, findings: readonly ProfileFinding[], evidence: (f: ProfileFinding) => string): string {
  const lines = findings.slice(0, MAX_FINDINGS).map((f) => `- [${f.severity}] ${f.title}. ${f.detail} (evidence: ${evidence(f)})`);
  const head = $localize`:@@profiling.explain.prompt:Explain these profiling findings for "${profileName}:name:" in plain words: what the likely root cause is, and what to change first. The findings were computed by IMT's profiling engine:`;
  const text = [head, ...lines].join('\n');
  return text.length > MAX_CHARS ? `${text.slice(0, MAX_CHARS - 1)}…` : text;
}

/** "Explain with AI": opens the chat and asks about the findings. */
@Injectable({ providedIn: 'root' })
export class FindingsExplainer {
  private readonly injector = inject(Injector);

  async explain(profileName: string, findings: readonly ProfileFinding[], evidence: (f: ProfileFinding) => string): Promise<boolean> {
    this.injector.get(COMMAND_CONTEXT).showView(AI_CHAT_VIEW);
    const { ChatStore } = await import('../../ai/chat/chat.store');
    return this.injector.get(ChatStore).send(explainPrompt(profileName, findings, evidence));
  }
}
