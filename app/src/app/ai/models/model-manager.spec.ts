import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { Logger } from '../../core/logger';
import { MockAiBackend, MockAiOptions } from '../../core/mock/mock-ai-backend';
import { Platform } from '../../core/platform';
import { DialogService } from '../../ui/dialog.service';
import { MemoryLogger, settle } from '../../../testing/helpers';
import { AiBackend, AiError } from '../ai-backend';
import { AiStatusStore } from '../ai-status.store';
import { AiSetupWizard } from '../setup/setup-wizard';
import { AiModelManager } from './model-manager';

function setup(options: MockAiOptions = {}, pick: string | null = '/models/My Model.gguf') {
  const backend = new MockAiBackend({ stepMs: 0, ...options });
  const platform = { pickOpenPath: vi.fn(async () => pick) };
  TestBed.configureTestingModule({
    providers: [
      { provide: AiBackend, useValue: backend },
      { provide: Logger, useClass: MemoryLogger },
      { provide: Platform, useValue: platform },
    ],
  });
  const dialogs = TestBed.inject(DialogService);
  const fixture = TestBed.createComponent(AiModelManager);
  fixture.detectChanges();
  const el = fixture.nativeElement as HTMLElement;
  const q = (id: string) => el.querySelector<HTMLElement>(`[data-testid="${id}"]`);
  const button = (label: string) => [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.getAttribute('aria-label') === label || b.textContent!.trim() === label);
  return { backend, dialogs, platform, fixture, el, q, button };
}

async function until(fixture: { detectChanges(): void }, cond: () => boolean, rounds = 200): Promise<void> {
  for (let i = 0; i < rounds && !cond(); i++) {
    await settle();
    fixture.detectChanges();
  }
  expect(cond()).toBe(true);
}

/** Answers the topmost dialog (the delete confirmation). */
function answer(dialogs: DialogService, result: boolean | undefined) {
  dialogs.stack().at(-1)!.ref.close(result);
}

describe('AiModelManager', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('lists installed models, the engine and the other catalog models', async () => {
    const { fixture, q, el } = setup();
    await until(fixture, () => !!q('ai-installed-Qwen3.5-4B-Q4_K_M.gguf'));
    const row = q('ai-installed-Qwen3.5-4B-Q4_K_M.gguf')!;
    expect(row.textContent).toContain('Active');
    expect(row.textContent).toContain('Fits comfortably');
    expect(row.textContent).toContain('2.7 GB');
    expect(row.textContent).toContain('apache-2.0');
    expect(row.textContent).toContain('tool calling');
    await until(fixture, () => q('ai-engine-status')!.textContent!.includes('Vulkan'));
    expect(q('ai-engine-status')!.textContent).toContain('Running');
    expect(q('ai-engine-status')!.textContent).toContain('3.2 GB in memory');
    expect(el.textContent).toContain('Qwen3.5 9B');
    expect(q('ai-engine-reset')).toBeNull();
  });

  it('deletes only after confirmation', async () => {
    const { fixture, q, dialogs, button, backend } = setup();
    await until(fixture, () => !!button('Delete Qwen3.5 4B'));
    button('Delete Qwen3.5 4B')!.click();
    await settle();
    expect(dialogs.stack().at(-1)!.options.inputs!['message']).toContain('no model until you choose another');
    answer(dialogs, undefined);
    await settle();
    expect((await backend.models()).length).toBe(1);
    button('Delete Qwen3.5 4B')!.click();
    await settle();
    answer(dialogs, true);
    await until(fixture, () => !!q('ai-no-models'));
    expect(TestBed.inject(AiStatusStore).needsSetup()).toBe(true);
  });

  it('adds a local GGUF and activates it with progress', async () => {
    const { fixture, q, button, platform } = setup();
    await until(fixture, () => !!q('ai-installed-Qwen3.5-4B-Q4_K_M.gguf'));
    q('ai-add-local')!.click();
    await until(fixture, () => !!q('ai-installed-My_Model.gguf'));
    expect(platform.pickOpenPath).toHaveBeenCalledWith('Add a GGUF model', [{ name: 'GGUF model', extensions: ['gguf'] }]);
    expect(q('ai-model-progress')!.textContent).toContain('was added');
    button('Use My_Model')!.click();
    fixture.detectChanges();
    await until(fixture, () => q('ai-model-progress')?.textContent?.includes('is now active') ?? false);
    await until(fixture, () => q('ai-installed-My_Model.gguf')!.textContent!.includes('Active'));
  });

  it('does nothing when the file dialog is cancelled and shows add errors', async () => {
    const none = setup({}, null);
    await settle();
    none.q('ai-add-local')!.click();
    await settle();
    expect(none.q('ai-models-error')).toBeNull();
    TestBed.resetTestingModule();
    const bad = setup({}, '/x/model.bin');
    await until(bad.fixture, () => !!bad.q('ai-installed-Qwen3.5-4B-Q4_K_M.gguf'));
    bad.q('ai-add-local')!.click();
    await until(bad.fixture, () => !!bad.q('ai-models-error'));
    expect(bad.q('ai-models-error')!.textContent).toContain('.gguf');
  });

  it('shows load and activation failures with Retry', async () => {
    const { fixture, q, backend, button } = setup({ fail: ['models'] });
    await until(fixture, () => !!q('ai-models-error'));
    expect(q('ai-models-error')!.textContent).toContain('simulated');
    vi.spyOn(backend, 'models').mockResolvedValue([]);
    (q('ai-models-error')!.querySelector('button') as HTMLButtonElement).click();
    await until(fixture, () => !!q('ai-no-models'));
    TestBed.resetTestingModule();
    const s = setup();
    await until(s.fixture, () => !!s.q('ai-add-local'));
    await s.fixture.componentInstance.addLocal();
    s.fixture.detectChanges();
    vi.spyOn(s.backend, 'activateModel').mockRejectedValue(new AiError('busy', 'busy now'));
    s.button('Use My_Model')!.click();
    await until(s.fixture, () => s.q('ai-models-error')?.textContent?.includes('busy now') ?? false);
    vi.spyOn(s.backend, 'deleteModel').mockRejectedValue(new AiError('io', 'locked'));
    s.button('Delete My_Model')!.click();
    await settle();
    answer(s.dialogs, true);
    await until(s.fixture, () => s.q('ai-models-error')?.textContent?.includes('locked') ?? false);
    expect(button).toBeDefined();
  });

  it('follows activation stages, failures and CPU fallback notes', async () => {
    const { fixture, q } = setup();
    await until(fixture, () => !!q('ai-add-local'));
    const cmp = fixture.componentInstance;
    const onSetup = (cmp as unknown as { onSetup(e: unknown): void }).onSetup.bind(cmp);
    onSetup({ stage: 'loading', gpu: true });
    expect(cmp.progress()).toBe('');
    cmp.busy.set(true);
    onSetup({ stage: 'loading', gpu: true });
    expect(cmp.progress()).toContain('GPU');
    onSetup({ stage: 'fellBackToCpu', reason: 'GPU lost' });
    expect(cmp.progress()).toBe('GPU lost');
    onSetup({ stage: 'failed', code: 'engine', message: 'boom' });
    await settle();
    expect(cmp.error()).toBe('boom');
    expect(cmp.busy()).toBe(false);
  });

  it('resets a switched-off engine and opens the wizard for another model', async () => {
    const { fixture, q, backend, dialogs, button } = setup();
    await until(fixture, () => !!button('Download Qwen3.5 9B'));
    const status = TestBed.inject(AiStatusStore);
    status.setStatus({ ...(await backend.status()), state: 'disabled', reason: 'The AI engine failed 3 times' });
    fixture.detectChanges();
    expect(q('ai-engine-status')!.textContent).toContain('Switched off');
    q('ai-engine-reset')!.click();
    await until(fixture, () => !q('ai-engine-reset'));
    vi.spyOn(backend, 'reset').mockRejectedValue(new AiError('engine', 'still broken'));
    status.setStatus({ ...(await backend.status()), state: 'failed', reason: 'x' });
    fixture.detectChanges();
    q('ai-engine-reset')!.click();
    await until(fixture, () => !!q('ai-models-error'));
    button('Download Qwen3.5 9B')!.click();
    const top = dialogs.stack().at(-1)!;
    expect(top.component).toBe(AiSetupWizard);
    expect(top.options.inputs).toEqual({ modelId: 'qwen3.5-9b-q4km' });
    status.setStatus(null as never);
    fixture.detectChanges();
    expect(q('ai-engine-status')!.textContent).toContain('unknown');
  });

  it('shows why a model file cannot be used, even without setup events', async () => {
    const backend = new MockAiBackend({ stepMs: 0 });
    const [m] = await backend.models();
    vi.spyOn(backend, 'models').mockResolvedValue([{ ...m, active: false, problem: 'not a usable model file: truncated' }]);
    vi.spyOn(backend, 'onSetup').mockRejectedValue(new Error('no events'));
    TestBed.configureTestingModule({ providers: [{ provide: AiBackend, useValue: backend }, { provide: Logger, useClass: MemoryLogger }] });
    const fixture = TestBed.createComponent(AiModelManager);
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    await until(fixture, () => el.textContent!.includes('truncated'));
    const use = [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent!.trim() === 'Use')!;
    expect(use.disabled).toBe(true);
  });
});
