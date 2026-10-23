import { Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { TimeCursor } from '../analysis/time-cursor';
import { RecordView } from '../core/api.types';
import { DebriefBackend } from '../core/backend';
import { Logger } from '../core/logger';
import { MockAiBackend, MockAiOptions } from '../core/mock/mock-ai-backend';
import { WorkspaceStore } from '../core/workspace.store';
import { DEBRIEF_RECORD_MENU_ITEMS, RecordMenuContribution } from '../context/record-actions';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { CommandContext, DEBRIEF_COMMANDS, DEBRIEF_VIEWS, DebriefCommand, DebriefView } from '../extensions/tokens';
import { UI_STATE_CONTRIBUTIONS, UiStateContribution } from '../lifecycle/ui-state';
import { DialogService } from '../ui/dialog.service';
import { MemoryLogger, settle, tinyBackend } from '../../testing/helpers';
import { AiBackend, AiError } from './ai-backend';
import { AiButton } from './ai-button';
import { AiChatPersistence } from './ai-chat-persistence';
import { AiLauncher } from './ai-launcher';
import * as lazy from './ai-lazy';
import { AiStatusStore } from './ai-status.store';
import { AiStatus, ChatHistoryRecord } from './ai.types';
import { NotificationsStore } from '../notifications/notifications.store';
import { Injector } from '@angular/core';
import { aiCommands, aiRecordMenu, aiViews, offerSetupOnFirstRun, provideAi, rangeToSummarize } from './provide-ai';
import { at } from '../../testing/must';

function setup(options: MockAiOptions = {}, extra: unknown[] = []) {
  const backend = new MockAiBackend({ stepMs: 0, ...options });
  TestBed.configureTestingModule({
    providers: [
      { provide: AiBackend, useValue: backend },
      { provide: Logger, useValue: new MemoryLogger() },
      { provide: DebriefBackend, useValue: tinyBackend() },
      ...(extra as never[]),
    ],
  });
  return { backend, status: TestBed.inject(AiStatusStore) };
}

const record = (id: number, message = `m${id}`) => ({ id, message }) as RecordView;

describe('AiStatusStore', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('reads the status and follows status and set-up events', async () => {
    const { backend, status } = setup({ mode: 'setup' });
    await settle();
    expect([status.available(), status.hasModel(), status.needsSetup(), status.setupRunning()]).toEqual([true, false, true, false]);
    status.noteSetup({ stage: 'downloading', bytes: 1, total: 2, bytesPerSec: 1, etaSecs: 1 });
    expect(status.setupRunning()).toBe(true);
    await backend.setupRun('qwen3.5-4b-q4km');
    await vi.waitFor(() => expect(status.status()?.state).toBe('ready'));
    expect(status.setup()?.stage).toBe('ready');
    expect(status.hasModel()).toBe(true);
  });

  it('records why the status could not be read', async () => {
    const { backend, status } = setup();
    vi.spyOn(backend, 'status').mockRejectedValue(new AiError('unavailable', 'not in this build'));
    expect(await status.refresh()).toBeNull();
    expect([status.status(), status.error(), status.available()]).toEqual([null, 'not in this build', false]);
    status.setStatus({ available: true, state: 'idle' } as AiStatus);
    expect([status.error(), status.hasModel()]).toEqual([null, true]);
  });

  it('survives backends whose event subscriptions fail', async () => {
    const backend = new MockAiBackend({ stepMs: 0 });
    vi.spyOn(backend, 'onStatus').mockRejectedValue(new Error('x'));
    vi.spyOn(backend, 'onSetup').mockRejectedValue(new Error('x'));
    TestBed.configureTestingModule({ providers: [{ provide: AiBackend, useValue: backend }, { provide: Logger, useClass: MemoryLogger }] });
    const s = TestBed.inject(AiStatusStore);
    await settle();
    expect(s.available()).toBe(true);
  });
});

describe('AiLauncher', () => {
  afterEach(() => TestBed.resetTestingModule());

  async function launcher(options: MockAiOptions = {}) {
    const { status } = setup(options);
    await settle();
    const dialogs = TestBed.inject(DialogService);
    const open = vi.spyOn(dialogs, 'open');
    return { l: TestBed.inject(AiLauncher), open, status, dialogs };
  }

  it('has the AI draft the search text of "Research this" and shows it before anything is searched', async () => {
    const { l, open } = await launcher();
    const backend = TestBed.inject(AiBackend);
    const research = vi.spyOn(backend, 'research');
    open.mockReturnValue({ closed: Promise.resolve(undefined), close: () => undefined } as never);
    expect(await l.research({ recordId: 4, maxResults: 3 })).toBeNull();
    expect(research).not.toHaveBeenCalled();
    expect(open.mock.calls.at(-1)?.[1].inputs).toMatchObject({
      query: 'System.UnauthorizedAccessException Access is denied 0x80070005',
      source: 'model',
      recordId: 4,
      maxResults: 3,
      segments: [],
    });
    // Workspace data in the draft: shown highlighted.
    await l.research({ question: 'why is WEB01 slow?' });
    expect(open.mock.calls.at(-1)?.[1].inputs).toMatchObject({ query: 'WEB01 slow', source: 'model' });
    expect((open.mock.calls.at(-1)?.[1].inputs as { segments: unknown[] }).segments.length).toBeGreaterThan(0);
    // A text the user saw on the chat's card starts at once.
    const opened = open.mock.calls.length;
    expect(await l.research({ question: 'q', query: 'E_ACCESSDENIED IIS' })).toBeNull();
    expect(research).toHaveBeenCalledTimes(1);
    expect(open.mock.calls.length).toBe(opened);
    // A draft that fails is reported.
    vi.spyOn(backend, 'researchDraft').mockRejectedValueOnce(new AiError('engine', 'the model is busy'));
    expect(await l.research({ question: 'q' })).toBe('the model is busy');
  });

  it('opens each dialog with its inputs', async () => {
    const { l, open } = await launcher();
    await l.openSetup();
    await l.openSetup('m1');
    await l.openModels();
    await l.askQuery('errors');
    await l.propose('markers', { recordId: 3 }, 'timeouts');
    await l.propose('pipeline', { recordId: null }, '', { chooseKind: true });
    const calls = open.mock.calls.map(([c, o]) => [c, o.title, o.inputs]);
    expect(calls).toEqual([
      [lazy.AiSetupWizard, 'Set up the AI assistant', {}],
      [lazy.AiSetupWizard, 'Set up the AI assistant', { modelId: 'm1' }],
      [lazy.AiModelManager, 'AI models', undefined],
      [lazy.AiQueryDialog, 'Ask AI to write a query', { initial: 'errors' }],
      [lazy.AiProposeDialog, 'Propose markers', { kind: 'markers', source: { recordId: 3 }, initialGoal: 'timeouts', chooseKind: false }],
      [lazy.AiProposeDialog, 'Propose profile changes', { kind: 'pipeline', source: { recordId: null }, initialGoal: '', chooseKind: true }],
    ]);
  });

  it('starts explain, summarise and root causes in the background, with a toast and no dialog', async () => {
    const { l, open } = await launcher();
    const backend = TestBed.inject(AiBackend);
    const start = vi.spyOn(backend, 'start');
    const toasts = TestBed.inject(NotificationsStore);
    await l.explainEvent(7);
    await l.summarize(1, 2);
    await l.rootCauses();
    expect(start.mock.calls.map(([r]) => r)).toEqual([{ op: 'explain', recordId: 7 }, { op: 'summarize', from: 1, to: 2 }, { op: 'rootCauses' }]);
    expect(open).not.toHaveBeenCalled();
    expect(toasts.toasts().map((t) => t.title)).toEqual([
      'Explaining event #7 — running in Jobs',
      'Summarizing the range — running in Jobs',
      'Looking for root causes — running in Jobs',
    ]);
    expect(toasts.toasts()[0]?.primary).toEqual({ type: 'showJobs' });
    // A request that does not start says why.
    start.mockRejectedValueOnce(new AiError('busy', '20 ai jobs are already waiting'));
    await l.explainEvent(8);
    const last = toasts.toasts().at(-1);
    expect([last?.severity, last?.title, last?.body]).toEqual(['error', 'The AI request did not start', '20 ai jobs are already waiting']);
  });

  it('resolves a suggested regex with the pattern the user accepted', async () => {
    const { l, open, dialogs } = await launcher();
    const p = l.suggestRegex(['a', 'b'], { intent: 'ids' });
    await vi.waitFor(() => expect(dialogs.stack().length).toBe(1));
    expect(at(open.mock.calls, 0)[1]?.inputs).toEqual({ lines: ['a', 'b'], intent: 'ids', highlight: false });
    at(dialogs.stack(), 0).ref.close('x+');
    expect(await p).toBe('x+');
    const q = l.suggestRegex(['a'], { highlight: true });
    await vi.waitFor(() => expect(dialogs.stack().length).toBe(1));
    expect(at(open.mock.calls, 1)[1]?.inputs).toMatchObject({ intent: '', highlight: true });
    at(dialogs.stack(), 0).ref.close();
    expect(await q).toBeNull();
  });

  it('opens the set-up wizard instead of a use case when there is no model', async () => {
    const { l, open } = await launcher({ mode: 'setup' });
    await l.explainEvent(1);
    await l.summarize(1, 2);
    await l.rootCauses();
    await l.askQuery();
    expect(await l.suggestRegex(['a'])).toBeNull();
    await l.propose('extractors', {});
    expect(open.mock.calls.every(([c]) => c === lazy.AiSetupWizard)).toBe(true);
    expect(open).toHaveBeenCalledTimes(6);
  });
});

@Component({
  imports: [AiButton],
  template: `<debrief-ai-button label="Ask AI" hint="h" testId="t" [disabled]="off()" (pressed)="count = count + 1" />`,
})
class ButtonHost {
  count = 0;
  readonly off = signal(false);
}

describe('AiButton', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('shows only while AI is available and reports presses', async () => {
    setup();
    const f = TestBed.createComponent(ButtonHost);
    f.detectChanges();
    await settle(f);
    f.detectChanges();
    const b = (f.nativeElement as HTMLElement).querySelector<HTMLButtonElement>('[data-testid="t"]')!;
    expect(b.textContent).toContain('Ask AI');
    expect(b.title).toBe('h');
    b.click();
    expect(f.componentInstance.count).toBe(1);
    f.componentInstance.off.set(true);
    f.detectChanges();
    expect(b.disabled).toBe(true);
  });

  it('renders nothing when AI is unavailable', async () => {
    setup({ mode: 'off' });
    const f = TestBed.createComponent(ButtonHost);
    f.detectChanges();
    await settle(f);
    f.detectChanges();
    expect((f.nativeElement as HTMLElement).querySelector('button')).toBeNull();
  });
});

describe('AiChatPersistence', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('captures and restores the chats through the engine', async () => {
    const { backend } = setup();
    const p = TestBed.inject(AiChatPersistence);
    expect(p.contribution.key).toBe('chats');
    expect(p.contribution.capture()).toBeNull();
    await p.sync();
    expect(p.history()).toBeNull();
    const c = await backend.chatNew();
    await p.sync();
    expect(p.history()?.sessions.map((s) => s.id)).toEqual([c.id]);
    const saved: ChatHistoryRecord = { version: 1, sessions: [{ id: 'chat-7', title: 'saved', turns: [] }] };
    p.contribution.restore(saved);
    await vi.waitFor(() => expect(p.restored()).toBe(1));
    expect(p.history()).toEqual(saved);
    expect(p.warning()).toBeNull();
    await p.restore({ version: 9, sessions: [] });
    expect(p.warning()).toContain('newer');
    p.contribution.reset!();
    await vi.waitFor(() => expect(p.restored()).toBe(3));
    expect(p.history()).toBeNull();
  });

  it('keeps saved chats untouched when the engine cannot take them', async () => {
    const { backend } = setup();
    const p = TestBed.inject(AiChatPersistence);
    vi.spyOn(backend, 'chatRestore').mockRejectedValue(new AiError('unavailable', 'off'));
    vi.spyOn(backend, 'chatHistory').mockRejectedValue(new AiError('unavailable', 'off'));
    const saved: ChatHistoryRecord = { version: 1, sessions: [] };
    await p.restore(saved);
    expect(p.history()).toBe(saved);
    await p.restore('garbage');
    expect(p.history()).toBeNull();
    await p.sync();
    const logger = TestBed.inject(Logger) as MemoryLogger;
    expect(logger.entries.some((e) => e.message.includes('could not be read'))).toBe(true);
  });
});

describe('provide-ai', () => {
  afterEach(() => TestBed.resetTestingModule());

  function ctx(): CommandContext {
    return { showView: vi.fn() } as unknown as CommandContext;
  }

  it('registers the view, commands, menu items and saved chats', async () => {
    TestBed.configureTestingModule({
      providers: [provideAi(), { provide: Logger, useClass: MemoryLogger }, { provide: DebriefBackend, useValue: tinyBackend() }],
    });
    const views = (TestBed.inject(DEBRIEF_VIEWS) as unknown as DebriefView[][]).flat();
    const chat = views.find((v) => v.id === 'ai-chat')!;
    expect(chat.defaultPosition).toBe('right');
    expect((await chat.component()).name).toContain('AiChatPanel');
    const cmds = (TestBed.inject(DEBRIEF_COMMANDS) as unknown as DebriefCommand[][]).flat();
    expect(cmds.some((c) => c.id === 'ai.setup')).toBe(true);
    const menus = TestBed.inject(DEBRIEF_RECORD_MENU_ITEMS) as readonly RecordMenuContribution[];
    expect(menus.length).toBeGreaterThan(0);
    const ui = (TestBed.inject(UI_STATE_CONTRIBUTIONS) as unknown as UiStateContribution[][]).flat();
    expect(ui.map((u) => u.key)).toContain('chats');
    const status = TestBed.inject(AiStatusStore);
    await settle();
    expect(chat.enabled!()).toBe(status.available());
  });

  it('picks the range to summarize', () => {
    setup();
    const cursor = TestBed.inject(TimeCursor);
    const ws = TestBed.inject(WorkspaceStore);
    expect(rangeToSummarize(cursor, ws)).toBeNull();
    ws.setRange(10, 20);
    expect(rangeToSummarize(cursor, ws)).toEqual({ from: 10, to: 20 });
    cursor.select({ from: 1, to: 5 });
    expect(rangeToSummarize(cursor, ws)).toEqual({ from: 1, to: 5 });
  });

  it('offers each command only when it applies and runs it through the launcher', async () => {
    const { status } = setup();
    const ws = TestBed.inject(WorkspaceStore);
    const cursor = TestBed.inject(TimeCursor);
    const launcher = TestBed.inject(AiLauncher);
    const spies = {
      openSetup: vi.spyOn(launcher, 'openSetup').mockResolvedValue(),
      openModels: vi.spyOn(launcher, 'openModels').mockResolvedValue(),
      explainEvent: vi.spyOn(launcher, 'explainEvent').mockResolvedValue(),
      summarize: vi.spyOn(launcher, 'summarize').mockResolvedValue(),
      askQuery: vi.spyOn(launcher, 'askQuery').mockResolvedValue(),
      rootCauses: vi.spyOn(launcher, 'rootCauses').mockResolvedValue(),
      propose: vi.spyOn(launcher, 'propose').mockResolvedValue(),
    };
    const cmds = aiCommands(launcher, status, ws, cursor);
    const byId = (id: string) => cmds.find((c) => c.id === id)!;
    const c = ctx();
    // Nothing yet: no status, no data, no selection.
    expect(cmds.filter((x) => x.when!(c)).map((x) => x.id)).toEqual([]);
    await settle();
    await ws.reload();
    await settle();
    expect(byId('ai.explain').when!(c)).toBe(false);
    expect(byId('ai.query').when!(c)).toBe(true);
    byId('ai.chat.show').run(c);
    expect(c.showView).toHaveBeenCalledWith('ai-chat');
    byId('ai.setup').run(c);
    byId('ai.models').run(c);
    byId('ai.query').run(c);
    byId('ai.rootCauses').run(c);
    // Explain and propose need a selected event; without one, running them does nothing.
    byId('ai.explain').run(c);
    ws.select(record(5));
    expect(byId('ai.explain').when!(c)).toBe(true);
    byId('ai.explain').run(c);
    for (const id of ['pipeline', 'markers', 'extractors']) {
      expect(byId(`ai.propose.${id}`).when!(c)).toBe(true);
      byId(`ai.propose.${id}`).run(c);
    }
    ws.select(null);
    for (const id of ['pipeline', 'markers', 'extractors']) byId(`ai.propose.${id}`).run(c);
    // Summarize: the whole workspace unless a range is chosen.
    expect(byId('ai.summarize').when!(c)).toBe(true);
    byId('ai.summarize').run(c);
    expect(spies.summarize).toHaveBeenCalledTimes(1);
    vi.spyOn(cursor, 'selection').mockReturnValue(null);
    ws.setRange(5, 5);
    // A degenerate range: nothing to summarize.
    const summary = ws.summary()!;
    vi.spyOn(ws, 'summary').mockReturnValue({ ...summary, from: null, to: null });
    byId('ai.summarize').run(c);
    expect(spies.summarize).toHaveBeenCalledTimes(1);
    expect(spies.openSetup).toHaveBeenCalled();
    expect(spies.openModels).toHaveBeenCalled();
    expect(spies.askQuery).toHaveBeenCalled();
    expect(spies.rootCauses).toHaveBeenCalled();
    expect(spies.explainEvent).toHaveBeenCalledWith(5);
    expect(spies.propose.mock.calls.map((x) => [x[0], x[1]])).toEqual([
      ['pipeline', { recordId: 5 }],
      ['markers', { recordId: 5 }],
      ['extractors', { recordId: 5 }],
      ['pipeline', { recordId: null }],
      ['markers', { recordId: null }],
      ['extractors', { recordId: null }],
    ]);
  });

  it('adds explain and regex items to the record menu while AI is available', async () => {
    const { status } = setup();
    const launcher = TestBed.inject(AiLauncher);
    const explain = vi.spyOn(launcher, 'explainEvent').mockResolvedValue();
    const regex = vi.spyOn(launcher, 'suggestRegex').mockResolvedValue(null);
    const menu = aiRecordMenu(launcher, status);
    expect(menu(record(1), {})).toEqual([]);
    await settle();
    const items = menu(record(1, 'one'), {});
    expect(items.map((i) => i.label)).toEqual(['Explain this event (AI)…', 'Suggest a regex for these lines (AI)…']);
    at(items, 0).run();
    at(items, 1).run();
    expect(explain).toHaveBeenCalledWith(1);
    expect(regex).toHaveBeenCalledWith(['one'], { highlight: true });
    at(menu(record(1), { records: [record(2, 'two'), record(3, 'three')] }), 1).run();
    expect(regex).toHaveBeenLastCalledWith(['two', 'three'], { highlight: true });
    expect(aiViews(status)[0]?.enabled!()).toBe(true);
  });
});

describe('offerSetupOnFirstRun', () => {
  const KEY = 'debrief.ai.setupOffered.v1';
  afterEach(() => {
    TestBed.resetTestingModule();
    localStorage.removeItem(KEY);
    vi.restoreAllMocks();
  });

  async function run(options: MockAiOptions, desktop: boolean) {
    const { status } = setup(options);
    const launcher = TestBed.inject(AiLauncher);
    const open = vi.spyOn(launcher, 'openSetup').mockResolvedValue();
    offerSetupOnFirstRun(status, launcher, TestBed.inject(Injector), desktop);
    await settle();
    TestBed.tick();
    return open;
  }

  it('does nothing in the browser build', async () => {
    expect(await run({ mode: 'setup' }, false)).not.toHaveBeenCalled();
    expect(localStorage.getItem(KEY)).toBeNull();
  });

  it('offers the set-up once on the desktop when there is no model', async () => {
    expect(await run({ mode: 'setup' }, true)).toHaveBeenCalledTimes(1);
    expect(localStorage.getItem(KEY)).toBe('1');
    TestBed.resetTestingModule();
    expect(await run({ mode: 'setup' }, true)).not.toHaveBeenCalled();
  });

  it('does not offer when a model is ready or AI is off', async () => {
    expect(await run({ mode: 'ready' }, true)).not.toHaveBeenCalled();
    TestBed.resetTestingModule();
    expect(await run({ mode: 'off' }, true)).not.toHaveBeenCalled();
    expect(localStorage.getItem(KEY)).toBeNull();
  });

  it('tolerates storage that throws', async () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('blocked');
    });
    expect(await run({ mode: 'setup' }, true)).not.toHaveBeenCalled();
    vi.restoreAllMocks();
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('full');
    });
    TestBed.resetTestingModule();
    expect(await run({ mode: 'setup' }, true)).toHaveBeenCalledTimes(1);
  });
});

// The command context is part of the launcher's world only through dialogs; keep the token import used.
void COMMAND_CONTEXT;
