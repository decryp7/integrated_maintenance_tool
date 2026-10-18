import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { FakeContext, provideAnalysisTesting } from '../../analysis/test-setup';
import { MockAiBackend, MockAiOptions } from '../../core/mock/mock-ai-backend';
import { Platform } from '../../core/platform';
import { DialogService } from '../../ui/dialog.service';
import { all, buttonByText, el, settle, typeInto } from '../../../testing/helpers';
import { AiBackend, AiError } from '../ai-backend';
import { AiChatPersistence } from '../ai-chat-persistence';
import { AiLauncher } from '../ai-launcher';
import { AiStatusStore } from '../ai-status.store';
import { AiChatPanel } from './chat-panel';
import { ChatStore } from './chat.store';
import { AiCitations } from './citation-actions';
import { at } from '../../../testing/must';

function setup(options: MockAiOptions = {}, path: string | null = '/tmp/chat.md') {
  const backend = new MockAiBackend({ stepMs: 0, ...options });
  const platform = Object.assign(new Platform(), { pickSavePath: vi.fn(async () => path), saveDialogConfirmsOverwrite: () => false });
  TestBed.configureTestingModule({
    providers: [...provideAnalysisTesting({ context: new FakeContext() }), { provide: AiBackend, useValue: backend }, { provide: Platform, useValue: platform }],
  });
  const f = TestBed.createComponent(AiChatPanel);
  const store = TestBed.inject(ChatStore);
  return { backend, platform, f, store, root: f.nativeElement as HTMLElement };
}

async function until(f: { detectChanges(): void }, cond: () => boolean) {
  for (let i = 0; i < 300 && !cond(); i++) {
    await settle();
    f.detectChanges();
  }
  expect(cond(), String(cond)).toBe(true);
}

async function ask(f: ReturnType<typeof setup>['f'], text: string) {
  typeInto(el<HTMLTextAreaElement>(f, 'textarea'), text);
  f.detectChanges();
  el<HTMLButtonElement>(f, 'button[type=submit]').click();
}

describe('AiChatPanel', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('offers starter prompts and streams an answer with tool activity and evidence links', async () => {
    const { f, root, store } = setup();
    f.detectChanges();
    await until(f, () => !!root.querySelector('.starter'));
    await until(f, () => !el<HTMLTextAreaElement>(f, 'textarea').disabled);
    expect(el(f, '[data-testid=ai-engine-state]').textContent).toContain('Qwen3.5-4B-Q4_K_M · ready · Vulkan');
    const record = vi.spyOn(TestBed.inject(AiCitations), 'record').mockResolvedValue();
    const query = vi.spyOn(TestBed.inject(AiCitations), 'query').mockResolvedValueOnce(null).mockResolvedValueOnce('bad');
    at(all<HTMLButtonElement>(f, '.starter'), 0).click();
    await until(f, () => !store.running() && !!root.querySelector('[data-testid=ai-tool-chip]'));
    expect(el(f, '[data-testid=ai-tool-chip]').textContent).toContain('Query: 1 events');
    expect(el(f, '.user').textContent).toContain('What happened around');
    const links = all<HTMLButtonElement>(f, 'button.cite');
    links.find((b) => b.classList.contains('rec'))!.click();
    expect(record).toHaveBeenCalled();
    links.find((b) => b.classList.contains('q'))!.click();
    await settle();
    f.detectChanges();
    expect(el(f, '.ok.banner').textContent).toContain('Applied level >= error');
    links.find((b) => b.classList.contains('q'))!.click();
    await settle();
    f.detectChanges();
    expect(el(f, '.ok.banner').textContent).toContain('Not applied: bad');
    expect(query).toHaveBeenCalledTimes(2);
  });

  it('sends with Enter but not Shift+Enter, and stops a running answer', async () => {
    const { f, store } = setup({ stepMs: 10 });
    f.detectChanges();
    await until(f, () => !!store.activeId() && !el<HTMLTextAreaElement>(f, 'textarea').disabled);
    const ta = el<HTMLTextAreaElement>(f, 'textarea');
    typeInto(ta, 'line one');
    ta.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', shiftKey: true }));
    await settle();
    expect(store.activeTurns()).toEqual([]);
    ta.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }));
    await until(f, () => store.running());
    f.detectChanges();
    expect(ta.value === '' || store.activeTurns().length === 1).toBe(true);
    buttonByText(f, 'Stop').click();
    await until(f, () => !store.running());
    expect(store.activeTurns()[0]?.cancelled).toBe(true);
    expect((f.nativeElement as HTMLElement).textContent).toContain('Stopped.');
  });

  it('renders proposals, neutral "not found" answers, notes, and errors', async () => {
    const { f, root, store, backend } = setup();
    f.detectChanges();
    await until(f, () => !!store.activeId() && !el<HTMLTextAreaElement>(f, 'textarea').disabled);
    await ask(f, 'write a regex for these');
    await until(f, () => !!root.querySelector('[data-testid=ai-proposal]'));
    await until(f, () => !store.running());
    await ask(f, 'anything about kubernetes?');
    await until(f, () => !store.running() && !!root.querySelector('.answer.neutral'));
    // A turn with an error.
    await ask(f, 'another');
    const id = store.activeId()!;
    store.onEvent({ sessionId: id, seq: 0, type: 'error', message: 'the engine crashed', cancelled: false });
    await until(f, () => !!root.querySelector('.err[role=alert]'));
    await until(f, () => !store.running());
    // The engine's own turn ends too before asking again.
    await vi.waitFor(async () => expect((await backend.chatSessions()).every((x) => !x.running)).toBe(true));
    const before = store.activeTurns().length;
    buttonByText(f, 'Ask again').click();
    await until(f, () => store.activeTurns().length === before + 1 && !store.running());
    store.onEvent({ sessionId: id, seq: 0, type: 'done', text: 'x', citations: [], usage: { inputTokens: 0, outputTokens: 0 }, notes: [] });
    // The proposal card's Dismiss marks it.
    buttonByText(f, 'Dismiss').click();
    f.detectChanges();
    expect(store.activeTurns()[0]?.proposals[0]?.state).toBe('dismissed');
  });

  it('shows what is sent as context and lets the user leave parts out', async () => {
    const { f, store } = setup();
    f.detectChanges();
    await until(f, () => !!store.activeId());
    const chip = el<HTMLButtonElement>(f, '[data-testid=ai-context-chip]');
    expect(chip.textContent).toContain('view');
    chip.click();
    f.detectChanges();
    const boxes = all<HTMLInputElement>(f, '#ai-chat-context input[type=checkbox]');
    expect(boxes).toHaveLength(5);
    at(boxes, 4).dispatchEvent(new Event('change'));
    f.detectChanges();
    expect(store.toggles().view).toBe(false);
    expect(chip.textContent).toContain('nothing selected');
    // The engine's notes about the context show under the answer.
    vi.spyOn(TestBed.inject(AiBackend), 'chatSend').mockResolvedValue();
    await store.send('q');
    store.onEvent({ sessionId: store.activeId()!, seq: 0, type: 'done', text: 'ok', citations: [], usage: { inputTokens: 0, outputTokens: 0 }, notes: ['3 selected event ids do not exist and were ignored.'] });
    f.detectChanges();
    expect(el(f, '.note').textContent).toContain('do not exist');
  });

  it('switches, creates, deletes and exports chats', async () => {
    const { f, store, platform, backend } = setup();
    f.detectChanges();
    await until(f, () => !!store.activeId() && !el<HTMLTextAreaElement>(f, 'textarea').disabled);
    expect(buttonByText(f, 'Export…').disabled).toBe(true);
    await ask(f, 'q');
    await until(f, () => !store.running());
    buttonByText(f, 'Export…').click();
    await until(f, () => !!(f.nativeElement as HTMLElement).querySelector('.ok.banner'));
    expect(platform.pickSavePath).toHaveBeenCalledWith('Export chat', 'q.md', [{ name: 'Markdown', extensions: ['md'] }]);
    expect(el(f, '.ok.banner').textContent).toContain('Exported to /tmp/chat.md');
    // Exporting to the same file again asks before replacing it.
    buttonByText(f, 'Export…').click();
    const dialogs = TestBed.inject(DialogService);
    await until(f, () => dialogs.stack().length === 1);
    at(dialogs.stack(), 0).ref.close(true);
    await settle();
    vi.spyOn(backend, 'chatExportMarkdown').mockRejectedValueOnce(new AiError('io', 'disk full'));
    buttonByText(f, 'Export…').click();
    await until(f, () => el(f, '.ok.banner').textContent!.includes('Not exported: disk full'));
    buttonByText(f, 'New chat').click();
    await until(f, () => store.sessions().length === 2);
    const select = el<HTMLSelectElement>(f, 'select');
    typeInto(select, at(store.sessions(), 0).id, 'change');
    await until(f, () => store.activeId() === at(store.sessions(), 0).id);
    el<HTMLButtonElement>(f, 'button[aria-label="Delete this chat"]').click();
    await until(f, () => store.sessions().length === 1);
  });

  it('does not export when the save dialog is cancelled', async () => {
    const { f, store, platform } = setup({}, null);
    f.detectChanges();
    await until(f, () => !!store.activeId() && !el<HTMLTextAreaElement>(f, 'textarea').disabled);
    await ask(f, 'q');
    await until(f, () => !store.running());
    await f.componentInstance.exportMarkdown();
    expect(platform.pickSavePath).toHaveBeenCalled();
    expect((f.nativeElement as HTMLElement).querySelector('.ok.banner')).toBeNull();
  });

  it('asks for a set-up when there is no model and reloads after restored chats', async () => {
    const { f, root, store } = setup({ mode: 'setup' });
    f.detectChanges();
    await until(f, () => !!root.querySelector('.setup'));
    const open = vi.spyOn(TestBed.inject(AiLauncher), 'openSetup').mockResolvedValue();
    buttonByText(f, 'Set up AI assistant…').click();
    expect(open).toHaveBeenCalled();
    expect(el<HTMLTextAreaElement>(f, 'textarea').disabled).toBe(true);
    const reload = vi.spyOn(store, 'reload').mockResolvedValue();
    TestBed.inject(AiChatPersistence).restored.update((n) => n + 1);
    await until(f, () => reload.mock.calls.length === 1);
  });

  it('shows the engine as unavailable and the store error banner', async () => {
    const { f, store } = setup({ mode: 'off' });
    f.detectChanges();
    await until(f, () => TestBed.inject(AiStatusStore).status() !== null && !!store.error());
    expect(el(f, '[data-testid=ai-engine-state]').textContent).toContain('AI unavailable');
    buttonByText(f, 'Dismiss').click();
    f.detectChanges();
    expect(store.error()).toBeNull();
    // Without a chat, delete and export do nothing.
    await f.componentInstance.deleteChat();
    await f.componentInstance.exportMarkdown();
  });
});
