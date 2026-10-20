import { Clipboard } from '@angular/cdk/clipboard';
import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { Logger } from '../../core/logger';
import { MockAiBackend, MockAiOptions } from '../../core/mock/mock-ai-backend';
import { Platform } from '../../core/platform';
import { MemoryLogger, settle } from '../../../testing/helpers';
import { AiBackend } from '../ai-backend';
import { AiErrorCode, ManualKind } from '../ai.types';
import { AiRuntimeStore } from '../runtimes/ai-runtime.store';
import { AiManualDownload } from './manual-download';

function setup(kind: ManualKind, id: string, code: AiErrorCode | null = 'tls', options: MockAiOptions = {}) {
  const backend = new MockAiBackend({ stepMs: 0, mode: 'setup', ...options });
  const platform = {
    pickOpenPath: vi.fn(async () => '/Downloads/Qwen3.5-4B-Q4_K_M.gguf' as string | null),
    pickOpenPaths: vi.fn(async () => ['/Downloads/llama-b11669-bin-win-cpu-x64.zip']),
  };
  const clipboard = { copy: vi.fn(() => true) };
  TestBed.configureTestingModule({
    providers: [
      { provide: AiBackend, useValue: backend },
      { provide: Logger, useClass: MemoryLogger },
      { provide: Platform, useValue: platform },
      { provide: Clipboard, useValue: clipboard },
    ],
  });
  const fixture = TestBed.createComponent(AiManualDownload);
  fixture.componentRef.setInput('kind', kind);
  fixture.componentRef.setInput('id', id);
  fixture.componentRef.setInput('code', code);
  const added = vi.fn();
  fixture.componentInstance.added.subscribe(added);
  fixture.detectChanges();
  const el = fixture.nativeElement as HTMLElement;
  const q = (t: string) => el.querySelector<HTMLElement>(`[data-testid="${t}"]`);
  return { backend, platform, clipboard, fixture, el, q, added };
}

async function until(fixture: { detectChanges(): void }, cond: () => boolean, rounds = 200): Promise<void> {
  for (let i = 0; i < rounds && !cond(); i++) {
    await settle();
    fixture.detectChanges();
  }
  expect(cond()).toBe(true);
}

describe('AiManualDownload', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('names the failure, the file, its SHA-256 and the steps for a model', async () => {
    const { fixture, q, el } = setup('model', 'qwen3.5-4b-q4km');
    await until(fixture, () => !!q('ai-manual-file-0'));
    expect(q('ai-manual-kind')!.textContent).toContain('intercepts HTTPS');
    expect(q('ai-manual-file-0')!.textContent).toContain('Qwen3.5-4B-Q4_K_M.gguf');
    expect(q('ai-manual-sha')!.textContent).toBe('0'.repeat(64));
    const steps = q('ai-manual-steps')!.textContent!;
    expect(steps).toContain('huggingface.co');
    expect(steps).toContain('Add downloaded file');
    expect(steps).toContain('SHA-256');
    expect(q('ai-manual-folder')!.textContent).toContain('models');
    expect(q('ai-manual-custom')).toBeNull();
    expect(el.querySelector('section')!.getAttribute('aria-labelledby')).toBe(el.querySelector('h4')!.id);
  });

  it('opens the download in the browser through the engine and copies the checksum', async () => {
    const { fixture, q, backend, clipboard, el } = setup('model', 'qwen3.5-4b-q4km', null);
    await until(fixture, () => !!q('ai-manual-open'));
    expect(q('ai-manual-kind')).toBeNull();
    expect(q('ai-manual-open')!.getAttribute('aria-label')).toContain('Qwen3.5-4B-Q4_K_M.gguf');
    q('ai-manual-open')!.click();
    await until(fixture, () => !!q('ai-manual-status'));
    expect(backend.opened).toEqual([
      'https://huggingface.co/unsloth/Qwen3.5-4B-GGUF/resolve/main/Qwen3.5-4B-Q4_K_M.gguf?download=true',
    ]);
    expect(q('ai-manual-status')!.textContent).toContain('opened in your browser');
    const copy = [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) =>
      b.getAttribute('aria-label')?.startsWith('Copy the SHA-256'),
    )!;
    copy.click();
    fixture.detectChanges();
    expect(clipboard.copy).toHaveBeenCalledWith('0'.repeat(64));
    expect(q('ai-manual-status')!.textContent).toContain('Copied');
    clipboard.copy.mockReturnValueOnce(false);
    copy.click();
    fixture.detectChanges();
    expect(q('ai-manual-status')!.textContent).toContain('not available');
    backend.openManualDownload = () => Promise.reject(new Error('no browser'));
    q('ai-manual-open')!.click();
    await until(fixture, () => !!q('ai-manual-error'));
    expect(q('ai-manual-error')!.textContent).toContain('no browser');
  });

  it('adds the downloaded model file and reports it', async () => {
    const { fixture, q, platform, added } = setup('model', 'qwen3.5-4b-q4km');
    await until(fixture, () => !!q('ai-manual-add'));
    expect(q('ai-manual-add')!.textContent).toContain('Add downloaded file');
    q('ai-manual-add')!.click();
    await until(fixture, () => added.mock.calls.length > 0);
    expect(platform.pickOpenPath).toHaveBeenCalled();
    expect(added.mock.calls[0]![0]).toMatchObject({ catalogId: 'qwen3.5-4b-q4km' });
    expect(q('ai-manual-status')!.textContent).toContain('was checked and added');
    // A cancelled dialog does nothing; a rejected file shows the engine's reason.
    platform.pickOpenPath.mockResolvedValueOnce(null);
    q('ai-manual-add')!.click();
    await settle();
    expect(added).toHaveBeenCalledTimes(1);
    platform.pickOpenPath.mockResolvedValueOnce('/Downloads/notes.txt');
    q('ai-manual-add')!.click();
    await until(fixture, () => !!q('ai-manual-error'));
    expect(q('ai-manual-error')!.textContent).toContain('.gguf');
  });

  it('lists every archive of a runtime and installs the ones picked by hand', async () => {
    const { fixture, q, platform, added, el } = setup('runtime', 'cuda-13', 'proxyAuth');
    await until(fixture, () => !!q('ai-manual-file-1'));
    expect(q('ai-manual-kind')!.textContent).toContain('sign-in');
    expect(el.querySelectorAll('[data-testid="ai-manual-open"]').length).toBe(2);
    expect(q('ai-manual-steps')!.textContent).toContain('github.com');
    expect(q('ai-manual-custom')!.textContent).toContain('Custom llama-server');
    expect(q('ai-manual-folder')!.textContent).toContain('runtimes');
    expect(q('ai-manual-add')!.textContent).toContain('Add downloaded files');
    platform.pickOpenPaths.mockResolvedValueOnce([
      '/Downloads/llama-b11669-bin-win-cuda-13-x64.zip',
      '/Downloads/cudart-llama-bin-win-cuda-13-x64.zip',
    ]);
    const store = TestBed.inject(AiRuntimeStore);
    q('ai-manual-add')!.click();
    await until(fixture, () => added.mock.calls.length > 0);
    await until(fixture, () => store.outcome()?.stage === 'ready');
    platform.pickOpenPaths.mockResolvedValueOnce([]);
    q('ai-manual-add')!.click();
    await settle();
    expect(added).toHaveBeenCalledTimes(1);
  });

  it('shows a failed lookup with Retry', async () => {
    const { fixture, q, el, backend } = setup('model', 'nope');
    await until(fixture, () => !!el.querySelector('[role=alert]'));
    expect(el.textContent).toContain('not in the model list');
    expect(q('ai-manual-file-0')).toBeNull();
    const real = backend.manualDownload.bind(backend);
    backend.manualDownload = (_k, _id) => real('model', 'qwen3.5-4b-q4km');
    el.querySelector<HTMLButtonElement>('[role=alert] button')!.click();
    await until(fixture, () => !!q('ai-manual-file-0'));
  });

  it('says so when a checksum is not known', async () => {
    const { fixture, q, backend } = setup('model', 'qwen3.5-4b-q4km');
    const real = backend.manualDownload.bind(backend);
    backend.manualDownload = async (k, id) => {
      const d = await real(k, id);
      return { ...d, files: d.files.map((f) => ({ ...f, sha256: null })) };
    };
    fixture.componentRef.setInput('id', 'qwen3.5-9b-q4km');
    await until(fixture, () => !!q('ai-manual-sha-unknown'));
    expect(q('ai-manual-sha-unknown')!.textContent).toContain('not known');
  });
});
