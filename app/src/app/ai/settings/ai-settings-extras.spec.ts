import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { Logger } from '../../core/logger';
import { MockAiBackend, MockAiOptions } from '../../core/mock/mock-ai-backend';
import { MemoryLogger, settle } from '../../../testing/helpers';
import { AiBackend } from '../ai-backend';
import { AiLauncher } from '../ai-launcher';
import { AiStatusStore } from '../ai-status.store';
import { AiSettingsExtras } from './ai-settings-extras';

function setup(options: MockAiOptions = {}) {
  const backend = new MockAiBackend({ stepMs: 0, ...options });
  const launcher = { openSetup: vi.fn(), openModels: vi.fn() };
  TestBed.configureTestingModule({
    providers: [
      { provide: AiBackend, useValue: backend },
      { provide: Logger, useClass: MemoryLogger },
      { provide: AiLauncher, useValue: launcher },
    ],
  });
  const fixture = TestBed.createComponent(AiSettingsExtras);
  fixture.detectChanges();
  const el = fixture.nativeElement as HTMLElement;
  const q = (id: string) => el.querySelector<HTMLElement>(`[data-testid="${id}"]`);
  const button = (text: string) => [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent!.trim() === text);
  const flush = async () => {
    await settle();
    fixture.detectChanges();
  };
  return { backend, launcher, fixture, el, q, button, flush };
}

describe('AiSettingsExtras', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('offers set-up and models, and shows the engine status', async () => {
    const { q, launcher, flush } = setup();
    await flush();
    q('ai-settings-setup')!.click();
    q('ai-settings-models')!.click();
    expect(launcher.openSetup).toHaveBeenCalled();
    expect(launcher.openModels).toHaveBeenCalled();
    expect(q('ai-settings-status')!.textContent).toContain('Model: Qwen3.5-4B-Q4_K_M.gguf · Vulkan · 3.2 GB in memory');
    expect(q('ai-settings-server')!.textContent).toContain(
      'llama-server b0-mock · 8192 tokens of context · reads images · image marker <__media_mock__>',
    );
    const store = TestBed.inject(AiStatusStore);
    const server = store.status()!.server!;
    store.setStatus({ ...store.status()!, server: { ...server, build: null, context: null, vision: false, mediaMarker: null } });
    await flush();
    expect(q('ai-settings-server')!.textContent).toContain('llama-server unknown build · text only · default image marker');
    store.setStatus({ ...store.status()!, server: { ...server, vision: null } });
    await flush();
    expect(q('ai-settings-server')!.textContent).not.toMatch(/text only|reads images/);
    store.setStatus({ ...store.status()!, server: null });
    await flush();
    expect(q('ai-settings-server')).toBeNull();
    store.setStatus({ ...store.status()!, state: 'noModel' });
    await flush();
    expect(q('ai-settings-status')!.textContent).toContain('No model is set up yet');
    store.setStatus({ ...store.status()!, state: 'failed', model: null, backend: null, rssBytes: 0, reason: 'load failed' });
    await flush();
    expect(q('ai-settings-status')!.textContent).toContain('Model: none · load failed');
    store.setStatus(null as never);
    await flush();
    expect(q('ai-settings-unavailable')).not.toBeNull();
  });

  it('explains when AI is unavailable', async () => {
    const { q, flush } = setup({ mode: 'off' });
    await flush();
    expect(q('ai-settings-unavailable')!.textContent).toContain('not installed');
    expect(q('ai-settings-setup')).toBeNull();
  });
});
