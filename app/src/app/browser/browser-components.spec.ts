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
import { all, buttonByText, el, FakeDock, manualFrames, provideFakeDock, settle, typeInto } from '../../testing/helpers';
import { POPOUT_ROUTE } from '../popout/popout-route';
import { ResearchDialog } from './research-dialog';
import { BrowserBackend, ChatBrowserBackend } from './browser-backend';
import { BrowserStore } from './browser.store';
import { AgentDecision } from './browser.types';
import { engineLabel, handOffText, leakText, noticeText } from './browser-text';
import { AgentApprovalCard, AgentPaused, LinkCard } from './chat-cards';
import { ChatBrowserBar } from './chat-browser-bar';
import { ChatBrowserStore } from './chat-browser.store';
import { MockBrowserBackend, MockWeb, resetMockWeb } from './mock-browser-backend';
import { browserCommands, browserRecordMenu, BrowserLauncher, BrowserReveal, browserViews, worthSearching, WEB_INSPECTOR_SECTION } from './provide-browser';
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
    expect(root.textContent).toContain('it shows here');
    const address = el<HTMLInputElement>(f, '[data-testid=web-address]');
    typeInto(address, 'TimeoutException gateway');
    f.detectChanges();
    el<HTMLButtonElement>(f, 'button[type=submit]').click();
    await until(f, () => !!root.querySelector('[data-testid=web-page]'));
    expect(root.textContent).toContain('google.com/search');
    // The engine picker changes the engine of the next search.
    const engine = el<HTMLSelectElement>(f, '[data-testid=web-engine]');
    expect(all<HTMLOptionElement>(f, '[data-testid=web-engine] option').map((o) => o.textContent?.trim())).toEqual(['Google', 'DuckDuckGo', 'Bing']);
    expect(engine.value).toBe('google');
    typeInto(engine, 'duckDuckGo', 'change');
    typeInto(engine, 'nonsense', 'change');
    expect(TestBed.inject(BrowserStore).engine()).toBe('duckDuckGo');
    // Add page to AI context: a chat is created when there is none, and shown.
    el<HTMLButtonElement>(f, '[data-testid=web-capture]').click();
    await until(f, () => root.textContent!.includes('untrusted web content'));
    const chat = TestBed.inject(ChatStore).activeId()!;
    expect(TestBed.inject(ChatBrowserStore).sourcesOf(chat)).toHaveLength(1);
    expect(dock.calls).toContain('show:ai-chat');
    // Open in your browser asks first.
    buttonByText(f, 'Open in your browser').click();
    f.detectChanges();
    expect(el(f, '[data-testid=web-confirm]').textContent).toContain('google.com');
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
    buttonByText(f, 'Close page').click();
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
    // The AI drives: the panel header says so with the step and Stop, and so does the tab.
    web.update('chat-9', { agent: true, stepsUsed: 3 });
    web.drive({ kind: 'agent', sessionId: 'chat-9' });
    await until(f, () => !!root.querySelector('[data-testid=ai-browsing]'));
    expect(el(f, '[data-testid=ai-browsing]').textContent).toContain('step 3 of 25');
    expect(dock.calls).toContain('title:web:Browser · AI is browsing');
    el<HTMLButtonElement>(f, '[data-testid=ai-browsing-stop]').click();
    await until(f, () => !root.querySelector('[data-testid=ai-browsing]'));
    expect(dock.calls.at(-1)).toBe('title:web:Browser');
    // Research shows its job's progress.
    web.drive({ kind: 'research' });
    await until(f, () => !!root.querySelector('[data-testid=ai-browsing]'));
    expect(el(f, '[data-testid=ai-browsing]').textContent).toContain('Research is reading');
    web.drive(null);
    await until(f, () => !root.querySelector('[data-testid=ai-browsing]'));
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

  it('places the page over its area and hides it while an overlay covers it', async () => {
    const backend = configure();
    const place = vi.spyOn(backend, 'place');
    const frames = manualFrames();
    try {
      const f = TestBed.createComponent(WebPanel);
      f.detectChanges();
      const root = f.nativeElement as HTMLElement;
      const area = el<HTMLElement>(f, '[data-testid=web-page-area]');
      area.getBoundingClientRect = () => ({ left: 10, top: 80, right: 410, bottom: 380, width: 400, height: 300, x: 10, y: 80, toJSON: () => null }) as DOMRect;
      const step = async () => {
        for (let i = 0; i < 3; i++) {
          await settle();
          f.detectChanges();
          frames.flush();
        }
      };
      await step();
      expect(place.mock.calls.at(-1)?.[0]).toMatchObject({ visible: false, rect: { x: 10, y: 80, width: 400, height: 300 } });
      web.show('https://e.org/a');
      await step();
      expect(place.mock.calls.at(-1)?.[0].visible, 'a page is open').toBe(true);
      expect(area.dataset['shown']).toBe('true');
      // A dialog over the page: hidden, and the panel says why.
      const dialog = document.createElement('div');
      dialog.setAttribute('data-imt-overlay', '');
      dialog.getBoundingClientRect = () => ({ left: 0, top: 0, right: 2000, bottom: 2000, width: 2000, height: 2000, x: 0, y: 0, toJSON: () => null }) as DOMRect;
      document.body.appendChild(dialog);
      await step();
      expect(place.mock.calls.at(-1)?.[0].visible).toBe(false);
      expect(root.querySelector('[data-testid=web-page-hidden]')?.textContent).toContain('hidden while a menu');
      dialog.remove();
      await step();
      expect(place.mock.calls.at(-1)?.[0].visible).toBe(true);
      expect(root.querySelector('[data-testid=web-page-hidden]')).toBeNull();
      // The panel goes: the page hides.
      f.destroy();
      expect(place.mock.calls.at(-1)?.[0]).toMatchObject({ visible: false, rect: null });
    } finally {
      frames.restore();
      document.body.querySelectorAll('[data-imt-overlay]').forEach((e) => e.remove());
    }
  });

  it('focuses the address bar on Ctrl+L, takes the keyboard back on Esc and replays IMT shortcuts typed in the page', async () => {
    configure();
    const f = TestBed.createComponent(WebPanel);
    document.body.appendChild(f.nativeElement);
    f.detectChanges();
    await settle(f);
    const address = el<HTMLInputElement>(f, '[data-testid=web-address]');
    const area = el<HTMLElement>(f, '[data-testid=web-page-area]');
    // Ctrl+L in the panel itself.
    area.focus();
    area.dispatchEvent(new KeyboardEvent('keydown', { key: 'l', ctrlKey: true, bubbles: true }));
    expect(document.activeElement).toBe(address);
    // Keys the shell took from the page.
    web.emit({ type: 'key', action: 'returnFocus' });
    await settle(f);
    f.detectChanges();
    expect(document.activeElement).toBe(area);
    web.emit({ type: 'key', action: 'focusAddress' });
    await settle(f);
    f.detectChanges();
    expect(document.activeElement).toBe(address);
    const seen: KeyboardEvent[] = [];
    const listen = (e: KeyboardEvent) => seen.push(e);
    document.addEventListener('keydown', listen);
    try {
      web.emit({ type: 'key', action: 'shortcut', key: 'p', ctrl: true, shift: true, alt: false, meta: false });
      await settle(f);
      f.detectChanges();
      expect(seen.map((e) => [e.key, e.ctrlKey, e.shiftKey, e.altKey])).toEqual([['p', true, true, false]]);
    } finally {
      document.removeEventListener('keydown', listen);
      f.nativeElement.remove();
    }
  });

  it('says where the page is when it is not docked (Linux)', async () => {
    const backend = configure();
    vi.spyOn(backend, 'state').mockResolvedValue({ page: { open: true, url: 'https://e.org/', title: 'E', loading: false }, engine: 'google', customEngine: false, docked: false });
    const f = TestBed.createComponent(WebPanel);
    f.detectChanges();
    await until(f, () => (f.nativeElement as HTMLElement).textContent!.includes('a window of its own'));
  });
});

describe('BrowserReveal', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('shows the Browser panel when a page opens: in front for the user, quietly for the AI, never from a pop-out', async () => {
    configure();
    const reveal = TestBed.inject(BrowserReveal);
    const show = vi.spyOn(dock, 'show');
    reveal.start();
    reveal.start();
    await settle();
    web.show('https://e.org/a');
    expect(show).toHaveBeenLastCalledWith('web', { inactive: false });
    expect(show).toHaveBeenCalledTimes(1);
    web.show('https://e.org/b', false);
    expect(show).toHaveBeenLastCalledWith('web', { inactive: true });
    dock.openIds.set(new Set(['web']));
    web.show('https://e.org/c', false);
    expect(show, 'already open: the AI does not bring it to the front').toHaveBeenCalledTimes(2);
    reveal.onEvent({ type: 'state', open: true, url: '', title: '', loading: false });
    expect(show).toHaveBeenCalledTimes(2);
  });

  it('leaves it to the IMT window in a pop-out', async () => {
    configure([{ provide: POPOUT_ROUTE, useValue: { view: 'web', host: 'main', label: 'p-main-1' } }]);
    const show = vi.spyOn(dock, 'show');
    TestBed.inject(BrowserReveal).start();
    await settle();
    web.show('https://e.org/a');
    expect(show).not.toHaveBeenCalled();
  });
});

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

  it('lets the AI write the text first and never runs without one; data from the logs needs "Research anyway"', async () => {
    const { f, close, ai, root } = open({ question: 'why is WEB01 slow?' });
    const research = vi.spyOn(ai, 'research');
    expect(el(f, '[data-testid=research-run]').textContent).toContain('Write the search text');
    el<HTMLButtonElement>(f, '[data-testid=research-run]').click();
    await until(f, () => !!el<HTMLInputElement>(f, '[data-testid=research-query]').value);
    expect(research).not.toHaveBeenCalled();
    expect(el<HTMLInputElement>(f, '[data-testid=research-query]').value).toBe('WEB01 slow');
    expect(root.querySelector('[data-testid=research-flagged] mark')?.textContent).toContain('WEB01');
    expect(el(f, '[data-testid=research-run]').textContent).toContain('Research anyway');
    el<HTMLButtonElement>(f, '[data-testid=research-run]').click();
    await until(f, () => close.mock.calls.length > 0);
    expect(research.mock.calls[0]?.[0]).toMatchObject({ query: 'WEB01 slow', confirmed: true });
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
    expect(el(f, '[data-testid=research-source]').textContent).toContain('Built by IMT');
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
    expect(web.page.url).toBe('https://www.bing.com/search?q=TimeoutException+pool');
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
    expect(['duckDuckGo', 'google', 'bing', 'custom'].map((e) => engineLabel(e as never))).toEqual(['DuckDuckGo', 'Google', 'Bing', 'Custom']);
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
