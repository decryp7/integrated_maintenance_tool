import { inject, Injectable, signal } from '@angular/core';
import { Logger } from '../core/logger';
import { UiStateContribution } from '../lifecycle/ui-state';
import { AiBackend, toAiError } from './ai-backend';
import { ChatHistoryRecord } from './ai.types';

/**
 * The window's chats as part of the project and session UI state (key `chats`, `ChatHistoryRecord` v1).
 * The engine owns the chats; this keeps the last copy it returned so saving stays synchronous, and hands
 * saved chats back to the engine (which validates them: damaged or newer ones are dropped).
 */
@Injectable({ providedIn: 'root' })
export class AiChatPersistence {
  private readonly backend = inject(AiBackend);
  private readonly logger = inject(Logger);
  /** The last history the engine returned (null until the chat was used or a project restored it). */
  readonly history = signal<ChatHistoryRecord | null>(null);
  /** Bumped when saved chats were restored, so an open chat panel reloads. */
  readonly restored = signal(0);
  /** Why saved chats could not be loaded, if so. */
  readonly warning = signal<string | null>(null);

  /** Reads the current chats from the engine (after a turn, a new or deleted chat). */
  async sync(): Promise<void> {
    try {
      const h = await this.backend.chatHistory();
      this.history.set(h.sessions.length ? h : null);
    } catch (e) {
      this.logger.warn('ai', 'Chats could not be read for saving', { Code: toAiError(e).code });
    }
  }

  async restore(value: unknown): Promise<void> {
    try {
      const r = await this.backend.chatRestore(value ?? null);
      this.warning.set(r.warning);
      await this.sync();
    } catch (e) {
      // No AI in this run: the saved chats stay in the file untouched (they are captured back as they were).
      this.history.set(isHistory(value) ? value : null);
      this.logger.warn('ai', 'Saved chats not restored', { Code: toAiError(e).code });
    }
    this.restored.update((n) => n + 1);
  }

  readonly contribution: UiStateContribution = {
    key: 'chats',
    capture: () => this.history(),
    restore: (v) => void this.restore(v),
    reset: () => void this.restore(null),
  };
}

function isHistory(v: unknown): v is ChatHistoryRecord {
  return !!v && typeof v === 'object' && typeof (v as ChatHistoryRecord).version === 'number' && Array.isArray((v as ChatHistoryRecord).sessions);
}
