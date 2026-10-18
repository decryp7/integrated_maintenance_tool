import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { FakeContext, provideAnalysisTesting } from '../../analysis/test-setup';
import { TimeCursor } from '../../analysis/time-cursor';
import { RecordView } from '../../core/api.types';
import { MockAiBackend, MockAiOptions } from '../../core/mock/mock-ai-backend';
import { WorkspaceStore } from '../../core/workspace.store';
import { GridSelection } from '../../grid/grid-selection';
import { settle } from '../../../testing/helpers';
import { AiBackend, AiError } from '../ai-backend';
import { AiChatPersistence } from '../ai-chat-persistence';
import { ChatStore } from './chat.store';
import { at } from '../../../testing/must';

function setup(options: MockAiOptions = {}) {
  const backend = new MockAiBackend({ stepMs: 0, ...options });
  TestBed.configureTestingModule({ providers: [...provideAnalysisTesting({ context: new FakeContext() }), { provide: AiBackend, useValue: backend }] });
  const store = TestBed.inject(ChatStore);
  return { backend, store };
}

const done = (store: InstanceType<typeof ChatStore>) => vi.waitFor(() => expect(store.running()).toBe(false));

describe('ChatStore', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('starts a chat when there is none and loads starter prompts', async () => {
    const { store } = setup();
    await store.init();
    expect(store.sessions()).toHaveLength(1);
    expect(store.active()?.id).toBe(store.activeId());
    await vi.waitFor(() => expect(store.prompts().length).toBeGreaterThan(2));
    expect(store.activeTurns()).toEqual([]);
    expect(store.loading()).toBe(false);
  });

  it('sends the context the user chose and folds the streamed answer', async () => {
    const { store, backend } = setup();
    await store.init();
    const ws = TestBed.inject(WorkspaceStore);
    ws.select({ id: 3 } as RecordView);
    ws.patchQuery({ ql: 'level >= error', groups: ['WEB01'] });
    TestBed.inject(TimeCursor).select({ from: 1, to: 9 });
    expect(await store.send('  what crashed?  ')).toBe(true);
    expect(store.running()).toBe(true);
    expect(await store.send('again')).toBe(false);
    await done(store);
    expect(backend.sent[0]).toEqual({
      sessionId: store.activeId(),
      text: 'what crashed?',
      context: { query: 'level >= error', timeRange: [1, 9], selectedIds: [3], focusedInstance: 'WEB01', openView: 'events grid' },
    });
    const t = at(store.activeTurns(), 0);
    expect(t.tools[0]).toMatchObject({ name: 'run_query', running: false, summary: '1 events' });
    expect(t.text).toContain('OutOfMemoryException');
    expect(t.citations.length).toBe(2);
    await vi.waitFor(() => expect(store.sessions()[0]?.title).toBe('what crashed?'));
    // Grid multi-selection wins; toggles leave things out.
    TestBed.inject(GridSelection).publish([{ id: 4 } as RecordView, { id: 5 } as RecordView]);
    store.toggle('filter');
    store.toggle('range');
    await store.send('more');
    await done(store);
    expect(backend.sent[1]?.context).toMatchObject({ query: null, timeRange: null, selectedIds: [4, 5] });
    // A query range when nothing is brushed.
    TestBed.inject(TimeCursor).select(null);
    store.toggle('range');
    ws.setRange(100, 200);
    await store.send('range');
    await done(store);
    expect(backend.sent[2]?.context.timeRange).toEqual([100, 200]);
    expect(await store.send('   ')).toBe(false);
  });

  it('ignores replayed sequence numbers and events of finished turns', async () => {
    const { store } = setup();
    await store.init();
    const id = store.activeId()!;
    store.onEvent({ sessionId: id, seq: 1, type: 'delta', text: 'ignored: no running turn' });
    expect(store.activeTurns()).toEqual([]);
    await store.send('q');
    await done(store);
    const text = at(store.activeTurns(), 0).text;
    store.onEvent({ sessionId: id, seq: 1, type: 'delta', text: 'dup' });
    expect(store.activeTurns()[0]?.text).toBe(text);
  });

  it('shows a failed send and removes the pending turn', async () => {
    const { store, backend } = setup();
    await store.init();
    vi.spyOn(backend, 'chatSend').mockRejectedValueOnce(new AiError('busy', 'still answering'));
    expect(await store.send('q')).toBe(false);
    expect(store.activeTurns()).toEqual([]);
    expect(store.error()).toBe('still answering');
    store.dismissError();
    expect(store.error()).toBeNull();
  });

  it('reports a failing init', async () => {
    const { store, backend } = setup();
    vi.spyOn(backend, 'chatSessions').mockRejectedValue(new AiError('unavailable', 'off'));
    await store.init();
    expect(store.error()).toBe('off');
    expect(await store.send('q')).toBe(false);
  });

  it('keeps empty prompts when they cannot be read', async () => {
    const { store, backend } = setup();
    vi.spyOn(backend, 'suggestedPrompts').mockRejectedValue(new AiError('engine', 'x'));
    await store.init();
    await settle();
    expect(store.prompts()).toEqual([]);
  });

  it('cancels a streaming answer', async () => {
    const { store } = setup({ stepMs: 5 });
    await store.init();
    await store.send('tell me');
    await vi.waitFor(() => expect(store.activeTurns()[0]?.text.length).toBeGreaterThan(0));
    await store.cancel();
    await done(store);
    expect(store.activeTurns()[0]?.cancelled).toBe(true);
  });

  it('answers web-search approvals', async () => {
    const { store, backend } = setup({ web: true });
    await store.init();
    await store.respond(1, { kind: 'allow' });
    await store.send('search the web for this');
    await vi.waitFor(() => expect(store.activeTurns()[0]?.approvals[0]?.state).toBe('pending'));
    const fail = vi.spyOn(backend, 'approvalRespond').mockRejectedValueOnce(new AiError('invalid', 'empty'));
    await store.respond(1, { kind: 'allowEdited', text: '' });
    expect(store.activeTurns()[0]?.approvals[0]?.state).toBe('pending');
    expect(store.error()).toBe('empty');
    fail.mockRestore();
    await store.respond(1, { kind: 'deny' });
    await done(store);
    expect(store.activeTurns()[0]?.approvals[0]?.state).toBe('denied');
    expect(store.activeTurns()[0]?.text).toContain('declined');
  });

  it('manages several chats', async () => {
    const { store, backend } = setup();
    await store.init();
    const first = store.activeId()!;
    await store.send('q');
    await done(store);
    await store.newChat();
    const second = store.activeId()!;
    expect(second).not.toBe(first);
    expect(store.activeTurns()).toEqual([]);
    // Selecting a chat not loaded yet reads its turns.
    await store.reload();
    expect(store.activeId()).toBe(second);
    await store.select(first);
    expect(store.activeTurns()).toHaveLength(1);
    store.setProposalState(0, 0, 'applied');
    await store.deleteChat(first);
    expect(store.sessions().map((s) => s.id)).toEqual([second]);
    expect(store.activeId()).toBe(second);
    await store.deleteChat(second);
    expect(store.sessions()).toHaveLength(1);
    expect(store.activeId()).not.toBe(second);
    vi.spyOn(backend, 'chatDelete').mockRejectedValueOnce(new AiError('engine', 'x'));
    await store.deleteChat('x');
    expect(store.error()).toBe('x');
    vi.spyOn(backend, 'chatNew').mockRejectedValueOnce(new AiError('busy', 'too many'));
    await store.newChat();
    expect(store.error()).toBe('too many');
    vi.spyOn(backend, 'chatGet').mockRejectedValueOnce(new AiError('notFound', 'gone'));
    await store.select('chat-99');
    expect(store.error()).toBe('gone');
  });

  it('deleting the active chat moves to another that was not loaded', async () => {
    const { store, backend } = setup();
    await backend.chatNew();
    await backend.chatNew();
    await store.init();
    expect(store.activeId()).toBe('chat-2');
    await store.deleteChat('chat-2');
    expect(store.activeId()).toBe('chat-1');
    expect(store.activeTurns()).toEqual([]);
  });

  it('marks proposals applied or dismissed', async () => {
    const { store } = setup();
    await store.init();
    await store.send('write a regex');
    await done(store);
    store.setProposalState(0, 0, 'dismissed');
    expect(store.activeTurns()[0]?.proposals[0]?.state).toBe('dismissed');
    store.setProposalState(1, 0, 'applied');
    expect(store.activeTurns()[0]?.proposals[0]?.state).toBe('dismissed');
  });

  it('catches up with a running turn and reloads when it is too old', async () => {
    const { store, backend } = setup({ stepMs: 20 });
    const s = await backend.chatNew();
    await backend.chatSend(s.id, 'still running', { selectedIds: [] });
    await store.init();
    expect(store.activeTurns().at(-1)?.running).toBe(true);
    await done(store);
    vi.spyOn(backend, 'chatEventsSince').mockResolvedValueOnce(null);
    await store.resume(s.id);
    expect(store.activeTurns()).toHaveLength(1);
  });

  it('reloads after saved chats were restored', async () => {
    const { store } = setup();
    await store.init();
    const p = TestBed.inject(AiChatPersistence);
    await p.restore({ version: 1, sessions: [{ id: 'chat-5', title: 'Saved', turns: [{ user: 'u', assistant: 'a', citations: [], proposals: [], web: [], cancelled: false, usage: { inputTokens: 0, outputTokens: 0 } }], allowSearches: false }] });
    await store.reload();
    expect(store.activeId()).toBe('chat-5');
    expect(store.activeTurns()[0]?.text).toBe('a');
    store.setNotice('n');
    expect(store.notice()).toBe('n');
  });

  it('stops listening to the chat stream when destroyed', async () => {
    for (const early of [false, true]) {
      const backend = new MockAiBackend({ stepMs: 0 });
      const off = vi.fn();
      let resolve!: (u: () => void) => void;
      vi.spyOn(backend, 'onChat').mockReturnValue(new Promise((r) => (resolve = r)));
      TestBed.configureTestingModule({ providers: [...provideAnalysisTesting({ context: new FakeContext() }), { provide: AiBackend, useValue: backend }] });
      TestBed.inject(ChatStore);
      if (!early) {
        resolve(off);
        await settle();
      }
      TestBed.resetTestingModule();
      if (early) {
        resolve(off);
        await settle();
      }
      expect(off).toHaveBeenCalledTimes(1);
    }
    // A backend that cannot subscribe leaves the store usable.
    TestBed.resetTestingModule();
    const failing = new MockAiBackend({ stepMs: 0 });
    vi.spyOn(failing, 'onChat').mockRejectedValue(new Error('no events'));
    TestBed.configureTestingModule({ providers: [...provideAnalysisTesting({ context: new FakeContext() }), { provide: AiBackend, useValue: failing }] });
    await TestBed.inject(ChatStore).init();
    await settle();
    expect(TestBed.inject(ChatStore).sessions()).toHaveLength(1);
  });

  it('does nothing without an active chat', async () => {
    const { store } = setup();
    await store.cancel();
    await store.respond(1, { kind: 'allow' });
    store.setProposalState(0, 0, 'applied');
    expect(await store.send('q')).toBe(false);
  });
});
