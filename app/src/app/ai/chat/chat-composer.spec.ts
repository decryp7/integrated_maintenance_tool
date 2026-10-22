import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { FakeContext, provideAnalysisTesting } from '../../analysis/test-setup';
import { MockAiBackend, MockAiOptions } from '../../core/mock/mock-ai-backend';
import { RecordView } from '../../core/api.types';
import { WorkspaceStore } from '../../core/workspace.store';
import { resetMockWeb } from '../../browser/mock-browser-backend';
import { ChatBrowserStore } from '../../browser/chat-browser.store';
import { el, settle, typeInto } from '../../../testing/helpers';
import { AiBackend } from '../ai-backend';
import { AiLauncher } from '../ai-launcher';
import { AiStatusStore } from '../ai-status.store';
import { ChatComposer, COMPOSER_MAX_LINES, composerHeight } from './chat-composer';
import { ChatStore } from './chat.store';

function setup(options: MockAiOptions = {}) {
  const backend = new MockAiBackend({ stepMs: 0, ...options });
  TestBed.configureTestingModule({ providers: [...provideAnalysisTesting({ context: new FakeContext() }), { provide: AiBackend, useValue: backend }] });
  const store = TestBed.inject(ChatStore);
  const f = TestBed.createComponent(ChatComposer);
  return { backend, store, f, root: f.nativeElement as HTMLElement };
}

async function until(f: { detectChanges(): void }, cond: () => boolean) {
  for (let i = 0; i < 300 && !cond(); i++) {
    await settle();
    f.detectChanges();
  }
  expect(cond(), String(cond)).toBe(true);
}

async function ready(env: ReturnType<typeof setup>) {
  await env.store.init();
  env.f.componentRef.setInput('sessionId', env.store.activeId());
  env.f.detectChanges();
  await until(env.f, () => !el<HTMLTextAreaElement>(env.f, 'textarea').disabled);
}

describe('composerHeight', () => {
  it('follows the content up to the line limit', () => {
    expect(composerHeight(40, 20, 8)).toBe(40);
    expect(composerHeight(1000, 20, 8)).toBe(20 * COMPOSER_MAX_LINES + 8);
    expect(composerHeight(100, 20, 0, 3)).toBe(60);
  });
});

describe('ChatComposer', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('is one card with the question box, the toolbar and the keyboard hint; Send says why it is off', async () => {
    const env = setup();
    const { f, root } = env;
    await ready(env);
    const card = el(f, 'form.card');
    expect(card.querySelector('textarea')).not.toBeNull();
    expect(card.querySelector('[role=toolbar]')?.getAttribute('aria-label')).toBe('Question options');
    expect(el(f, '[data-testid=ai-attach]').getAttribute('aria-label')).toBe('Attach an image');
    expect(el(f, '#ai-chat-hint').textContent).toBe('Enter to send · Shift+Enter for a new line');
    expect(el(f, 'textarea').getAttribute('aria-describedby')).toBe('ai-chat-hint');
    const send = el<HTMLButtonElement>(f, '[data-testid=ai-send]');
    expect(send.disabled).toBe(true);
    expect(send.title).toBe('Type a question');
    typeInto(el<HTMLTextAreaElement>(f, 'textarea'), 'why?');
    f.detectChanges();
    expect(send.disabled).toBe(false);
    expect(send.title).toBe('Send (Enter)');
    // One Research entry, and the browser menu for this chat.
    expect(root.querySelectorAll('[data-testid=chat-research]')).toHaveLength(1);
    expect(el(f, '[data-testid=ai-browser-menu]').textContent).toContain('Browser: Off');
  });

  it('grows the question box with its content and never shows a resize grip', async () => {
    const env = setup();
    await ready(env);
    const ta = el<HTMLTextAreaElement>(env.f, 'textarea');
    expect(getComputedStyle(ta).resize).toBe('none');
    Object.defineProperty(ta, 'scrollHeight', { configurable: true, get: () => 10_000 });
    typeInto(ta, 'a\nb\nc');
    env.f.detectChanges();
    await settle();
    // jsdom has no layout: the height is the content's, capped at 8 lines.
    expect(parseFloat(ta.style.height)).toBeGreaterThan(0);
    expect(parseFloat(ta.style.height)).toBeLessThan(10_000);
  });

  it('stops a running answer with Esc, and Enter does not send while it runs', async () => {
    const env = setup({ stepMs: 20 });
    const { f, store } = env;
    await ready(env);
    const ta = el<HTMLTextAreaElement>(f, 'textarea');
    typeInto(ta, 'what crashed?');
    ta.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }));
    await until(f, () => store.running());
    f.detectChanges();
    expect(el(f, '[data-testid=ai-stop]').textContent?.trim()).toBe('Stop');
    typeInto(ta, 'again');
    ta.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }));
    const idle = new KeyboardEvent('keydown', { key: 'Escape', cancelable: true });
    ta.dispatchEvent(idle);
    expect(idle.defaultPrevented).toBe(true);
    await until(f, () => !store.running());
    expect(store.activeTurns()).toHaveLength(1);
    expect(store.activeTurns()[0]?.cancelled).toBe(true);
    // Esc while idle is left alone.
    const esc = new KeyboardEvent('keydown', { key: 'Escape', cancelable: true });
    ta.dispatchEvent(esc);
    expect(esc.defaultPrevented).toBe(false);
  });

  it('offers the selected events once there are some, and chooses automatically until the user picks', async () => {
    const env = setup();
    const { f, store } = env;
    await ready(env);
    const button = () => el(f, '[data-testid=ai-context-menu]').textContent ?? '';
    expect(button()).toContain('Context: Current view');
    TestBed.inject(WorkspaceStore).select({ id: 3 } as RecordView);
    f.detectChanges();
    expect(button()).toContain('Context: Selected events (1)');
    expect(el(f, '[data-testid=ai-context-menu]').title).toContain('The selected events, and');
    store.setContextMode('view');
    f.detectChanges();
    expect(button()).toContain('Context: Current view');
    store.setContextMode('none');
    f.detectChanges();
    expect(el(f, '[data-testid=ai-context-menu]').title).toBe('None: Only your question');
  });

  it('switches the browser mode from its menu and warns about small models for agent mode', async () => {
    const web = resetMockWeb();
    web.smallModel = true;
    const env = setup();
    const { f, store } = env;
    await ready(env);
    const id = store.activeId()!;
    // The chat panel's browser bar loads the chat's status.
    await TestBed.inject(ChatBrowserStore).load(id);
    el<HTMLButtonElement>(f, '[data-testid=ai-browser-menu]').click();
    f.detectChanges();
    await until(f, () => !!document.querySelector('[data-testid=ai-browser-agent]'));
    expect(document.querySelector('[data-testid=ai-browser-agent]')?.textContent).toContain('7B or larger');
    document.querySelector<HTMLButtonElement>('[data-testid=ai-browser-readAlong]')!.click();
    await until(f, () => web.status(id).readAlong);
    await until(f, () => el(f, '[data-testid=ai-browser-menu]').textContent!.includes('Browser: Read along'));
  });

  it('opens Research… prefilled with the draft', async () => {
    const env = setup();
    await ready(env);
    const research = vi.spyOn(TestBed.inject(AiLauncher), 'askResearch').mockResolvedValue();
    el<HTMLButtonElement>(env.f, '[data-testid=chat-research]').click();
    expect(research).toHaveBeenLastCalledWith({ sessionId: env.store.activeId(), question: undefined });
    typeInto(el<HTMLTextAreaElement>(env.f, 'textarea'), '  why does WEB02 deny access?  ');
    el<HTMLButtonElement>(env.f, '[data-testid=chat-research]').click();
    expect(research).toHaveBeenLastCalledWith({ sessionId: env.store.activeId(), question: 'why does WEB02 deny access?' });
    // With no draft, the chat's last question.
    await env.store.send('what crashed?');
    await until(env.f, () => !env.store.running());
    typeInto(el<HTMLTextAreaElement>(env.f, 'textarea'), '');
    el<HTMLButtonElement>(env.f, '[data-testid=chat-research]').click();
    expect(research).toHaveBeenLastCalledWith({ sessionId: env.store.activeId(), question: 'what crashed?' });
  });

  it('says why images cannot be attached and questions not sent without a model', async () => {
    const env = setup({ mode: 'setup' });
    await env.store.init();
    await until(env.f, () => TestBed.inject(AiStatusStore).status() !== null);
    const attach = el<HTMLButtonElement>(env.f, '[data-testid=ai-attach]');
    expect(attach.disabled).toBe(true);
    expect(attach.title).toBe('Set up the AI assistant first');
    expect(el<HTMLButtonElement>(env.f, '[data-testid=ai-send]').title).toBe('Set up the AI assistant first');
  });
});
