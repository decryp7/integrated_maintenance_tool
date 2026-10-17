import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { Logger } from '../../core/logger';
import { MockAiBackend, MockAiOptions } from '../../core/mock/mock-ai-backend';
import { MemoryLogger, settle } from '../../../testing/helpers';
import { AiBackend, AiError } from '../ai-backend';
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
    const store = TestBed.inject(AiStatusStore);
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

  it('stores and removes the Brave key without ever showing it', async () => {
    const { q, el, button, flush } = setup();
    await flush();
    expect(q('ai-key-state')!.textContent).toContain('No key is stored');
    const input = el.querySelector<HTMLInputElement>('#ai-brave-key')!;
    expect(input.type).toBe('password');
    expect(button('Save key')!.disabled).toBe(true);
    input.value = 'has space';
    input.dispatchEvent(new Event('input'));
    await flush();
    button('Save key')!.click();
    await flush();
    expect(el.querySelector('[role=alert]')!.textContent).toContain('exactly');
    input.value = 'BSA-123';
    input.dispatchEvent(new Event('input'));
    await flush();
    button('Save key')!.click();
    await flush();
    await flush();
    expect(q('ai-key-state')!.textContent).toContain('A key is stored');
    expect(input.value).toBe('');
    expect(input.placeholder).toContain('A key is stored');
    button('Remove key')!.click();
    await flush();
    await flush();
    expect(q('ai-key-state')!.textContent).toContain('No key is stored');
  });

  it('reports keychain failures', async () => {
    const { backend, button, el, fixture, flush } = setup({ web: true });
    vi.spyOn(backend, 'clearBraveKey').mockRejectedValue(new AiError('refused', 'keychain locked'));
    await flush();
    button('Remove key')!.click();
    await flush();
    expect(el.querySelector('[role=alert]')!.textContent).toContain('keychain locked');
    vi.spyOn(backend, 'braveKeySet').mockRejectedValue(new Error('x'));
    await (fixture.componentInstance as unknown as { refreshKey(): Promise<void> }).refreshKey();
    expect(fixture.componentInstance.keySet()).toBe(false);
  });

  it('explains when AI is unavailable', async () => {
    const { q, flush } = setup({ mode: 'off' });
    await flush();
    expect(q('ai-settings-unavailable')!.textContent).toContain('not installed');
    expect(q('ai-settings-setup')).toBeNull();
  });
});
