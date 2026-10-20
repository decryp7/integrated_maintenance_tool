import { TestBed } from '@angular/core/testing';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Logger } from '../core/logger';
import { MemoryLogger, settle } from '../../testing/helpers';
import { BrowserBackend, ChatBrowserBackend, TauriBrowserBackend } from './browser-backend';
import { BrowserStore, MAX_NOTICES } from './browser.store';
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
  return { backend, web, browser: TestBed.inject(BrowserStore), chats: TestBed.inject(ChatBrowserStore) };
}

describe('TauriBrowserBackend', () => {
  it('maps every method to its command', async () => {
    const calls: [string, unknown][] = [];
    const call = (async (cmd: string, args?: unknown) => {
      calls.push([cmd, args]);
      return cmd;
    }) as never;
    const handlers: ((p: unknown) => void)[] = [];
    const b = new TauriBrowserBackend(call, async (event, h) => {
      expect(event).toBe('imt://browser');
      handlers.push(h);
      return () => undefined;
    });
    const cases: [() => Promise<unknown>, string, unknown][] = [
      [() => b.state(), 'browser_state', undefined],
      [() => b.open({ kind: 'search', query: 'x' }), 'browser_open', { target: { kind: 'search', query: 'x' } }],
      [() => b.eventQuery(4), 'browser_event_query', { recordId: 4 }],
      [() => b.nav('back'), 'browser_nav', { command: 'back' }],
      [() => b.close(), 'browser_close', undefined],
      [() => b.researchStop(), 'browser_research_stop', undefined],
      [() => b.driving(), 'browser_driving', undefined],
      [() => b.openExternal('https://e.org/'), 'browser_open_external', { url: 'https://e.org/' }],
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
    handlers[0]?.({ type: 'notice', kind: 'popup', url: 'u', message: '' });
    handlers[0]?.('junk');
    handlers[0]?.(null);
    expect(seen).toHaveLength(1);
  });
});

describe('BrowserStore', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('opens pages and searches and follows the page shown', async () => {
    const { browser, web } = setup();
    expect(browser.engine()).toBeNull();
    await browser.load();
    expect(browser.loaded() && !browser.page().open).toBe(true);
    expect(browser.engine()).toBe(web.engine);
    expect(await browser.go('connection reset by peer')).toBe(true);
    await settle();
    expect(browser.page().url).toBe(mockSearchUrl('google', 'connection reset by peer'));
    browser.setEngine('bing');
    await browser.go('learn.microsoft.com/dotnet');
    await settle();
    expect(browser.page().url).toBe('https://learn.microsoft.com/dotnet');
    await browser.open({ kind: 'search', query: 'x', engine: 'stackOverflow' });
    await settle();
    expect(browser.page().url).toContain('stackoverflow.com');
    await browser.nav('back');
    await settle();
    expect(browser.page().url).toBe('https://learn.microsoft.com/dotnet');
    await browser.nav('forward');
    await browser.nav('reload');
    await settle();
    expect(web.history).toHaveLength(3);
    expect(await browser.openExternal(browser.page().url)).toBe(true);
    expect(web.external).toEqual([web.page.url]);
    await browser.close();
    await settle();
    expect(browser.page().open).toBe(false);
    // Loading again keeps the engine the user picked.
    await browser.load();
    expect(browser.engine()).toBe('bing');
  });

  it('keeps failures for the inline error and retries them', async () => {
    const { browser, backend } = setup();
    expect(await browser.go('javascript:alert(1)')).toBe(false);
    expect(browser.error()).toContain('http and https');
    browser.dismissError();
    expect(browser.error()).toBeNull();
    const fail = vi.spyOn(backend, 'state').mockRejectedValueOnce('engine down');
    await browser.load();
    expect(browser.error()).toBe('engine down');
    await browser.retry();
    expect(fail).toHaveBeenCalledTimes(2);
    expect(browser.error()).toBeNull();
    vi.spyOn(backend, 'nav').mockRejectedValueOnce({});
    await browser.nav('reload');
    expect(browser.error()).toBe('The browser did not answer.');
  });

  it('collects notices, newest last, without duplicates', async () => {
    const { browser, web } = setup();
    await settle();
    for (let i = 0; i < MAX_NOTICES + 2; i++) web.emit({ type: 'notice', kind: 'popup', url: `https://p${i}.example/`, message: '' });
    web.emit({ type: 'notice', kind: 'popup', url: `https://p${MAX_NOTICES + 1}.example/`, message: '' });
    expect(browser.notices()).toHaveLength(MAX_NOTICES);
    expect(browser.notices().at(-1)?.url).toBe(`https://p${MAX_NOTICES + 1}.example/`);
    browser.dismiss(browser.notices()[0]!.id);
    expect(browser.notices()).toHaveLength(MAX_NOTICES - 1);
    web.emit({ type: 'sources', sessionId: 'x', sources: [] });
    expect(browser.notices()).toHaveLength(MAX_NOTICES - 1);
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
    const { chats, browser } = setup(web);
    expect(await chats.capture('chat-1')).toBe(false);
    expect(chats.error()).toContain('not open');
    await browser.go('https://learn.example.com/a');
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
