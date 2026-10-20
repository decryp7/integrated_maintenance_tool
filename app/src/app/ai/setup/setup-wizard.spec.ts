import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { Logger } from '../../core/logger';
import { MockAiBackend, MockAiOptions } from '../../core/mock/mock-ai-backend';
import { COMMAND_CONTEXT } from '../../extensions/command-registry';
import { DIALOG_REF } from '../../ui/dialog.service';
import { MemoryLogger, settle } from '../../../testing/helpers';
import { AiBackend, AiError } from '../ai-backend';
import { SetupPlan } from '../ai.types';
import { Platform } from '../../core/platform';
import { AiSetupWizard } from './setup-wizard';

function setup(options: MockAiOptions = { mode: 'setup' }, modelId: string | null = null) {
  const backend = new MockAiBackend({ stepMs: 0, ...options });
  const ctx = { showView: vi.fn() };
  const ref = { close: vi.fn(), closed: new Promise(() => undefined) };
  TestBed.configureTestingModule({
    providers: [
      { provide: AiBackend, useValue: backend },
      { provide: Logger, useClass: MemoryLogger },
      { provide: COMMAND_CONTEXT, useValue: ctx },
      { provide: DIALOG_REF, useValue: ref },
    ],
  });
  const fixture = TestBed.createComponent(AiSetupWizard);
  if (modelId) fixture.componentRef.setInput('modelId', modelId);
  fixture.detectChanges();
  const el = fixture.nativeElement as HTMLElement;
  const q = (id: string) => el.querySelector<HTMLElement>(`[data-testid="${id}"]`);
  return { backend, ctx, ref, fixture, el, q };
}

async function until(
  fixture: { detectChanges(): void },
  cond: () => boolean,
  rounds = 200,
): Promise<void> {
  for (let i = 0; i < rounds && !cond(); i++) {
    await settle();
    fixture.detectChanges();
  }
  expect(cond()).toBe(true);
}

describe('AiSetupWizard', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('shows the hardware, the models with their facts, and the local-only promise', async () => {
    const { fixture, el, q, ref } = setup();
    expect(el.textContent).toContain('Checking this computer');
    await until(fixture, () => !!q('ai-hardware'));
    expect(q('ai-local-note')!.textContent).toContain('fully on this computer');
    expect(q('ai-local-note')!.textContent).toContain('Hugging Face');
    expect(q('ai-hardware')!.textContent).toContain('RTX 4060 Laptop GPU · 8 GB video memory');
    expect(q('ai-hardware')!.textContent).toContain('32 GB RAM');
    const best = q('ai-model-qwen3.5-9b-q4km')!;
    expect(best.textContent).toContain('Best');
    expect(best.textContent).toContain('Fits comfortably');
    expect(best.textContent).toContain('Fast');
    expect(best.textContent).toContain('apache-2.0');
    expect(best.textContent).toContain('Tool calling');
    expect(q('ai-model-qwen3.5-4b-q4km')!.textContent).toContain('Fastest');
    expect(q('ai-model-gemma3-12b-q4km')!.textContent).toContain('No tool calling');
    expect(q('ai-model-qwen3.5-32b-q4km')!.textContent).toContain('Too large');
    expect(q('ai-plan-summary')!.textContent).toContain('will be downloaded');
    // A model that is too large cannot be set up.
    q('ai-model-qwen3.5-32b-q4km')!.querySelector('input')!.dispatchEvent(new Event('change'));
    fixture.detectChanges();
    expect((q('ai-setup-start') as HTMLButtonElement).disabled).toBe(true);
    (el.querySelector('.btn.ghost') as HTMLButtonElement).click();
    expect(ref.close).toHaveBeenCalled();
  });

  it('downloads with progress and becomes ready, then opens the chat', async () => {
    const { fixture, q, backend, ctx, ref } = setup();
    const stages: string[] = [];
    void backend.onSetup((e) => stages.push(e.stage));
    await until(fixture, () => !!q('ai-setup-start'));
    q('ai-setup-start')!.click();
    await until(fixture, () => !!q('ai-setup-bytes'));
    expect(q('ai-setup-stage')!.textContent).toContain('Step 2 of 5');
    expect(
      fixture.nativeElement.querySelector('[role=progressbar]').getAttribute('aria-valuenow'),
    ).toMatch(/^\d+$/);
    expect(q('ai-setup-bytes')!.textContent).toMatch(/of 5\.6 GB/);
    await until(fixture, () => !!q('ai-setup-ready'));
    expect(stages).toContain('verifying');
    expect(q('ai-setup-ready')!.textContent).toContain('Vulkan');
    expect(q('ai-setup-live')!.textContent).toContain('ready');
    q('ai-open-chat')!.click();
    expect(ctx.showView).toHaveBeenCalledWith('ai-chat');
    expect(ref.close).toHaveBeenCalled();
  });

  it('sets up an installed or preselected model without downloading', async () => {
    const { fixture, q } = setup({ mode: 'setup' }, 'llama3.2-3b-q4km');
    await until(fixture, () => !!q('ai-plan-summary'));
    const radio = q('ai-model-llama3.2-3b-q4km')!.querySelector('input') as HTMLInputElement;
    expect(radio.checked).toBe(true);
  });

  it('explains a ready model is set up rather than downloaded', async () => {
    const { fixture, q, el } = setup({ mode: 'ready' });
    await until(fixture, () => !!q('ai-plan-summary'));
    q('ai-model-qwen3.5-4b-q4km')!.querySelector('input')!.dispatchEvent(new Event('change'));
    fixture.detectChanges();
    expect(q('ai-plan-summary')!.textContent).toContain('already downloaded');
    expect(q('ai-setup-start')!.textContent).toContain('Set up');
    q('ai-setup-start')!.click();
    await until(fixture, () => !!q('ai-setup-ready'));
    expect(el.textContent).toContain('Close');
  });

  it('can be cancelled and started again', async () => {
    const { fixture, q, backend } = setup({ mode: 'setup', stepMs: 5 });
    await until(fixture, () => !!q('ai-setup-start'));
    q('ai-setup-start')!.click();
    await until(fixture, () => !!q('ai-setup-cancel'));
    q('ai-setup-cancel')!.click();
    fixture.detectChanges();
    expect(q('ai-setup-cancel')!.textContent).toContain('Cancelling');
    await until(fixture, () => !!q('ai-setup-cancelled'), 2000);
    expect(await backend.status()).toMatchObject({ state: 'noModel' });
    (fixture.nativeElement.querySelector('.btn.primary') as HTMLButtonElement).click();
    await until(fixture, () => !!q('ai-setup-start'));
  });

  it('shows failures inline with Retry', async () => {
    const { fixture, q, el } = setup({ mode: 'setup', fail: ['setup'] });
    await until(fixture, () => !!q('ai-setup-start'));
    q('ai-setup-start')!.click();
    await until(fixture, () => !!q('ai-setup-error'));
    expect(q('ai-setup-error')!.textContent).toContain('connection');
    expect(q('ai-setup-live')!.textContent).toContain('failed');
    // A network failure offers the manual download of that model.
    expect(q('ai-manual-download')).not.toBeNull();
    [...el.querySelectorAll<HTMLButtonElement>('.foot .btn.primary')].find((b) => b.textContent!.includes('Retry'))!.click();
    await until(fixture, () => !!q('ai-setup-start'));
  });

  it('explains a TLS-inspecting proxy and sets up the model downloaded by hand', async () => {
    const platform = { pickOpenPath: vi.fn(async () => '/Downloads/Qwen3.5-9B-Q4_K_M.gguf') };
    TestBed.overrideProvider(Platform, { useValue: platform });
    const { fixture, q, backend } = setup({ mode: 'setup', net: 'tls' });
    await until(fixture, () => !!q('ai-setup-start'));
    q('ai-setup-start')!.click();
    await until(fixture, () => !!q('ai-manual-file-0'));
    expect(q('ai-setup-error')!.textContent).toContain('Zscaler');
    expect(q('ai-manual-kind')!.textContent).toContain('intercepts HTTPS');
    expect(q('ai-manual-file-0')!.textContent).toContain('Qwen3.5-9B-Q4_K_M.gguf');
    q('ai-manual-add')!.click();
    // Added and verified: the wizard starts again with the model installed.
    await until(fixture, () => !!q('ai-plan-summary'));
    expect(q('ai-plan-summary')!.textContent).toContain('already downloaded');
    expect((await backend.models()).some((m) => m.catalogId === 'qwen3.5-9b-q4km')).toBe(true);
  });

  it('does not offer the manual download for failures it cannot work around', async () => {
    const { fixture, q } = setup({ mode: 'setup' });
    await until(fixture, () => !!q('ai-setup-start'));
    fixture.componentInstance.dispatch({ type: 'failed', code: 'disk', message: 'not enough disk space' });
    fixture.detectChanges();
    expect(q('ai-setup-error')).not.toBeNull();
    expect(q('ai-manual-download')).toBeNull();
  });

  it('reports an unavailable engine and a rejected start', async () => {
    const off = setup({ mode: 'off' });
    await until(off.fixture, () => !!off.q('ai-setup-error'));
    expect(off.q('ai-setup-error')!.textContent).toContain('not installed');
    TestBed.resetTestingModule();
    const { fixture, q, backend } = setup();
    await until(fixture, () => !!q('ai-setup-start'));
    vi.spyOn(backend, 'setupRun').mockRejectedValue(new AiError('busy', 'already running'));
    q('ai-setup-start')!.click();
    await until(fixture, () => !!q('ai-setup-error'));
    expect(q('ai-setup-error')!.textContent).toContain('already running');
  });

  it('keeps cancelling when the cancel request itself fails, and shows the CPU fallback note', async () => {
    const { fixture, q, backend } = setup({ mode: 'setup', stepMs: 5 });
    await until(fixture, () => !!q('ai-setup-start'));
    q('ai-setup-start')!.click();
    await until(fixture, () => !!q('ai-setup-cancel'));
    const cmp = fixture.componentInstance;
    cmp.dispatch({
      type: 'setup',
      event: { stage: 'fellBackToCpu', reason: 'The GPU could not be used' },
    });
    fixture.detectChanges();
    expect(q('ai-setup-fallback')!.textContent).toContain('GPU could not be used');
    vi.spyOn(backend, 'setupCancel').mockRejectedValue(new Error('x'));
    await cmp.cancel();
    expect(cmp.state()).toMatchObject({ kind: 'running', cancelling: true });
    fixture.destroy();
  });

  it('asks for an inference runtime first when none is installed, then continues to the models', async () => {
    const { fixture, q } = setup({ mode: 'setup', runtime: 'none' });
    await until(fixture, () => !!q('ai-setup-runtime'));
    expect(q('ai-setup-runtime')!.textContent).toContain('Inference runtime');
    expect(q('ai-hardware')).toBeNull();
    const go = q('ai-runtime-continue') as HTMLButtonElement;
    expect(go.disabled).toBe(true);
    await until(fixture, () => !!q('ai-runtime-download-suggested'));
    q('ai-runtime-download-suggested')!.click();
    await until(fixture, () => !!q('ai-runtime-ready'));
    await until(fixture, () => !(q('ai-runtime-continue') as HTMLButtonElement).disabled);
    q('ai-runtime-continue')!.click();
    await until(fixture, () => !!q('ai-hardware'));
    q('ai-setup-start')!.click();
    await until(fixture, () => !!q('ai-setup-ready'));
    expect(q('ai-setup-ready')!.textContent).toContain('CUDA');
  });

  it('offers the runtime download when a set-up fails for want of one', async () => {
    // The runtime was deleted after the wizard checked (or the check failed): the load reports it.
    const { fixture, q } = setup({ mode: 'setup', runtime: 'none' });
    await until(fixture, () => !!q('ai-setup-runtime'));
    fixture.componentInstance.dispatch({ type: 'runtimeReady' });
    fixture.detectChanges();
    q('ai-setup-start')!.click();
    await until(fixture, () => !!q('ai-setup-error'));
    expect(q('ai-setup-error')!.textContent).toContain('no inference runtime is installed');
    await until(fixture, () => !!q('ai-runtime-download-suggested'));
    expect(q('ai-setup-runtime')).not.toBeNull();
  });

  it('shows plan notes, the fallback note when ready, and survives a failed event subscription', async () => {
    const backend = new MockAiBackend({ stepMs: 0, mode: 'setup' });
    const plan = await backend.setupPlan();
    vi.spyOn(backend as AiBackend, 'setupPlan').mockResolvedValue({
      ...plan,
      note: 'Could not reach the model host to confirm the licence',
    } as SetupPlan);
    vi.spyOn(backend, 'onSetup').mockRejectedValue(new Error('no events'));
    TestBed.configureTestingModule({
      providers: [
        { provide: AiBackend, useValue: backend },
        { provide: Logger, useClass: MemoryLogger },
        { provide: COMMAND_CONTEXT, useValue: { showView: vi.fn() } },
      ],
    });
    const fixture = TestBed.createComponent(AiSetupWizard);
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    await until(fixture, () => !!el.querySelector('.note'));
    expect(el.querySelector('.note')!.textContent).toContain('licence');
    const cmp = fixture.componentInstance;
    cmp.dispatch({ type: 'start' });
    cmp.dispatch({ type: 'setup', event: { stage: 'fellBackToCpu', reason: 'CPU instead' } });
    cmp.dispatch({ type: 'setup', event: { stage: 'ready', model: 'm', backend: 'CPU' } });
    fixture.detectChanges();
    expect(el.querySelector('.note')!.textContent).toContain('CPU instead');
    cmp.close();
  });
});
