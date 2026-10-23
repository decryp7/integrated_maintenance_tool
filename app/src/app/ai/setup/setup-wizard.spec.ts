import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { Logger } from '../../core/logger';
import { MockAiBackend, MockAiOptions } from '../../core/mock/mock-ai-backend';
import { DIALOG_REF } from '../../ui/dialog.service';
import { MemoryLogger, settle } from '../../../testing/helpers';
import { AiBackend, AiError } from '../ai-backend';
import { SetupPlan } from '../ai.types';
import { AiSetupWizard } from './setup-wizard';

function setup(options: MockAiOptions = { mode: 'setup' }, modelId: string | null = null) {
  const backend = new MockAiBackend({ stepMs: 0, ...options });
  const ref = { close: vi.fn(), closed: new Promise(() => undefined) };
  TestBed.configureTestingModule({
    providers: [
      { provide: AiBackend, useValue: backend },
      { provide: Logger, useClass: MemoryLogger },
      { provide: DIALOG_REF, useValue: ref },
    ],
  });
  const fixture = TestBed.createComponent(AiSetupWizard);
  if (modelId) fixture.componentRef.setInput('modelId', modelId);
  fixture.detectChanges();
  const el = fixture.nativeElement as HTMLElement;
  const q = (id: string) => el.querySelector<HTMLElement>(`[data-testid="${id}"]`);
  return { backend, ref, fixture, el, q };
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
    // The numbers behind the verdict, and what to change when it is not comfortable.
    expect(q('ai-fit-qwen3.5-9b-q4km')!.textContent).toMatch(/Needs (about|roughly) \d+\.\d GB \(model \d+\.\d \+ context \d+\.\d/);
    expect(q('ai-fit-qwen3.5-9b-q4km')!.textContent).toContain('available now');
    expect(q('ai-fit-advice-qwen3.5-9b-q4km')).toBeNull();
    expect(q('ai-fit-advice-qwen3.5-32b-q4km')!.textContent).toContain('Fits this computer');
    expect(q('ai-fit-advice-qwen3.5-32b-q4km')!.textContent).toContain('smaller model');
    // A model that is too large cannot be set up.
    q('ai-model-qwen3.5-32b-q4km')!.querySelector('input')!.dispatchEvent(new Event('change'));
    fixture.detectChanges();
    expect((q('ai-setup-start') as HTMLButtonElement).disabled).toBe(true);
    (el.querySelector('.btn.ghost') as HTMLButtonElement).click();
    expect(ref.close).toHaveBeenCalled();
  });

  it('"Download and set up" hands the choice to the engine as a background job and closes at once', async () => {
    const { fixture, q, backend, ref } = setup();
    const started = vi.spyOn(backend, 'setupBackground');
    const stages: string[] = [];
    void backend.onSetup((e) => stages.push(e.stage));
    await until(fixture, () => !!q('ai-setup-start'));
    q('ai-setup-start')!.click();
    await until(fixture, () => ref.close.mock.calls.length > 0);
    expect(started).toHaveBeenCalledWith('qwen3.5-9b-q4km');
    // The dialog shows no progress of its own: the download is not tracked in the window.
    expect(q('ai-setup-stage')).toBeNull();
    expect(q('ai-setup-bytes')).toBeNull();
    expect(fixture.nativeElement.querySelector('[role=progressbar]')).toBeNull();
  });

  it('sets up an installed or preselected model without downloading', async () => {
    const { fixture, q } = setup({ mode: 'setup' }, 'llama3.2-3b-q4km');
    await until(fixture, () => !!q('ai-plan-summary'));
    const radio = q('ai-model-llama3.2-3b-q4km')!.querySelector('input') as HTMLInputElement;
    expect(radio.checked).toBe(true);
  });

  it('explains a ready model is set up rather than downloaded', async () => {
    const { fixture, q, backend, ref } = setup({ mode: 'ready' });
    const started = vi.spyOn(backend, 'setupBackground');
    await until(fixture, () => !!q('ai-plan-summary'));
    q('ai-model-qwen3.5-4b-q4km')!.querySelector('input')!.dispatchEvent(new Event('change'));
    fixture.detectChanges();
    expect(q('ai-plan-summary')!.textContent).toContain('already downloaded');
    expect(q('ai-setup-start')!.textContent).toContain('Set up');
    q('ai-setup-start')!.click();
    await until(fixture, () => ref.close.mock.calls.length > 0);
    expect(started).toHaveBeenCalledWith('qwen3.5-4b-q4km');
  });

  it('says the inference runtime comes first when none is installed, without a step of its own', async () => {
    const { fixture, q } = setup({ mode: 'setup', runtime: 'none' });
    await until(fixture, () => !!q('ai-hardware'));
    expect(q('ai-setup-runtime')).toBeNull();
    expect(q('ai-runtime-continue')).toBeNull();
    await until(fixture, () => !!q('ai-setup-runtime-note'));
    expect(q('ai-setup-runtime-note')!.textContent).toMatch(/inference runtime .* is downloaded first/);
  });

  it('shows no runtime note when a runtime is installed', async () => {
    const { fixture, q } = setup();
    await until(fixture, () => !!q('ai-hardware'));
    expect(q('ai-setup-runtime-note')).toBeNull();
  });

  it('shows a rejected start in the dialog, which stays open and can start again', async () => {
    const { fixture, q, backend, ref } = setup();
    await until(fixture, () => !!q('ai-setup-start'));
    const spy = vi.spyOn(backend, 'setupBackground').mockRejectedValueOnce(new AiError('busy', 'already running'));
    q('ai-setup-start')!.click();
    await until(fixture, () => !!q('ai-setup-start-error'));
    expect(q('ai-setup-start-error')!.textContent).toContain('already running');
    expect(ref.close).not.toHaveBeenCalled();
    expect((q('ai-setup-start') as HTMLButtonElement).disabled).toBe(false);
    q('ai-setup-start')!.click();
    await until(fixture, () => ref.close.mock.calls.length > 0);
    expect(spy).toHaveBeenCalledTimes(2);
  });

  it('does not start twice while the first request is on its way', async () => {
    const { fixture, q, backend } = setup();
    await until(fixture, () => !!q('ai-setup-start'));
    const spy = vi.spyOn(backend, 'setupBackground').mockReturnValue(new Promise(() => undefined));
    void fixture.componentInstance.start();
    await fixture.componentInstance.start();
    await fixture.componentInstance.start();
    expect(spy).toHaveBeenCalledTimes(1);
  });

  it('reports an unavailable engine with Retry', async () => {
    const off = setup({ mode: 'off' });
    await until(off.fixture, () => !!off.q('ai-setup-error'));
    expect(off.q('ai-setup-error')!.textContent).toContain('not installed');
    [...off.el.querySelectorAll<HTMLButtonElement>('.foot .btn.primary')].find((b) => b.textContent!.includes('Retry'))!.click();
    off.fixture.detectChanges();
    await until(off.fixture, () => !!off.q('ai-setup-error'));
  });

  it('shows plan notes', async () => {
    const backend = new MockAiBackend({ stepMs: 0, mode: 'setup' });
    const plan = await backend.setupPlan();
    vi.spyOn(backend as AiBackend, 'setupPlan').mockResolvedValue({
      ...plan,
      note: 'Could not reach the model host to confirm the licence',
    } as SetupPlan);
    TestBed.configureTestingModule({
      providers: [
        { provide: AiBackend, useValue: backend },
        { provide: Logger, useClass: MemoryLogger },
      ],
    });
    const fixture = TestBed.createComponent(AiSetupWizard);
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    await until(fixture, () => !!el.querySelector('.note'));
    expect(el.querySelector('.note')!.textContent).toContain('licence');
    fixture.componentInstance.close();
  });
});
