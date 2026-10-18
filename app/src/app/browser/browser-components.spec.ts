import { Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AiBackend } from '../ai/ai-backend';
import { AgentApprovalView, LinkView, PauseView } from '../ai/chat/chat-model';
import { ChatStore } from '../ai/chat/chat.store';
import { FakeContext, provideAnalysisTesting } from '../analysis/test-setup';
import { RecordView } from '../core/api.types';
import { MockAiBackend } from '../core/mock/mock-ai-backend';
import { WorkspaceStore } from '../core/workspace.store';
import { DialogService, DIALOG_REF } from '../ui/dialog.service';
import { all, buttonByText, el, FakeDock, provideFakeDock, settle, typeInto } from '../../testing/helpers';
import { BrowserBackend, ChatBrowserBackend } from './browser-backend';
import { BrowserStore } from './browser.store';
import { AgentDecision } from './browser.types';
import { engineLabel, handOffText, leakText, noticeText } from './browser-text';
import { AgentApprovalCard, AgentPaused, LinkCard } from './chat-cards';
import { ChatBrowserBar } from './chat-browser-bar';
import { ChatBrowserStore } from './chat-browser.store';
import { MockBrowserBackend, MockWeb, resetMockWeb } from './mock-browser-backend';
import { browserCommands, browserRecordMenu, BrowserLauncher, browserViews, worthSearching, WEB_INSPECTOR_SECTION } from './provide-browser';
import { SearchWebDialog } from './search-web-dialog';
import { WebPanel } from './web-panel';

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

describe('WebPanel', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('drives the browser, hands the page to the chat and opens it externally after a confirm', { timeout: 30000 }, async () => {
    configure();
    const f = TestBed.createComponent(WebPanel);
    f.detectChanges();
    const root = f.nativeElement as HTMLElement;
    expect(root.textContent).toContain('separate browser window');
    const address = el<HTMLInputElement>(f, '[data-testid=web-address]');
    typeInto(address, 'TimeoutException gateway');
    f.detectChanges();
    el<HTMLButtonElement>(f, 'button[type=submit]').click();
    await until(f, () => !!root.querySelector('[data-testid=web-page]'));
    expect(root.textContent).toContain('duckduckgo.com');
    // The engine picker changes the engine of the next search.
    const engine = el<HTMLSelectElement>(f, '[data-testid=web-engine]');
    expect(all<HTMLOptionElement>(f, '[data-testid=web-engine] option').map((o) => o.textContent?.trim())).toEqual(['DuckDuckGo', 'Google', 'Bing', 'Stack Overflow']);
    typeInto(engine, 'google', 'change');
    typeInto(engine, 'nonsense', 'change');
    expect(TestBed.inject(BrowserStore).engine()).toBe('google');
    // Add page to AI context: a chat is created when there is none, and shown.
    el<HTMLButtonElement>(f, '[data-testid=web-capture]').click();
    await until(f, () => root.textContent!.includes('untrusted web content'));
    const chat = TestBed.inject(ChatStore).activeId()!;
    expect(TestBed.inject(ChatBrowserStore).sourcesOf(chat)).toHaveLength(1);
    expect(dock.calls).toContain('show:ai-chat');
    // Open in your browser asks first.
    buttonByText(f, 'Open in your browser').click();
    f.detectChanges();
    expect(el(f, '[data-testid=web-confirm]').textContent).toContain('duckduckgo.com');
    buttonByText(f, 'Cancel').click();
    f.detectChanges();
    expect(root.querySelector('[data-testid=web-confirm]')).toBeNull();
    buttonByText(f, 'Open in your browser').click();
    f.detectChanges();
    buttonByText(f, /^Open$/).click();
    await settle();
    expect(web.external).toHaveLength(1);
    // Back, forward, reload and close.
    el<HTMLButtonElement>(f, '[aria-label=Back]').click();
    el<HTMLButtonElement>(f, '[aria-label=Forward]').click();
    el<HTMLButtonElement>(f, '[aria-label=Reload]').click();
    web.page = { ...web.page, loading: true };
    web.emit({ type: 'state', ...web.page });
    f.detectChanges();
    el<HTMLButtonElement>(f, '[aria-label="Stop loading"]').click();
    buttonByText(f, 'Close browser').click();
    await until(f, () => !root.querySelector('[data-testid=web-page]'));
  });

  it('shows notices, the agent driving with Stop, and failures with Retry', async () => {
    const backend = configure();
    const f = TestBed.createComponent(WebPanel);
    f.detectChanges();
    const root = f.nativeElement as HTMLElement;
    web.show('https://e.org/a');
    web.emit({ type: 'notice', kind: 'download', url: 'https://e.org/file.zip', message: '' });
    web.emit({ type: 'notice', kind: 'popup', url: 'https://e.org/a', message: '' });
    web.emit({ type: 'notice', kind: 'blocked', url: 'file:///x', message: 'only http and https pages open' });
    web.emit({ type: 'notice', kind: 'agentBlocked', url: 'https://evil.example/', message: 'evil.example' });
    f.detectChanges();
    expect(all(f, '.notice')).toHaveLength(4);
    expect(root.textContent).toContain('Downloads are blocked');
    expect(root.textContent).toContain('evil.example, which this chat may not visit');
    all<HTMLButtonElement>(f, '.notice button').find((b) => b.textContent?.includes('Open in your browser'))!.click();
    f.detectChanges();
    buttonByText(f, /^Open$/).click();
    await settle();
    f.detectChanges();
    buttonByText(f, 'Dismiss').click();
    f.detectChanges();
    expect(all(f, '.notice').length).toBeLessThan(4);
    web.update('chat-9', { agent: true, stepsUsed: 3 });
    f.detectChanges();
    expect(el(f, '[data-testid=web-agent]').textContent).toContain('step 3 of 25');
    el<HTMLButtonElement>(f, '[data-testid=web-agent-stop]').click();
    await until(f, () => !root.querySelector('[data-testid=web-agent]'));
    vi.spyOn(backend, 'nav').mockRejectedValueOnce(new Error('the browser is not open'));
    el<HTMLButtonElement>(f, '[aria-label=Reload]').click();
    await until(f, () => !!root.querySelector('[data-testid=web-error]'));
    buttonByText(f, 'Retry').click();
    await until(f, () => !root.querySelector('[data-testid=web-error]'));
    vi.spyOn(backend, 'nav').mockRejectedValueOnce(new Error('x'));
    el<HTMLButtonElement>(f, '[aria-label=Reload]').click();
    await until(f, () => !!root.querySelector('[data-testid=web-error]'));
    all<HTMLButtonElement>(f, '[data-testid=web-error] button').at(-1)!.click();
    f.detectChanges();
    expect(root.querySelector('[data-testid=web-error]')).toBeNull();
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
    expect(el(f, '[data-testid=web-search-removed]').textContent).toContain('1 names from your logs');
    typeInto(input, 'TimeoutException pool');
    typeInto(el<HTMLSelectElement>(f, '#web-search-engine'), 'stackOverflow', 'change');
    f.detectChanges();
    buttonByText(f, 'Search').click();
    await until(f, () => close.mock.calls.length > 0);
    expect(close).toHaveBeenCalledWith(true);
    expect(web.page.url).toBe('https://stackoverflow.com/search?q=TimeoutException+pool');
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
    <imt-agent-approval [approval]="a()" (decide)="got.push($event)" />
    <imt-link-card [link]="l()" (open)="opened.push($event)" />
    <imt-agent-paused [paused]="p()" (continue)="continued = continued + 1" />
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

  it('name engines, notices, leaks and hand-offs', () => {
    expect(['duckDuckGo', 'google', 'bing', 'stackOverflow', 'custom'].map((e) => engineLabel(e as never))).toEqual(['DuckDuckGo', 'Google', 'Bing', 'Stack Overflow', 'Custom']);
    expect(noticeText('popup', '')).toContain('new window');
    expect(noticeText('blocked', 'why')).toContain('why');
    expect([leakText('logValue'), leakText('logText'), leakText('private')]).toEqual(['from your logs', 'text from your logs', 'looks private']);
    expect(handOffText('login')).toContain('sign in');
    expect(handOffText('payment')).toContain('payment');
    expect(handOffText(null)).toContain('handed the browser back');
  });
});

describe('ChatBrowserBar', () => {
  beforeEach(() => configure());
  afterEach(() => TestBed.resetTestingModule());

  it('switches read-along and agent mode, shows the small-model hint, Stop and the pages', async () => {
    web.smallModel = true;
    const f = TestBed.createComponent(ChatBrowserBar);
    f.componentRef.setInput('sessionId', 'chat-1');
    f.detectChanges();
    await until(f, () => !!TestBed.inject(ChatBrowserStore).statusOf('chat-1'));
    const root = f.nativeElement as HTMLElement;
    el<HTMLInputElement>(f, '[data-testid=chat-read-along]').click();
    el<HTMLInputElement>(f, '[data-testid=chat-agent]').click();
    await until(f, () => !!root.querySelector('[data-testid=chat-agent-stop]'));
    expect(root.textContent).toContain('New sites, typing and forms need your approval');
    expect(el(f, '[data-testid=chat-small-model]').textContent).toContain('7B or larger');
    el<HTMLButtonElement>(f, '[data-testid=chat-agent-stop]').click();
    await until(f, () => !root.querySelector('[data-testid=chat-agent-stop]'));
    expect(web.status('chat-1')).toMatchObject({ readAlong: true, agent: false });
    // Pages for the next question.
    web.show('https://e.org/a');
    await TestBed.inject(ChatBrowserStore).capture('chat-1');
    f.detectChanges();
    expect(el(f, '[data-testid=chat-source]').textContent).toContain('untrusted web content');
    buttonByText(f, 'Remove pages').click();
    await until(f, () => !root.querySelector('[data-testid=chat-source]'));
    // A failure is shown with Retry.
    vi.spyOn(TestBed.inject(ChatBrowserBackend), 'setAgent').mockRejectedValueOnce(new Error('engine busy'));
    el<HTMLInputElement>(f, '[data-testid=chat-agent]').click();
    await until(f, () => !!root.querySelector('.err'));
    buttonByText(f, 'Retry').click();
    await until(f, () => !!root.querySelector('[data-testid=chat-agent-stop]'));
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

  it('offers the Web view in Tools, commands, the record menu and the inspector section', async () => {
    const [view] = browserViews();
    expect(view).toMatchObject({ id: 'web', category: 'Tools', icon: 'globe' });
    expect(await view!.component()).toBe(WebPanel);
    const launcher = TestBed.inject(BrowserLauncher);
    const store = TestBed.inject(WorkspaceStore);
    const search = vi.spyOn(launcher, 'searchEvent').mockResolvedValue();
    const cmds = browserCommands(launcher, store);
    expect(cmds.map((c) => c.id)).toEqual(['browser.show', 'browser.searchSelected']);
    const ctx = {} as never;
    expect(cmds[1]!.when?.(ctx)).toBe(false);
    void cmds[0]!.run(ctx);
    expect(dock.calls).toContain('show:web');
    store.select({ id: 5 } as RecordView);
    expect(cmds[1]!.when?.(ctx)).toBe(true);
    void cmds[1]!.run(ctx);
    expect(search).toHaveBeenCalledWith(5);
    const [entry] = browserRecordMenu(launcher)({ id: 9 } as RecordView, {});
    expect(entry?.label).toBe('Search the web for this error…');
    entry?.run();
    expect(search).toHaveBeenCalledWith(9);
    const r = (level: string, exception: string | null = null) => ({ level, exception }) as RecordView;
    expect([r('Error'), r('Warning'), r('Fatal'), r('Information', 'x'), r('Information'), r('Debug')].map(worthSearching)).toEqual([true, true, true, true, false, false]);
    expect(WEB_INSPECTOR_SECTION.when?.(r('Error'))).toBe(true);
  });

  it('opens the search dialog and the panel after a search', async () => {
    const dialogs = TestBed.inject(DialogService);
    const closed = Promise.resolve(true);
    const open = vi.spyOn(dialogs, 'open').mockReturnValue({ closed, close: () => undefined } as never);
    await TestBed.inject(BrowserLauncher).searchEvent(4);
    expect(open.mock.calls[0]?.[0]).toBe(SearchWebDialog);
    expect(open.mock.calls[0]?.[1]).toMatchObject({ inputs: { recordId: 4 } });
    expect(dock.calls).toContain('show:web');
    open.mockReturnValue({ closed: Promise.resolve(undefined), close: () => undefined } as never);
    dock.calls.length = 0;
    await TestBed.inject(BrowserLauncher).searchEvent(4);
    expect(dock.calls).toEqual([]);
  });
});
