import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';
import { MockAiBackend } from '../core/mock/mock-ai-backend';
import {
  AiBackend,
  AiError,
  lazyAiBackend,
  loadAiBackend,
  provideAiBackend,
  toAiError,
} from './ai-backend';
import { TauriAiBackend } from './ai-backend-tauri';
import { AI_CHAT_EVENT, AI_RUNTIME_EVENT, AI_SETUP_EVENT, AI_STATUS_EVENT } from './ai.types';

describe('toAiError', () => {
  it('keeps AiErrors, reads {code, message} payloads and wraps anything else', () => {
    const e = new AiError('busy', 'b');
    expect(toAiError(e)).toBe(e);
    const p = toAiError({ code: 'noModel', message: 'set up first' });
    expect(p).toBeInstanceOf(AiError);
    expect([p.code, p.message, p.name]).toEqual(['noModel', 'set up first', 'AiError']);
    expect(toAiError(new Error('x'))).toMatchObject({ code: 'engine', message: 'x' });
    expect(toAiError('plain')).toMatchObject({ code: 'engine', message: 'plain' });
    expect(toAiError(42)).toMatchObject({ code: 'engine', message: 'The AI assistant failed.' });
    expect(toAiError({ code: 1, message: 'm' }).code).toBe('engine');
  });
});

describe('lazyAiBackend', () => {
  it('loads the real backend once, on the first call, and is not a thenable', async () => {
    const real = {
      status: vi.fn(async () => 's'),
      cancel: vi.fn(async () => 3),
    } as unknown as AiBackend;
    const load = vi.fn(async () => real);
    const b = lazyAiBackend(load);
    const raw = b as unknown as Record<PropertyKey, unknown>;
    expect([raw['then'], raw['ngOnDestroy'], raw[Symbol.iterator]]).toEqual([
      undefined,
      undefined,
      undefined,
    ]);
    expect(load).not.toHaveBeenCalled();
    expect(b.status).toBe(b.status);
    expect(await b.status()).toBe('s');
    expect(await b.cancel()).toBe(3);
    expect(load).toHaveBeenCalledTimes(1);
    // A proxy can be awaited (no `then`), as Angular does with injected values.
    expect(await Promise.resolve(b)).toBe(b);
  });

  it('loads the scripted mock in a plain browser', async () => {
    const b = await loadAiBackend();
    expect(b).toBeInstanceOf(MockAiBackend);
    const provider = provideAiBackend()[0] as unknown as { useFactory: () => AiBackend };
    TestBed.configureTestingModule({});
    expect(
      (await TestBed.runInInjectionContext(() => provider.useFactory()).status()).available,
    ).toBe(true);
  });
});

describe('TauriAiBackend', () => {
  const calls: [string, unknown][] = [];
  const invoke = vi.fn(async (cmd: string, args?: unknown) => {
    calls.push([cmd, args]);
    return cmd;
  });

  it('maps every method to its command and arguments', async () => {
    const b = new TauriAiBackend(invoke as never, vi.fn() as never);
    const cases: [() => Promise<unknown>, string, unknown][] = [
      [() => b.status(), 'ai_status', undefined],
      [() => b.hardware(), 'ai_hardware', undefined],
      [() => b.recommend(), 'ai_recommend', undefined],
      [() => b.catalog(), 'ai_catalog', undefined],
      [() => b.setupPlan(), 'ai_setup_plan', { modelId: null }],
      [() => b.setupPlan('m'), 'ai_setup_plan', { modelId: 'm' }],
      [() => b.setupRun('m'), 'ai_setup_run', { modelId: 'm' }],
      [() => b.setupCancel(), 'ai_setup_cancel', undefined],
      [() => b.models(), 'ai_models', undefined],
      [() => b.activateModel('f.gguf'), 'ai_model_activate', { fileName: 'f.gguf' }],
      [() => b.deleteModel('f.gguf'), 'ai_model_delete', { fileName: 'f.gguf' }],
      [() => b.addLocalModel('/p.gguf'), 'ai_model_add_local', { path: '/p.gguf' }],
      [() => b.reset(), 'ai_reset', undefined],
      [() => b.runtimes(), 'ai_runtimes', undefined],
      [() => b.runtimeInstall('cuda-13'), 'ai_runtime_install', { id: 'cuda-13' }],
      [() => b.runtimeCancel(), 'ai_runtime_cancel', undefined],
      [() => b.runtimeDelete('b1', 'cpu'), 'ai_runtime_delete', { tag: 'b1', id: 'cpu' }],
      [() => b.runtimeChoose('auto'), 'ai_runtime_choose', { id: 'auto' }],
      [
        () => b.runtimeAddArchives('cpu', ['/d/a.zip']),
        'ai_runtime_add_archives',
        { id: 'cpu', paths: ['/d/a.zip'] },
      ],
      [() => b.manualDownload('model', 'm'), 'ai_manual_download', { kind: 'model', id: 'm' }],
      [
        () => b.openManualDownload('runtime', 'cpu', 1),
        'ai_manual_download_open',
        { kind: 'runtime', id: 'cpu', index: 1 },
      ],
      [() => b.start({ op: 'query', question: 'q' }), 'ai_start', { request: { op: 'query', question: 'q' } }],
      [() => b.start({ op: 'explain', recordId: 3 }), 'ai_start', { request: { op: 'explain', recordId: 3 } }],
      [() => b.start({ op: 'rootCauses' }), 'ai_start', { request: { op: 'rootCauses' } }],
      [() => b.start({ op: 'regex', lines: ['a'], intent: null }), 'ai_start', { request: { op: 'regex', lines: ['a'], intent: null } }],
      [() => b.cancel(), 'ai_cancel', undefined],
      [() => b.chatSessions(), 'ai_chat_sessions', undefined],
      [() => b.chatNew(), 'ai_chat_new', undefined],
      [() => b.chatGet('c'), 'ai_chat_get', { sessionId: 'c' }],
      [() => b.chatDelete('c'), 'ai_chat_delete', { sessionId: 'c' }],
      [
        () => b.chatSend('c', 't', { selectedIds: [] }),
        'ai_chat_send',
        { sessionId: 'c', text: 't', context: { selectedIds: [] } },
      ],
      [() => b.chatCancel('c'), 'ai_chat_cancel', { sessionId: 'c' }],
      [() => b.chatEventsSince('c', 5), 'ai_chat_events_since', { sessionId: 'c', seq: 5 }],
      [() => b.chatHistory(), 'ai_chat_history', undefined],
      [() => b.chatRestore(undefined), 'ai_chat_restore', { record: null }],
      [() => b.chatRestore({ version: 1 }), 'ai_chat_restore', { record: { version: 1 } }],
      [
        () => b.chatExportMarkdown('c', '/a.md'),
        'ai_chat_export_markdown',
        { sessionId: 'c', path: '/a.md' },
      ],
      [
        () => b.chatExportMarkdown('c', '/a.md', true),
        'ai_chat_export_markdown',
        { sessionId: 'c', path: '/a.md', overwrite: true },
      ],
      [() => b.suggestedPrompts(), 'ai_suggested_prompts', undefined],
      [() => b.openLink('https://x'), 'ai_open_link', { url: 'https://x' }],
    ];
    for (const [run, cmd, args] of cases) {
      calls.length = 0;
      expect(await run()).toBe(cmd);
      expect(calls).toEqual([[cmd, args]]);
    }
  });

  it('turns rejections into AiErrors', async () => {
    const b = new TauriAiBackend(
      (async () => Promise.reject({ code: 'unavailable', message: 'off' })) as never,
      vi.fn() as never,
    );
    await expect(b.status()).rejects.toMatchObject({ code: 'unavailable', message: 'off' });
    await expect(b.status()).rejects.toBeInstanceOf(AiError);
  });

  it('listens to the AI events and drops malformed payloads', async () => {
    const handlers = new Map<string, (p: unknown) => void>();
    const off = vi.fn();
    const listen = vi.fn(async (event: string, h: (p: unknown) => void) => {
      handlers.set(event, h);
      return off;
    });
    const b = new TauriAiBackend(invoke as never, listen);
    const setup = vi.fn();
    const status = vi.fn();
    const chat = vi.fn();
    const runtime = vi.fn();
    expect(await b.onSetup(setup)).toBe(off);
    await b.onStatus(status);
    await b.onChat(chat);
    await b.onRuntime(runtime);
    for (const bad of [
      null,
      'x',
      [],
      { stage: 1 },
      { state: 'ready' },
      { sessionId: 'c', type: 'delta' },
    ]) {
      handlers.get(AI_SETUP_EVENT)!(bad);
      handlers.get(AI_STATUS_EVENT)!(bad);
      handlers.get(AI_CHAT_EVENT)!(bad);
      handlers.get(AI_RUNTIME_EVENT)!(bad);
    }
    expect([setup, status, chat, runtime].map((f) => f.mock.calls.length)).toEqual([0, 0, 0, 0]);
    handlers.get(AI_SETUP_EVENT)!({ stage: 'probing' });
    handlers.get(AI_STATUS_EVENT)!({ state: 'ready', available: true });
    handlers.get(AI_CHAT_EVENT)!({ sessionId: 'c', type: 'delta', seq: 1, text: 'x' });
    handlers.get(AI_RUNTIME_EVENT)!({ stage: 'finishing', id: 'cpu' });
    expect([setup, status, chat, runtime].map((f) => f.mock.calls.length)).toEqual([1, 1, 1, 1]);
  });
});
