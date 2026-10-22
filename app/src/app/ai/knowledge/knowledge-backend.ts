import { inject, Injectable } from '@angular/core';
import { invoke } from '../../core/activity/ipc';
import { reportBackendFailures } from '../../core/failure-reporter';
import { Logger, withFailureLogging } from '../../core/logger';
import { browserMocks } from '../../core/mock-registry';
import { isTauri } from '../../core/platform';
import { toAiError } from '../ai-backend';
import { KnowledgeChange, KnowledgeInfo, KnowledgeList, KnowledgeRecord, KnowledgeRestore, KnowledgeScope, KnowledgeText } from '../ai.types';

/**
 * The knowledge files of the AI (ADR 0029): Markdown notes the engine adds to every AI request. The engine checks
 * names, sizes and limits, stores the global files and owns the project record; the UI only names ids. Injected as a
 * class token: the Tauri commands in the app, the in-memory mock in a plain browser (dev server, e2e tests).
 */
@Injectable({ providedIn: 'root', useFactory: createKnowledgeBackend })
export abstract class KnowledgeBackend {
  /** Both scopes' files (the project's are this window's). */
  abstract listKnowledge(): Promise<KnowledgeList>;
  abstract getKnowledge(scope: KnowledgeScope, id: string): Promise<KnowledgeText>;
  abstract createKnowledge(scope: KnowledgeScope, name: string, text?: string): Promise<KnowledgeInfo>;
  /** Imports a `.md`/`.txt` file the user chose in the open dialog, named after the file. */
  abstract importKnowledge(scope: KnowledgeScope, path: string): Promise<KnowledgeInfo>;
  abstract updateKnowledge(scope: KnowledgeScope, id: string, change: KnowledgeChange): Promise<KnowledgeInfo>;
  abstract removeKnowledge(scope: KnowledgeScope, id: string): Promise<boolean>;
  /** Writes the file's text to a path the user chose in the save dialog; refuses to replace one unless `overwrite`. */
  abstract exportKnowledge(scope: KnowledgeScope, id: string, path: string, overwrite?: boolean): Promise<string>;
  /** The window's project knowledge as saved in projects and sessions. */
  abstract getProjectKnowledge(): Promise<KnowledgeRecord>;
  /** Replaces the window's project knowledge with a saved record (null clears it). */
  abstract restoreProjectKnowledge(record: unknown): Promise<KnowledgeRestore>;
}

/** Talks to `app/src-tauri/src/ai/knowledge/commands.rs`; rejections become `AiError`s. */
export class TauriKnowledgeBackend extends KnowledgeBackend {
  /** `call` is Tauri's `invoke`; tests pass a fake. */
  constructor(private readonly call: typeof invoke = invoke) {
    super();
  }

  private async run<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
    try {
      return await this.call<T>(cmd, args);
    } catch (e) {
      throw toAiError(e);
    }
  }

  listKnowledge() {
    return this.run<KnowledgeList>('ai_knowledge_list');
  }
  getKnowledge(scope: KnowledgeScope, id: string) {
    return this.run<KnowledgeText>('ai_knowledge_get', { scope, id });
  }
  createKnowledge(scope: KnowledgeScope, name: string, text = '') {
    return this.run<KnowledgeInfo>('ai_knowledge_create', { scope, name, text });
  }
  importKnowledge(scope: KnowledgeScope, path: string) {
    return this.run<KnowledgeInfo>('ai_knowledge_import', { scope, path });
  }
  updateKnowledge(scope: KnowledgeScope, id: string, change: KnowledgeChange) {
    return this.run<KnowledgeInfo>('ai_knowledge_update', { scope, id, change });
  }
  removeKnowledge(scope: KnowledgeScope, id: string) {
    return this.run<boolean>('ai_knowledge_remove', { scope, id });
  }
  exportKnowledge(scope: KnowledgeScope, id: string, path: string, overwrite = false) {
    return this.run<string>('ai_knowledge_export', { scope, id, path, ...(overwrite ? { overwrite } : {}) });
  }
  getProjectKnowledge() {
    return this.run<KnowledgeRecord>('ai_knowledge_project_record');
  }
  restoreProjectKnowledge(record: unknown) {
    return this.run<KnowledgeRestore>('ai_knowledge_project_restore', { record: record ?? null });
  }
}

export function createKnowledgeBackend(): KnowledgeBackend {
  const backend: KnowledgeBackend = isTauri() ? new TauriKnowledgeBackend() : new (browserMocks().MockKnowledgeBackend)(globalThis.location?.search ?? '');
  return withFailureLogging<KnowledgeBackend>(backend, inject(Logger), { category: 'ui.ai', onFailure: reportBackendFailures() });
}
