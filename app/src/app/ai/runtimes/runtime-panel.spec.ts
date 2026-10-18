import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it } from 'vitest';
import { Logger } from '../../core/logger';
import { MockAiBackend, MockAiOptions } from '../../core/mock/mock-ai-backend';
import { DialogService } from '../../ui/dialog.service';
import { MemoryLogger, settle } from '../../../testing/helpers';
import { AiBackend } from '../ai-backend';
import { AiRuntimeStore } from './ai-runtime.store';
import { AiRuntimePanel } from './runtime-panel';

function setup(options: MockAiOptions = {}) {
  const backend = new MockAiBackend({ stepMs: 0, ...options });
  TestBed.configureTestingModule({
    providers: [
      { provide: AiBackend, useValue: backend },
      { provide: Logger, useClass: MemoryLogger },
    ],
  });
  const dialogs = TestBed.inject(DialogService);
  const fixture = TestBed.createComponent(AiRuntimePanel);
  fixture.detectChanges();
  const el = fixture.nativeElement as HTMLElement;
  const q = (id: string) => el.querySelector<HTMLElement>(`[data-testid="${id}"]`);
  const button = (label: string) =>
    [...el.querySelectorAll<HTMLButtonElement>('button')].find(
      (b) => b.getAttribute('aria-label') === label || b.textContent!.trim() === label,
    );
  return { backend, dialogs, fixture, el, q, button, store: TestBed.inject(AiRuntimeStore) };
}

async function until(
  fixture: { detectChanges(): void },
  cond: () => boolean,
  rounds = 300,
): Promise<void> {
  for (let i = 0; i < rounds && !cond(); i++) {
    await settle();
    fixture.detectChanges();
  }
  expect(cond()).toBe(true);
}

describe('AiRuntimePanel', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('lists the builds with size, state, badges and devices', async () => {
    const { fixture, q, el } = setup();
    await until(fixture, () => !!q('ai-runtime-vulkan'));
    const vk = q('ai-runtime-vulkan')!;
    expect(vk.textContent).toContain('Active');
    expect(vk.textContent).toContain('In use');
    expect(vk.textContent).toContain('on disk');
    expect(vk.textContent).toContain('NVIDIA GeForce RTX 4060 Laptop GPU (Vulkan0');
    const cuda = q('ai-runtime-cuda-13')!;
    expect(cuda.textContent).toContain('Suggested');
    expect(cuda.textContent).toContain('Not installed');
    expect(cuda.textContent).toContain('download');
    expect(q('ai-runtime-choice')!.textContent).toContain('Automatic choice');
    expect(q('ai-runtime-outdated-b10012-cpu')!.textContent).toContain('no longer offered');
    expect(el.textContent).toContain('release v0.7.0');
    // Something works: no "download first" offer.
    expect(q('ai-runtime-suggested')).toBeNull();
  });

  it('offers the suggested download first and shows its progress until it is installed', async () => {
    const { fixture, q, store } = setup({ runtime: 'none' });
    await until(fixture, () => !!q('ai-runtime-suggested'));
    expect(q('ai-runtime-suggested')!.textContent).toContain('CUDA 13 (NVIDIA) is recommended');
    expect(q('ai-runtime-suggested')!.textContent).toContain('needs an inference runtime');
    q('ai-runtime-download-suggested')!.click();
    await until(fixture, () => !!q('ai-runtime-progress'));
    await until(fixture, () => !!q('ai-runtime-ready'));
    expect(q('ai-runtime-ready')!.textContent).toContain('CUDA 13 (NVIDIA) is installed');
    await until(fixture, () => q('ai-runtime-cuda-13')!.textContent!.includes('Active'));
    expect(store.needsDownload()).toBe(false);
    expect(q('ai-runtime-suggested')).toBeNull();
  });

  it('shows a progress bar and cancels a download', async () => {
    const { fixture, q, button } = setup({ runtime: 'none', stepMs: 5 });
    await until(fixture, () => !!button('Download CPU'));
    button('Download CPU')!.click();
    await until(fixture, () => !!fixture.nativeElement.querySelector('[role=progressbar]'));
    q('ai-runtime-cancel')!.click();
    fixture.detectChanges();
    await until(fixture, () => !!q('ai-runtime-cancelled'));
    expect(q('ai-runtime-progress')).toBeNull();
  });

  it('reports a failed download with a retry', async () => {
    const { fixture, q, button } = setup({ runtime: 'none', fail: ['runtime'] });
    await until(fixture, () => !!button('Download Vulkan (any GPU vendor)'));
    button('Download Vulkan (any GPU vendor)')!.click();
    await until(fixture, () => !!q('ai-runtime-error'));
    expect(q('ai-runtime-error')!.textContent).toContain('connection to github.com was reset');
    expect(q('ai-runtime-error')!.querySelector('button')!.textContent).toContain('Try again');
  });

  it('marks a build that finds no GPU and does not offer to use it', async () => {
    const { fixture, q, button } = setup({ runtime: 'broken' });
    await until(fixture, () => !!q('ai-runtime-vulkan'));
    expect(q('ai-runtime-vulkan')!.textContent).toContain('does not work here');
    expect(q('ai-runtime-vulkan')!.textContent).toContain('did not find a supported GPU');
    expect(button('Use this: Vulkan (any GPU vendor)')!.disabled).toBe(true);
    expect(q('ai-runtime-suggested')).not.toBeNull();
  });

  it('switches between a fixed build and automatic', async () => {
    const { fixture, q, button } = setup();
    await until(fixture, () => !!button('Use this: Vulkan (any GPU vendor)'));
    button('Use this: Vulkan (any GPU vendor)')!.click();
    await until(fixture, () => q('ai-runtime-choice')!.textContent!.includes('Fixed to Vulkan'));
    q('ai-runtime-auto')!.click();
    await until(fixture, () => q('ai-runtime-choice')!.textContent!.includes('Automatic choice'));
  });

  it('deletes a build and an outdated one only after confirmation', async () => {
    const { fixture, q, button, dialogs } = setup();
    await until(fixture, () => !!button('Delete Vulkan (any GPU vendor)'));
    button('Delete Vulkan (any GPU vendor)')!.click();
    await settle();
    expect(dialogs.stack().at(-1)!.options.title).toBe('Delete inference runtime?');
    dialogs.stack().at(-1)!.ref.close(undefined);
    await until(fixture, () => dialogs.stack().length === 0);
    expect(q('ai-runtime-vulkan')!.textContent).toContain('In use');
    button('Delete Vulkan (any GPU vendor)')!.click();
    await settle();
    dialogs.stack().at(-1)!.ref.close(true);
    await until(fixture, () => q('ai-runtime-vulkan')!.textContent!.includes('Not installed'));
    expect(q('ai-runtime-suggested')).not.toBeNull();
    button('Delete CPU b10012')!.click();
    await settle();
    dialogs.stack().at(-1)!.ref.close(true);
    await until(fixture, () => !q('ai-runtime-outdated-b10012-cpu'));
  });

  it('shows a failed listing with Retry', async () => {
    const { fixture, q, backend } = setup();
    await until(fixture, () => !!q('ai-runtime-vulkan'));
    const real = backend.runtimes.bind(backend);
    backend.runtimes = () => Promise.reject(new Error('worker gone'));
    const store = TestBed.inject(AiRuntimeStore);
    await store.load();
    fixture.detectChanges();
    expect(q('ai-runtime-load-error')!.textContent).toContain('worker gone');
    backend.runtimes = real;
    q('ai-runtime-load-error')!.querySelector('button')!.click();
    await until(fixture, () => !q('ai-runtime-load-error'));
  });
});
