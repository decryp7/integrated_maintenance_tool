import { effect, inject, Injectable, untracked } from '@angular/core';
import { SourceInfo, WorkspaceSummary } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { NotificationsStore } from '../notifications/notifications.store';
import { AiLauncher } from './ai-launcher';

/** Key of the message in the notification centre (a new import replaces it). */
export const PARSER_SUGGEST_NOTICE = 'parser-suggest';
/** The parser the library falls back to when no other reads a file (ADR 0026). */
const FALLBACK_PARSER = 'generic';

/**
 * The first source that only the fallback parser read and that the engine reported problems for (lines not read,
 * timestamps not understood): a file worth generating a parser for. Display logic only: the engine decided both.
 */
export function parserSuggestion(summary: WorkspaceSummary | null | undefined): SourceInfo | null {
  return (
    summary?.sources.find(
      (s) => s.parser === FALLBACK_PARSER && (!!s.timestampIssue || (s.problems ?? []).some((p) => p.count > 0) || s.skipped > 0),
    ) ?? null
  );
}

/**
 * After an import (not on start-up), when a file is read by no parser but the generic one and has problems, offers
 * "Generate a parser with AI…" as a message in the notification centre (ADR 0032). The user confirms; nothing runs
 * before.
 */
@Injectable({ providedIn: 'root' })
export class ParserSuggestNotice {
  private readonly store = inject(WorkspaceStore);
  private readonly notices = inject(NotificationsStore);
  private readonly ai = inject(AiLauncher);

  constructor() {
    let seen = this.store.importVersion();
    effect(() => {
      const version = this.store.importVersion();
      if (version === seen) return;
      seen = version;
      untracked(() => this.offer());
    });
  }

  private offer(): void {
    const source = parserSuggestion(this.store.summary());
    if (!source) return;
    this.notices.postLocal({
      key: PARSER_SUGGEST_NOTICE,
      source: 'ai',
      severity: 'info',
      title: $localize`:@@ai.parserSuggest.title:No parser reads ${source.name}:name: well`,
      body: $localize`:@@ai.parserSuggest.body:The local model can draft a parser for it; the engine checks every draft on the file and shows you the result to review.`,
      actionLabel: $localize`:@@ai.parserSuggest.action:Generate a parser with AI…`,
      run: () => this.ai.propose('parser', { sourceId: source.id, fileName: source.name }),
    });
  }
}
