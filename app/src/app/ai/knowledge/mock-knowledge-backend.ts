import { AiError } from '../ai-backend';
import { KnowledgeChange, KnowledgeInfo, KnowledgeList, KnowledgeRecord, KnowledgeRestore, KnowledgeScope, KnowledgeText } from '../ai.types';
import type { KnowledgeBackend } from './knowledge-backend';

const MAX_FILE_BYTES = 64 * 1024;
const MAX_FILES = 32;

interface File {
  id: string;
  name: string;
  enabled: boolean;
  text: string;
}

const bytes = (s: string) => new TextEncoder().encode(s).length;

/**
 * In-memory knowledge files shaped like the engine's (browser development and UI tests only; the engine is the
 * reference for the checks). `?mockKnowledge=1` starts with a global glossary.
 */
export class MockKnowledgeBackend implements KnowledgeBackend {
  private readonly files: Record<KnowledgeScope, File[]> = { project: [], global: [] };

  constructor(search = '') {
    if (new URLSearchParams(search).get('mockKnowledge') === '1') {
      this.files.global.push({ id: 'k1', name: 'Glossary', enabled: true, text: '# Glossary\n\n- **ACME-GW**: the payment gateway.\n' });
    }
  }

  private info(scope: KnowledgeScope, f: File): KnowledgeInfo {
    return { id: f.id, scope, name: f.name, enabled: f.enabled, bytes: bytes(f.text) };
  }

  private find(scope: KnowledgeScope, id: string): File {
    const f = this.files[scope].find((x) => x.id === id);
    if (!f) throw new AiError('notFound', 'not found: this knowledge file no longer exists');
    return f;
  }

  private checked(name: string | undefined, text: string | undefined): void {
    if (name !== undefined && !name.trim()) throw new AiError('invalid', 'give the knowledge file a name');
    if (text !== undefined && bytes(text) > MAX_FILE_BYTES) throw new AiError('invalid', 'the knowledge file is too large (at most 64 KiB): split it or shorten it');
  }

  async listKnowledge(): Promise<KnowledgeList> {
    return {
      project: this.files.project.map((f) => this.info('project', f)),
      global: this.files.global.map((f) => this.info('global', f)),
      warnings: [],
      globalReadOnly: false,
      maxFileBytes: MAX_FILE_BYTES,
      maxFiles: MAX_FILES,
    };
  }

  async getKnowledge(scope: KnowledgeScope, id: string): Promise<KnowledgeText> {
    const f = this.find(scope, id);
    return { ...this.info(scope, f), text: f.text };
  }

  async createKnowledge(scope: KnowledgeScope, name: string, text = ''): Promise<KnowledgeInfo> {
    this.checked(name, text);
    const list = this.files[scope];
    if (list.length >= MAX_FILES) throw new AiError('invalid', `there are already ${MAX_FILES} knowledge files here`);
    let n = 1;
    while (list.some((f) => f.id === `k${n}`)) n++;
    const f = { id: `k${n}`, name: name.trim(), enabled: true, text: text.replaceAll('\r\n', '\n') };
    list.push(f);
    return this.info(scope, f);
  }

  async importKnowledge(scope: KnowledgeScope, path: string): Promise<KnowledgeInfo> {
    const name = (path.split(/[\\/]/).pop() ?? 'Imported').replace(/\.(md|markdown|txt)$/i, '') || 'Imported';
    return this.createKnowledge(scope, name, `# ${name}\n\nImported from ${path}.\n`);
  }

  async updateKnowledge(scope: KnowledgeScope, id: string, change: KnowledgeChange): Promise<KnowledgeInfo> {
    this.checked(change.name, change.text);
    const f = this.find(scope, id);
    if (change.name !== undefined) f.name = change.name.trim();
    if (change.text !== undefined) f.text = change.text.replaceAll('\r\n', '\n');
    if (change.enabled !== undefined) f.enabled = change.enabled;
    return this.info(scope, f);
  }

  async removeKnowledge(scope: KnowledgeScope, id: string): Promise<boolean> {
    this.find(scope, id);
    this.files[scope] = this.files[scope].filter((f) => f.id !== id);
    return true;
  }

  async exportKnowledge(scope: KnowledgeScope, id: string, path: string): Promise<string> {
    this.find(scope, id);
    return path;
  }

  async getProjectKnowledge(): Promise<KnowledgeRecord> {
    return { version: 1, files: this.files.project.map((f) => ({ ...f })) };
  }

  async restoreProjectKnowledge(record: unknown): Promise<KnowledgeRestore> {
    const r = record as Partial<KnowledgeRecord> | null;
    if (r && (typeof r.version !== 'number' || r.version > 1)) {
      this.files.project = [];
      return { files: [], warning: "The project's knowledge files are damaged and were not loaded." };
    }
    this.files.project = (r?.files ?? []).map((f) => ({ ...f }));
    return { files: this.files.project.map((f) => this.info('project', f)), warning: null };
  }
}
