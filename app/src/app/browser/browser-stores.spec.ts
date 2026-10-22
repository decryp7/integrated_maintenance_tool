import { TestBed } from '@angular/core/testing';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Logger } from '../core/logger';
import { MemoryLogger, settle } from '../../testing/helpers';
import { BrowserBackend, ChatBrowserBackend, TauriBrowserBackend } from './browser-backend';
import { BrowserActivityStore } from './browser-activity.store';
import { BrowserEvent } from './browser.types';
import { ChatBrowserStore } from './chat-browser.store';
import { MockBrowserBackend, mockSearchUrl, MockWeb, resetMockWeb } from './mock-browser-backend';

function setup(web: MockWeb = resetMockWeb()) {
  const backend = new MockBrowserBackend(web);
  TestBed.configureTestingModule({
    providers: [
      { provide: BrowserBackend, useValue: backend },
      { provide: ChatBrowserBackend, useValue: backend },
      { provide: Logger, useClass: MemoryLogger },
    ],
  });
  return { backend, web, activity: TestBed.inject(BrowserActivityStore), chats: TestBed.inject(ChatBrowserStore) };
}

describe('TauriBrowserBackend', () => {
  it('stops research by cancelling the active research jobs of the window', async () => {
    const calls: [string, unknown][] = [];
    const job = (id: number, kind: string, state: string) => ({ id, kind, state });
    const call = (async (cmd: string, args?: unknown) => {
      calls.push([cmd, args]);
      return cmd === 'jobs_list' ? { jobs: [job(1, 'research', 'running'), job(2, 'research', 'done'), job(3, 'import', 'running')], paused: [] } : undefined;
    }) as never;
    const b = new TauriBrowserBackend(call, async () => () => undefined);
    expect(await b.researchStop()).toBe(1);
    expect(calls).toEqual([
      ['jobs_list', undefined],
      ['jobs_cancel', { id: 1 }],
    ]);
  });

  it('maps every method to its command', async () => {
    const calls: [string, unknown][] = [];
    const call = (async (cmd: string, args?: unknown) => {
      calls.push([cmd, args]);
      return cmd;
    }) as never;
    const handlers: ((p: unknown) => void)[] = [];
    const b = new TauriBrowserBackend(call, async (event, h) => {
      expect(event).toBe('debrief://browser');
      handlers.push(h);
      return () => undefined;
    });
    const cases: [() => Promise<unknown>, string, unknown][] = [
      [() => b.open({ kind: 'search', query: 'x' }), 'browser_open', { target: { kind: 'search', query: 'x' } }],
      [() => b.eventQuery(4), 'browser_event_query', { recordId: 4 }],
      [() => b.driving(), 'browser_driving', undefined],
      [() => b.stopAll(), 'browser_stop', undefined],
      [() => b.capture('c'), 'browser_capture', { sessionId: 'c' }],
      [() => b.sources('c'), 'browser_sources', { sessionId: 'c' }],
      [() => b.discardSources('c'), 'browser_discard_sources', { sessionId: 'c' }],
      [() => b.status('c'), 'browser_chat_status', { sessionId: 'c' }],
      [() => b.setReadAlong('c', true), 'browser_set_read_along', { sessionId: 'c', on: true }],
      [() => b.setAgent('c', false), 'browser_set_agent', { sessionId: 'c', on: false }],
      [() => b.stop('c'), 'browser_agent_stop', { sessionId: 'c' }],
      [() => b.continueTask('c'), 'browser_agent_continue', { sessionId: 'c' }],
      [() => b.respond('c', 2, 'allowSite'), 'browser_agent_respond', { sessionId: 'c', requestId: 2, decision: 'allowSite' }],
      [() => b.openSuggested('c', 'https://e.org/'), 'browser_open_suggested', { sessionId: 'c', url: 'https://e.org/' }],
    ];
    for (const [run, cmd, args] of cases) {
      calls.length = 0;
      expect(await run()).toBe(cmd);
      expect(calls).toEqual([[cmd, args]]);
    }
    const seen: BrowserEvent[] = [];
    await b.onEvent((e) => seen.push(e));
    handlers[0]?.({ type: 'status', text: 'Reading 1 of 3 results' });
    handlers[0]?.('junk');
    handlers[0]?.(null);
    expect(seen).toHaveLength(1);
  });
});

describe('BrowserActivityStore', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('follows who drives and what the AI window does, and forgets it when the AI lets go', async () => {
    const { activity, web } = setup();
    await settle();
    expect(activity.driver()).toBeNull();
    web.drive({ kind: 'research' });
    web.report('reading', 'learn.example.com', 'Reading 3 of 5 results');
    expect(activity.driver()).toEqual({ kind: 'research' });
    expect(activity.text()).toBe('Reading 3 of 5 results');
    expect(activity.activity()).toBe('reading');
    expect(activity.host()).toBe('learn.example.com');
    web.report('waitingForYou', 'bank.example', undefined);
    expect(activity.activity()).toBe('waitingForYou');
    expect(activity.text()).toBe('Reading 3 of 5 results');
    web.drive(null);
    expect(activity.driver()).toBeNull();
    expect([activity.text(), activity.activity(), activity.host()]).toEqual([null, null, null]);
  });

  it('starts from who drives now, but an event wins over a slower answer', async () => {
    const web = resetMockWeb();
    web.driver = { kind: 'agent', sessionId: 'c-1' };
    const { activity } = setup(web);
    await settle();
    expect(activity.driver()).toEqual({ kind: 'agent', sessionId: 'c-1' });
    activity.onEvent({ type: 'driving', by: null });
    web.driver = { kind: 'research' };
    await activity.load();
    expect(activity.driver()).toBeNull();
  });

  it('stays hidden when the engine does not answer', async () => {
    const { activity, backend } = setup();
    vi.spyOn(backend, 'driving').mockRejectedValueOnce(new Error('down'));
    await activity.load();
    expect(activity.driver()).toBeNull();
  });

  it('the mock opens addresses in the default browser and stops everything at once', async () => {
    const { backend, web, chats } = setup();
    expect(await backend.open({ kind: 'search', query: 'a b' })).toBe(mockSearchUrl('google', 'a b'));
    expect(await backend.open({ kind: 'address', text: 'learn.microsoft.com/dotnet', engine: 'bing' })).toBe('https://learn.microsoft.com/dotnet');
    expect(web.external).toHaveLength(2);
    await expect(backend.open({ kind: 'address', text: 'javascript:alert(1)' })).rejects.toThrow('http and https');
    await expect(backend.open({ kind: 'search', query: ' ' })).rejects.toThrow('empty');
    expect(await backend.stopAll()).toBe(false);
    await chats.setAgent('chat-1', true);
    web.drive({ kind: 'agent', sessionId: 'chat-1' });
    expect(await backend.stopAll()).toBe(true);
    expect(web.driver).toBeNull();
    expect(web.status('chat-1').agent).toBe(false);
  });
});

describe('ChatBrowserStore', () => {
  let web: MockWeb;
  beforeEach(() => (web = resetMockWeb()));
  afterEach(() => TestBed.resetTestingModule());

  it('switches read-along and agent mode per chat, with Stop and Continue', async () => {
    const { chats } = setup(web);
    await chats.load('chat-1');
    expect(chats.statusOf('chat-1')?.agent).toBe(false);
    expect(chats.statusOf(null)).toBeNull();
    await chats.setReadAlong('chat-1', true);
    await chats.setAgent('chat-1', true);
    expect(chats.statusOf('chat-1')).toMatchObject({ readAlong: true, agent: true, stepLimit: 25 });
    expect(chats.statusOf('chat-2')).toBeNull();
    web.update('chat-1', { stepsUsed: 25, paused: true });
    await settle();
    expect(chats.statusOf('chat-1')?.paused).toBe(true);
    await chats.continueTask('chat-1');
    expect(chats.statusOf('chat-1')).toMatchObject({ paused: false, stepLimit: 50 });
    const waiting = web.ask('chat-1', 1);
    await chats.stop('chat-1');
    expect(await waiting).toBe('deny');
    expect(chats.statusOf('chat-1')?.agent).toBe(false);
    expect(await chats.continueTask('chat-1')).toBeNull();
    expect(chats.error()).toContain('agent mode is off');
    chats.dismissError();
    await chats.setAgent('chat-1', false);
  });

  it('adds pages for the next question and answers approvals', async () => {
    const { chats } = setup(web);
    expect(await chats.capture('chat-1')).toBe(false);
    expect(chats.error()).toContain('No page is open');
    web.show('https://learn.example.com/a');
    expect(await chats.capture('chat-1')).toBe(true);
    expect(chats.sourcesOf('chat-1').map((s) => s.url)).toEqual(['https://learn.example.com/a']);
    expect(chats.sourcesOf(null)).toEqual([]);
    chats.sent('chat-1');
    expect(chats.sourcesOf('chat-1')).toEqual([]);
    await chats.capture('chat-1');
    await chats.discardSources('chat-1');
    expect(chats.sourcesOf('chat-1')).toEqual([]);
    const answer = web.ask('chat-1', 3);
    expect(await chats.respond('chat-1', 3, 'allowSite')).toBe(true);
    expect(await answer).toBe('allowSite');
    expect(await chats.respond('chat-1', 3, 'allow')).toBe(false);
    expect(await chats.openSuggested('chat-1', 'https://e.org/')).toBe(false);
    web.suggested.set('chat-1', new Set(['https://e.org/']));
    expect(await chats.openSuggested('chat-1', 'https://e.org/')).toBe(true);
    web.forget('chat-1');
    expect(web.statuses.has('chat-1')).toBe(false);
  });
});
