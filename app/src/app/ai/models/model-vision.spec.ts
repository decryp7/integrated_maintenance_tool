import { TestBed } from '@angular/core/testing';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Logger } from '../../core/logger';
import { MockAiBackend, MockAiOptions } from '../../core/mock/mock-ai-backend';
import { Platform } from '../../core/platform';
import { MockJobs, resetMockJobs } from '../../jobs/mock-jobs';
import { MemoryLogger, settle } from '../../../testing/helpers';
import { AiBackend, AiError } from '../ai-backend';
import { visionLabel } from '../ai-format';
import { AiStatusStore } from '../ai-status.store';
import { AiSetupWizard } from '../setup/setup-wizard';
import { AiModelManager } from './model-manager';
import { COMMAND_CONTEXT } from '../../extensions/command-registry';
import { DIALOG_REF } from '../../ui/dialog.service';
import { FakeContext } from '../../analysis/test-setup';

function setup(options: MockAiOptions = {}, pick: string | null = '/models/mmproj-Qwen3.5-4B-F16.gguf') {
  const backend = new MockAiBackend({ stepMs: 0, ...options });
  const platform = { pickOpenPath: vi.fn(async () => pick) };
  TestBed.configureTestingModule({
    providers: [
      { provide: AiBackend, useValue: backend },
      { provide: Logger, useClass: MemoryLogger },
      { provide: Platform, useValue: platform },
    ],
  });
  const f = TestBed.createComponent(AiModelManager);
  f.detectChanges();
  const el = f.nativeElement as HTMLElement;
  const q = (id: string) => el.querySelector<HTMLElement>(`[data-testid="${id}"]`);
  return { backend, platform, f, el, q };
}

async function until(f: { detectChanges(): void }, cond: () => boolean) {
  for (let i = 0; i < 300 && !cond(); i++) {
    await settle();
    f.detectChanges();
  }
  expect(cond()).toBe(true);
}

const ROW = 'ai-installed-Qwen3.5-4B-Q4_K_M.gguf';

describe('AiModelManager image support', () => {
  beforeEach(() => resetMockJobs(new MockJobs({ visibleMs: 0, attentionMs: 0, failOnce: new Set() })));
  afterEach(() => {
    TestBed.resetTestingModule();
    resetMockJobs();
  });

  it('labels installed and catalog models "Text + images" or "Text only", with the catalog notes', async () => {
    const { f, q, el } = setup();
    await until(f, () => !!q(ROW));
    expect(q(ROW)!.querySelector('[data-testid=ai-model-vision]')!.textContent).toContain('Text + images');
    expect(q(ROW)!.textContent).toContain('Qwen3.5-4B-Q4_K_M.mmproj.gguf');
    expect(q(ROW)!.querySelector('[data-testid=ai-add-image-support]')).toBeNull();
    expect(q(ROW)!.querySelector('[data-testid=ai-add-projector]')).toBeNull();
    const catalog = [...el.querySelectorAll('section')].at(-1)!.textContent!;
    expect(catalog).toContain('Llama 3.2 3B');
    expect(catalog).toMatch(/Llama 3\.2 3B[\s\S]*Text only/);
    expect(catalog).toContain('Qwen3.6 35B-A3B');
    expect(catalog).toContain('needs 32 GB of RAM');
    expect([...el.querySelectorAll('[data-testid=ai-catalog-notes]')].length).toBeGreaterThan(3);
    expect(visionLabel(true)).toBe('Text + images');
    expect(visionLabel(false)).toBe('Text only');
  });

  it('adds the catalog image support of a text-only model as a download job', async () => {
    const { f, q, backend } = setup({ vision: 'missing', stepMs: 2 });
    await until(f, () => !!q(ROW));
    const row = q(ROW)!;
    expect(row.textContent).toContain('Text only');
    const add = row.querySelector<HTMLButtonElement>('[data-testid=ai-add-image-support]')!;
    expect(add.textContent).toContain('Add image support (640 MB)');
    const install = vi.spyOn(backend, 'installProjector');
    add.click();
    await until(f, () => q('ai-model-progress')?.textContent?.includes('running in Jobs') ?? false);
    expect(install).toHaveBeenCalledWith('Qwen3.5-4B-Q4_K_M.gguf');
    await until(f, () => q('ai-model-progress')?.textContent?.includes('now reads images') ?? false);
    expect(q(ROW)!.textContent).toContain('Text + images');
    expect(TestBed.inject(AiStatusStore).status()?.vision).toBe(true);
  });

  it('reports a failed download and a refused start', async () => {
    const { f, q, backend } = setup({ vision: 'missing', net: 'offline' });
    await until(f, () => !!q(ROW));
    q(ROW)!.querySelector<HTMLButtonElement>('[data-testid=ai-add-image-support]')!.click();
    await until(f, () => !!q('ai-models-error'));
    vi.spyOn(backend, 'installProjector').mockRejectedValueOnce(new AiError('busy', 'a download is running'));
    q(ROW)!.querySelector<HTMLButtonElement>('[data-testid=ai-add-image-support]')!.click();
    await until(f, () => q('ai-models-error')?.textContent?.includes('a download is running') ?? false);
  });

  it('adds a projector file the user chooses, and reports one that does not fit', async () => {
    const { f, q, platform, backend } = setup({ vision: 'none' });
    await until(f, () => !!q(ROW));
    expect(q(ROW)!.querySelector('[data-testid=ai-add-image-support]')).toBeNull();
    const addFile = q(ROW)!.querySelector<HTMLButtonElement>('[data-testid=ai-add-projector]')!;
    expect(addFile.getAttribute('aria-label')).toBe('Add a projector file for Qwen3.5 4B');
    const spy = vi.spyOn(backend, 'addProjector');
    addFile.click();
    await until(f, () => q('ai-model-progress')?.textContent?.includes('now reads images') ?? false);
    expect(platform.pickOpenPath).toHaveBeenCalledWith('Add an image projector (mmproj GGUF)', [{ name: 'Image projector', extensions: ['gguf'] }]);
    expect(spy).toHaveBeenCalledWith('Qwen3.5-4B-Q4_K_M.gguf', '/models/mmproj-Qwen3.5-4B-F16.gguf');
    TestBed.resetTestingModule();

    const other = setup({ vision: 'none' }, '/models/Qwen3.5-4B-Q4_K_M.gguf');
    await until(other.f, () => !!other.q(ROW));
    other.q(ROW)!.querySelector<HTMLButtonElement>('[data-testid=ai-add-projector]')!.click();
    await until(other.f, () => other.q('ai-models-error')?.textContent?.includes('not an image projector') ?? false);
    TestBed.resetTestingModule();

    const cancelled = setup({ vision: 'none' }, null);
    await until(cancelled.f, () => !!cancelled.q(ROW));
    const add = vi.spyOn(cancelled.backend, 'addProjector');
    cancelled.q(ROW)!.querySelector<HTMLButtonElement>('[data-testid=ai-add-projector]')!.click();
    await settle(cancelled.f);
    expect(add).not.toHaveBeenCalled();
  });

  it('shows a projector that cannot be used', async () => {
    const env = setup();
    const models = await env.backend.models();
    vi.spyOn(env.backend, 'models').mockResolvedValue([{ ...models[0]!, vision: false, projector: { ...models[0]!.projector!, problem: 'this projector belongs to another model generation' } }]);
    await env.f.componentInstance.load();
    env.f.detectChanges();
    expect(env.q(ROW)!.textContent).toContain('this projector belongs to another model generation');
  });
});

describe('AiSetupWizard image support', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('says which catalog models read images', async () => {
    const backend = new MockAiBackend({ stepMs: 0, mode: 'setup' });
    TestBed.configureTestingModule({
      providers: [
        { provide: AiBackend, useValue: backend },
        { provide: Logger, useClass: MemoryLogger },
        { provide: COMMAND_CONTEXT, useValue: new FakeContext() },
        { provide: DIALOG_REF, useValue: { close: vi.fn() } },
      ],
    });
    const f = TestBed.createComponent(AiSetupWizard);
    f.detectChanges();
    const el = f.nativeElement as HTMLElement;
    await until(f, () => !!el.querySelector('[data-testid="ai-model-qwen3.5-9b-q4km"]'));
    expect(el.querySelector('[data-testid="ai-model-qwen3.5-9b-q4km"] [data-testid=ai-model-vision]')!.textContent).toBe('Text + images');
    expect(el.querySelector('[data-testid="ai-model-llama3.2-3b-q4km"] [data-testid=ai-model-vision]')!.textContent).toBe('Text only');
    expect(el.querySelector('[data-testid="ai-model-qwen3.6-35b-a3b-q4km"]')!.textContent).toContain('needs 32 GB of RAM');
  });
});
