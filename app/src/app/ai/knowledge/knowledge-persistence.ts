import { inject, Injectable, signal } from '@angular/core';
import { Logger } from '../../core/logger';
import { UiStateContribution } from '../../lifecycle/ui-state';
import { toAiError } from '../ai-backend';
import { KnowledgeRecord } from '../ai.types';
import { KnowledgeBackend } from './knowledge-backend';

/**
 * The window's project knowledge files as part of the project and session UI state (key `knowledge`,
 * `KnowledgeRecord` v1). The engine owns them; this keeps the last record it returned so saving stays synchronous
 * (a change marks the project unsaved), and hands saved ones back to the engine, which checks them.
 */
@Injectable({ providedIn: 'root' })
export class KnowledgePersistence {
  private readonly backend = inject(KnowledgeBackend);
  private readonly logger = inject(Logger);
  /** The last record the engine returned (null while the project has no knowledge files). */
  readonly record = signal<KnowledgeRecord | null>(null);
  /** Bumped when a project's files were restored, so an open list reloads. */
  readonly restored = signal(0);
  /** Why saved knowledge files could not be loaded, if so. */
  readonly warning = signal<string | null>(null);

  /** Reads the project's files from the engine (after a change). */
  async sync(): Promise<void> {
    try {
      const r = await this.backend.getProjectKnowledge();
      this.record.set(r.files.length ? r : null);
    } catch (e) {
      this.logger.warn('ai', 'Knowledge files could not be read for saving', { Code: toAiError(e).code });
    }
  }

  async restore(value: unknown): Promise<void> {
    try {
      const r = await this.backend.restoreProjectKnowledge(value ?? null);
      this.warning.set(r.warning);
      await this.sync();
    } catch (e) {
      // The engine could not take them: the saved files stay in the project untouched (captured back as they were).
      this.record.set(isRecord(value) ? value : null);
      this.logger.warn('ai', 'Saved knowledge files not restored', { Code: toAiError(e).code });
    }
    this.restored.update((n) => n + 1);
  }

  readonly contribution: UiStateContribution = {
    key: 'knowledge',
    capture: () => this.record(),
    restore: (v) => void this.restore(v),
    reset: () => void this.restore(null),
  };
}

function isRecord(v: unknown): v is KnowledgeRecord {
  return !!v && typeof v === 'object' && typeof (v as KnowledgeRecord).version === 'number' && Array.isArray((v as KnowledgeRecord).files);
}
