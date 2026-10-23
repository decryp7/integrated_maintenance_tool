import { Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AiBackend } from '../ai/ai-backend';
import { AgentApprovalView, LinkView, PauseView } from '../ai/chat/chat-model';

import { FakeContext, provideAnalysisTesting } from '../analysis/test-setup';
import { RecordView } from '../core/api.types';
import { MockAiBackend } from '../core/mock/mock-ai-backend';
import { WorkspaceStore } from '../core/workspace.store';
import { DialogService, DIALOG_REF } from '../ui/dialog.service';
import { all, buttonByText, el, FakeDock, provideFakeDock, settle, typeInto } from '../../testing/helpers';
import { ResearchDialog } from './research-dialog';
import { BrowserBackend, ChatBrowserBackend } from './browser-backend';
import { AgentDecision, chatBrowserMode } from './browser.types';
import { engineLabel, handOffText, leakText } from './browser-text';
import { AgentApprovalCard, AgentPaused, LinkCard } from './chat-cards';
import { ChatBrowserBar } from './chat-browser-bar';
import { ChatBrowserStore } from './chat-browser.store';
import { MockBrowserBackend, MockWeb, resetMockWeb } from './mock-browser-backend';
import { browserCommands, browserRecordMenu, BrowserLauncher, worthSearching, WEB_INSPECTOR_SECTION } from './provide-browser';
import { SearchWebDialog } from './search-web-dialog';

let web: MockWeb;
let dock: FakeDock;

function configure(extra: unknown[] = []) {
  web = resetMockWeb();
  dock = new FakeDock();
  const backend = new MockBrowserBackend(web);
  TestBed.configureTestingModule({
    providers: [
      ...provideAnalysisTesting({ context: new FakeContext() }),
      ...provideFakeDock(dock),
      { provide: AiBackend, useValue: new MockAiBackend({ stepMs: 0 }) },
      { provide: BrowserBackend, useValue: backend },
      { provide: ChatBrowserBackend, useValue: backend },
      ...(extra as never[]),
    ],
  });
  return backend;
}

async function until(f: { detectChanges(): void }, cond: () => boolean) {
  for (let i = 0; i < 200 && !cond(); i++) {
    await settle();
    f.detectChanges();
  }
  expect(cond(), String(cond)).toBe(true);
}

describe('ResearchDialog', () => {
  afterEach(() => TestBed.resetTestingModule());

  function open(inputs: Record<string, unknown>) {
    const close = vi.fn();
    configure([{ provide: DIALOG_REF, useValue: { close } }]);
    const ai = TestBed.inject(AiBackend);
    const f = TestBed.createComponent(ResearchDialog);
    for (const [k, v] of Object.entries(inputs)) f.componentRef.setInput(k, v);
    f.detectChanges();
    return { f, close, ai, root: f.nativeElement as HTMLElement };
  }

  it('shows the drafted search text and who wrote it, and the user edits it before it runs', async () => {
    const { f, close, ai } = open({ question: 'why does it fail?', query: 'System.UnauthorizedAccessException 0x80070005', source: 'model', recordId: 7 });
    expect(el(f, '[data-testid=research-source]').textContent).toContain('The AI wrote it from the question and the event');
    const research = vi.spyOn(ai, 'research');
    typeInto(el<HTMLInputElement>(f, '[data-testid=research-query]'), 'UnauthorizedAccessException IIS');
    f.detectChanges();
    expect(el(f, '[data-testid=research-source]').textContent).toContain('Only this text goes to the search engine');
    el<HTMLButtonElement>(f, '[data-testid=research-run]').click();
    await until(f, () => close.mock.calls.length > 0);
    expect(research.mock.calls[0]?.[0]).toMatchObject({ question: 'why does it fail?', query: 'UnauthorizedAccessException IIS', recordId: null, confirmed: false });
  });

  it('lets the AI suggest the text first and never runs without one; data from the logs needs "Research anyway"', async () => {
    const { f, close, ai, root } = open({ question: 'why is WEB01 slow?' });
    const research = vi.spyOn(ai, 'research');
    const start = el<HTMLButtonElement>(f, '[data-testid=research-run]');
    expect(start.textContent).toContain('Start research');
    expect(start.disabled).toBe(true);
    expect(start.title).toBe('Write a search text, or let the AI suggest one');
    expect(el(f, '[data-testid=research-draft]').textContent).toContain('Suggest search text');
    el<HTMLButtonElement>(f, '[data-testid=research-draft]').click();
    await until(f, () => !!el<HTMLInputElement>(f, '[data-testid=research-query]').value);
    expect(research).not.toHaveBeenCalled();
    expect(el<HTMLInputElement>(f, '[data-testid=research-query]').value).toBe('WEB01 slow');
    expect(root.querySelector('[data-testid=research-flagged] mark')?.textContent).toContain('WEB01');
    expect(el(f, '[data-testid=research-run]').textContent).toContain('Research anyway');
    el<HTMLButtonElement>(f, '[data-testid=research-run]').click();
    await until(f, () => close.mock.calls.length > 0);
    expect(research.mock.calls[0]?.[0]).toMatchObject({ query: 'WEB01 slow', confirmed: true });
  });

  it('needs a question before anything can be suggested or started', () => {
    const { f } = open({});
    const suggest = el<HTMLButtonElement>(f, '[data-testid=research-draft]');
    expect(suggest.disabled).toBe(true);
    expect(suggest.title).toBe('Say what the AI should find out first');
    expect(el<HTMLButtonElement>(f, '[data-testid=research-run]').title).toBe('Say what the AI should find out first');
    typeInto(el<HTMLTextAreaElement>(f, '[data-testid=research-question]'), 'why?');
    typeInto(el<HTMLInputElement>(f, '[data-testid=research-query]'), 'why');
    f.detectChanges();
    expect(suggest.disabled).toBe(false);
    expect(el<HTMLButtonElement>(f, '[data-testid=research-run]').title).toContain('the answer arrives in the chat');
  });

  it('shows a failed draft and the engine fallback', async () => {
    const { f, ai, root } = open({ question: 'q' });
    vi.spyOn(ai, 'researchDraft').mockRejectedValueOnce(new Error('the model is busy'));
    el<HTMLButtonElement>(f, '[data-testid=research-draft]').click();
    await until(f, () => !!root.querySelector('.err'));
    expect(el(f, '.err').textContent).toContain('the model is busy');
    vi.spyOn(ai, 'researchDraft').mockResolvedValueOnce({ question: 'q', checked: { query: 'engine text', needsConfirm: false, segments: [] }, source: 'engine' });
    el<HTMLButtonElement>(f, '[data-testid=research-draft]').click();
    await until(f, () => el<HTMLInputElement>(f, '[data-testid=research-query]').value === 'engine text');
    expect(el(f, '[data-testid=research-source]').textContent).toContain('Built by Debrief');
    expect(root.querySelector('.err')).toBeNull();
  });
});

describe('SearchWebDialog', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('shows the engine-built search, lets the user edit it and opens the browser', async () => {
    const close = vi.fn();
    configure([{ provide: DIALOG_REF, useValue: { close } }]);
    const f = TestBed.createComponent(SearchWebDialog);
    f.componentRef.setInput('recordId', 7);
    f.detectChanges();
    await until(f, () => !!(f.nativeElement as HTMLElement).querySelector('[data-testid=web-search-text]'));
    const input = el<HTMLInputElement>(f, '[data-testid=web-search-text]');
    expect(input.value).toBe('System.TimeoutException The operation has timed out');
    expect(el(f, '[data-testid=web-search-removed]').textContent).toContain('1 value from your logs');
    expect(el(f, '[data-testid=web-search-source]').textContent).toContain('The AI wrote this search');
    typeInto(input, 'TimeoutException pool');
    typeInto(el<HTMLSelectElement>(f, '#web-search-engine'), 'bing', 'change');
    f.detectChanges();
    buttonByText(f, 'Search').click();
    await until(f, () => close.mock.calls.length > 0);
    expect(close).toHaveBeenCalledWith(true);
    expect(web.external).toEqual(['https://www.bing.com/search?q=TimeoutException+pool']);
  });

  it('reports a failure with Retry and a refused search', async () => {
    const close = vi.fn();
    const backend = configure([{ provide: DIALOG_REF, useValue: { close } }]);
    vi.spyOn(backend, 'eventQuery').mockRejectedValueOnce(new Error('nothing to search for'));
    const f = TestBed.createComponent(SearchWebDialog);
    f.componentRef.setInput('recordId', 3);
    f.componentRef.setInput('text', 'start');
    f.detectChanges();
    await until(f, () => !!(f.nativeElement as HTMLElement).querySelector('.err'));
    expect(el(f, '.err').textContent).toContain('nothing to search for');
    buttonByText(f, 'Retry').click();
    await until(f, () => el<HTMLInputElement>(f, '[data-testid=web-search-text]').value.includes('TimeoutException'));
    vi.spyOn(backend, 'open').mockRejectedValueOnce(new Error('the search is empty'));
    buttonByText(f, 'Search').click();
    await until(f, () => !!(f.nativeElement as HTMLElement).querySelector('.err'));
    expect(close).not.toHaveBeenCalled();
    buttonByText(f, 'Cancel').click();
    expect(close).toHaveBeenCalledWith();
  });
});

@Component({
  imports: [AgentApprovalCard, LinkCard, AgentPaused],
  template: `
    <debrief-agent-approval [approval]="a()" (decide)="got.push($event)" />
    <debrief-link-card [link]="l()" (open)="opened.push($event)" />
    <debrief-agent-paused [paused]="p()" (continue)="continued = continued + 1" />
  `,
})
class CardsHost {
  readonly a = signal<AgentApprovalView>({
    request: {
      requestId: 1,
      ask: { kind: 'leavingData', site: 'evil.example', url: 'https://evil.example/?h=WEB01', element: null, text: null, submit: false, canAllowSite: false },
      urlSegments: [
        { text: 'evil.example/?h=', leak: null },
        { text: 'WEB01', leak: 'logValue' },
      ],
      textSegments: [],
    },
    state: 'pending',
  });
  readonly l = signal<LinkView>({ kind: 'search', url: 'https://duckduckgo.com/?q=x', title: 'Search x', query: 'x', segments: [] });
  readonly p = signal<PauseView>({ reason: 'stepCap', steps: 25, handOff: null });
  readonly got: AgentDecision[] = [];
  readonly opened: string[] = [];
  continued = 0;
}

describe('chat cards', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('show what the agent would do, with workspace data highlighted, and take the answer', () => {
    const f = TestBed.createComponent(CardsHost);
    f.detectChanges();
    const card = el(f, '[data-testid=agent-approval]');
    expect(card.textContent).toContain('send data from your logs to evil.example');
    expect(el(f, 'mark.logValue').textContent).toContain('WEB01');
    expect(card.textContent).toContain('Highlighted parts come from your logs');
    expect(all<HTMLButtonElement>(f, '[data-testid=agent-approval] button').map((b) => b.textContent?.trim())).toEqual(['Deny', 'Allow']);
    buttonByText(f, 'Deny').click();
    buttonByText(f, 'Allow').click();
    expect(f.componentInstance.got).toEqual(['deny', 'allow']);
    f.componentInstance.a.update((a) => ({
      ...a,
      request: { ...a.request, ask: { ...a.request.ask, kind: 'typeText', text: 'hello', submit: true, canAllowSite: true }, textSegments: [{ text: 'hello', leak: null }] },
    }));
    f.detectChanges();
    expect(card.textContent).toContain('…and then press Enter.');
    buttonByText(f, 'Allow this site for this chat').click();
    expect(f.componentInstance.got.at(-1)).toBe('allowSite');
    for (const [kind, words] of [['newSite', 'wants to open'], ['submitForm', 'submit a form'], ['pressButton', 'press a button']] as const) {
      f.componentInstance.a.update((a) => ({ ...a, request: { ...a.request, ask: { ...a.request.ask, kind } } }));
      f.detectChanges();
      expect(card.textContent).toContain(words);
    }
    for (const [state, words] of [['allow', 'Allowed'], ['allowSite', 'this site is allowed'], ['deny', 'Denied'], ['expired', 'Not done']] as const) {
      f.componentInstance.a.update((a) => ({ ...a, state }));
      f.detectChanges();
      expect(el(f, '.result').textContent).toContain(words);
    }
    // Link card and the step cap.
    expect(el(f, '[data-testid=link-card]').textContent).toContain('https://duckduckgo.com/?q=x');
    buttonByText(f, /^Open$/).click();
    expect(f.componentInstance.opened).toEqual(['https://duckduckgo.com/?q=x']);
    f.componentInstance.l.update((l) => ({ ...l, kind: 'link' }));
    f.detectChanges();
    expect(el(f, '[data-testid=link-card]').textContent).toContain('suggested page');
    expect(el(f, '[data-testid=agent-paused]').textContent).toContain('Stopped after 25 steps. Continue?');
    el<HTMLButtonElement>(f, '[data-testid=agent-continue]').click();
    expect(f.componentInstance.continued).toBe(1);
    f.componentInstance.p.set({ reason: 'handBack', steps: 3, handOff: 'captcha' });
    f.detectChanges();
    expect(el(f, '[data-testid=agent-paused]').textContent).toContain('CAPTCHA');
    expect(f.nativeElement.querySelector('[data-testid=agent-continue]')).toBeNull();
    f.componentInstance.p.set({ reason: 'stopped', steps: 3, handOff: null });
    f.detectChanges();
    expect(el(f, '[data-testid=agent-paused]').textContent).toContain('You stopped the AI');
  });

  it('name engines, leaks and hand-offs', () => {
    expect(['duckDuckGo', 'google', 'bing', 'custom'].map((e) => engineLabel(e as never))).toEqual(['DuckDuckGo', 'Google', 'Bing', 'Custom']);
    expect([leakText('logValue'), leakText('logText'), leakText('private')]).toEqual(['from your logs', 'text from your logs', 'looks private']);
    expect(handOffText('login')).toContain('sign in');
    expect(handOffText('payment')).toContain('payment');
    expect(handOffText(null)).toContain('handed the browser back');
  });
});

describe('ChatBrowserBar', () => {
  beforeEach(() => configure());
  afterEach(() => TestBed.resetTestingModule());

  it('puts a chat in one browser mode at a time: off, read-along or agent', async () => {
    const store = TestBed.inject(ChatBrowserStore);
    await store.load('chat-1');
    expect(chatBrowserMode(store.statusOf('chat-1'))).toBe('off');
    expect(chatBrowserMode(null)).toBe('off');
    await store.setMode('chat-1', 'readAlong');
    expect(web.status('chat-1')).toMatchObject({ readAlong: true, agent: false });
    await store.setMode('chat-1', 'agent');
    expect(chatBrowserMode(store.statusOf('chat-1'))).toBe('agent');
    // The same mode again changes nothing.
    const setAgent = vi.spyOn(TestBed.inject(ChatBrowserBackend), 'setAgent');
    await store.setMode('chat-1', 'agent');
    expect(setAgent).not.toHaveBeenCalled();
    // Read-along again: agent mode ends, read-along stays.
    await store.setMode('chat-1', 'readAlong');
    expect(web.status('chat-1')).toMatchObject({ readAlong: true, agent: false });
    await store.setMode('chat-1', 'agent');
    await store.setMode('chat-1', 'off');
    expect(web.status('chat-1')).toMatchObject({ readAlong: false, agent: false });
    // A failed step stops there, with the error shown.
    await store.setMode('chat-1', 'agent');
    setAgent.mockRejectedValueOnce(new Error('engine busy'));
    expect(await store.setMode('chat-1', 'off')).toBeNull();
    expect(store.error()).toBe('engine busy');
    expect(web.status('chat-1').agent).toBe(true);
  });

  it('shows what agent mode may do, the small-model hint and the pages', async () => {
    web.smallModel = true;
    const f = TestBed.createComponent(ChatBrowserBar);
    f.componentRef.setInput('sessionId', 'chat-1');
    f.detectChanges();
    await until(f, () => !!TestBed.inject(ChatBrowserStore).statusOf('chat-1'));
    const root = f.nativeElement as HTMLElement;
    expect(root.textContent).not.toContain('New sites, typing and forms need your approval');
    await TestBed.inject(ChatBrowserStore).setMode('chat-1', 'agent');
    f.detectChanges();
    expect(root.textContent).toContain('A real browser window will open next to the app');
    expect(root.textContent).toContain('New sites, typing and forms need your approval');
    expect(el(f, '[data-testid=chat-small-model]').textContent).toContain('7B or larger');
    // Pages for the next question.
    web.show('https://e.org/a');
    await TestBed.inject(ChatBrowserStore).capture('chat-1');
    f.detectChanges();
    expect(el(f, '[data-testid=chat-source]').textContent).toContain('untrusted web content');
    buttonByText(f, 'Remove pages').click();
    await until(f, () => !root.querySelector('[data-testid=chat-source]'));
    // A failure is shown with Retry, which reads the chat's status again.
    vi.spyOn(TestBed.inject(ChatBrowserBackend), 'setAgent').mockRejectedValueOnce(new Error('engine busy'));
    await TestBed.inject(ChatBrowserStore).setMode('chat-1', 'off');
    f.detectChanges();
    expect(el(f, '.err').textContent).toContain('engine busy');
    const load = vi.spyOn(TestBed.inject(ChatBrowserStore), 'load');
    buttonByText(f, 'Retry').click();
    await until(f, () => !root.querySelector('.err'));
    expect(load).toHaveBeenCalledWith('chat-1');
  });

  it('adds the open page to the question while a browser mode is on, and shows the engine message when no page is open', async () => {
    const f = TestBed.createComponent(ChatBrowserBar);
    f.componentRef.setInput('sessionId', 'chat-1');
    f.detectChanges();
    const root = f.nativeElement as HTMLElement;
    const store = TestBed.inject(ChatBrowserStore);
    await until(f, () => !!store.statusOf('chat-1'));
    expect(root.querySelector('[data-testid=chat-capture]'), 'off: no button').toBeNull();
    await store.setMode('chat-1', 'readAlong');
    f.detectChanges();
    expect(el(f, '[data-testid=chat-capture]').textContent).toContain('Add the open page to the question');
    el<HTMLButtonElement>(f, '[data-testid=chat-capture]').click();
    await until(f, () => !!root.querySelector('.err'));
    expect(el(f, '.err').textContent).toContain('No page is open');
    web.show('https://e.org/a');
    el<HTMLButtonElement>(f, '[data-testid=chat-capture]').click();
    await until(f, () => !!root.querySelector('[data-testid=chat-source]'));
    expect(el(f, '[data-testid=chat-source]').textContent).toContain('untrusted web content');
    expect(root.querySelector('.err')).toBeNull();
  });

  it('offers a web search for the selected event', async () => {
    const f = TestBed.createComponent(ChatBrowserBar);
    f.componentRef.setInput('sessionId', 'chat-1');
    f.detectChanges();
    expect((f.nativeElement as HTMLElement).querySelector('[data-testid=chat-search-web]')).toBeNull();
    const search = vi.spyOn(TestBed.inject(BrowserLauncher), 'searchEvent').mockResolvedValue();
    TestBed.inject(WorkspaceStore).select({ id: 12 } as RecordView);
    f.detectChanges();
    el<HTMLButtonElement>(f, '[data-testid=chat-search-web]').click();
    expect(search).toHaveBeenCalledWith(12);
  });
});

describe('provideBrowser contributions', () => {
  beforeEach(() => configure());
  afterEach(() => TestBed.resetTestingModule());

  it('offers commands, the record menu and the inspector section (no panel of its own)', () => {
    const launcher = TestBed.inject(BrowserLauncher);
    const store = TestBed.inject(WorkspaceStore);
    const search = vi.spyOn(launcher, 'searchEvent').mockResolvedValue();
    const cmds = browserCommands(launcher, store);
    expect(cmds.map((c) => c.id)).toEqual(['browser.searchSelected']);
    const ctx = {} as never;
    expect(cmds[0]!.when?.(ctx)).toBe(false);
    store.select({ id: 5 } as RecordView);
    expect(cmds[0]!.when?.(ctx)).toBe(true);
    void cmds[0]!.run(ctx);
    expect(search).toHaveBeenCalledWith(5);
    const [entry] = browserRecordMenu(launcher)({ id: 9 } as RecordView, {});
    expect(entry?.label).toBe('Search the web for this error…');
    entry?.run();
    expect(search).toHaveBeenCalledWith(9);
    const r = (level: string, exception: string | null = null) => ({ level, exception }) as RecordView;
    expect([r('Error'), r('Warning'), r('Fatal'), r('Information', 'x'), r('Information'), r('Debug')].map(worthSearching)).toEqual([true, true, true, true, false, false]);
    expect(WEB_INSPECTOR_SECTION.when?.(r('Error'))).toBe(true);
  });

  it('opens the search dialog, which opens the search in the default browser', async () => {
    const dialogs = TestBed.inject(DialogService);
    const open = vi.spyOn(dialogs, 'open').mockReturnValue({ closed: Promise.resolve(true), close: () => undefined } as never);
    await TestBed.inject(BrowserLauncher).searchEvent(4);
    expect(open.mock.calls[0]?.[0]).toBe(SearchWebDialog);
    expect(open.mock.calls[0]?.[1]).toMatchObject({ inputs: { recordId: 4 } });
    expect(dock.calls).toEqual([]);
  });
});
