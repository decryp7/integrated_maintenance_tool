import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { Logger } from '../../core/logger';
import { MockAiBackend, MockAiOptions } from '../../core/mock/mock-ai-backend';
import { MemoryLogger, settle } from '../../../testing/helpers';
import { gatedPairingClock } from '../../../testing/pitwall-clock';
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

  describe('Pitwall', () => {
    const text = (el: HTMLElement) => el.textContent!.replace(/\s+/g, ' ');

    it('says built-in when Pitwall is absent, with no Pitwall buttons', async () => {
      const { q, flush } = setup();
      await flush();
      expect(q('ai-engine-status')!.textContent).toBe('Engine: built-in');
      for (const id of ['ai-pitwall-connect', 'ai-pitwall-cancel', 'ai-pitwall-forget', 'ai-use-built-in']) expect(q(id)).toBeNull();
    });

    it('offers to connect when Pitwall runs unpaired, shows the code while waiting and ends in Pitwall', async () => {
      const gate = gatedPairingClock();
      const { q, flush, backend } = setup({ pitwall: 'running', stepMs: 1, clock: gate.clock });
      await flush();
      expect(q('ai-engine-status')!.textContent).toContain('Pitwall is running on this computer. Connect Debrief to it');
      const connect = vi.spyOn(backend, 'pitwallConnect');
      q('ai-pitwall-connect')!.click();
      await flush();
      expect(connect).toHaveBeenCalledWith(false);
      expect(q('ai-pitwall-code')!.textContent).toBe('482 913');
      expect(q('ai-pitwall-connect')).toBeNull();
      expect(q('ai-pitwall-cancel')).not.toBeNull();
      gate.approve();
      for (let i = 0; i < 50 && !q('ai-pitwall-forget'); i++) await flush();
      expect(q('ai-engine-status')!.textContent).toBe('Engine: Pitwall 1.0.4 · model qwen3-8b-q4km');
      expect(q('ai-pitwall-code')).toBeNull();
      expect(q('ai-pitwall-connect')).toBeNull();
    });

    it('cancels a waiting pairing', async () => {
      const { q, flush, backend } = setup({ pitwall: 'running', stepMs: 1, clock: gatedPairingClock().clock });
      await flush();
      q('ai-pitwall-connect')!.click();
      await flush();
      const cancel = vi.spyOn(backend, 'pitwallCancel');
      q('ai-pitwall-cancel')!.click();
      await flush();
      expect(cancel).toHaveBeenCalled();
      expect(q('ai-pitwall-code')).toBeNull();
      expect(q('ai-pitwall-connect')).not.toBeNull();
    });

    it('forgets a paired Pitwall and shows the denial as the problem', async () => {
      const { q, flush, backend } = setup({ pitwall: 'paired' });
      await flush();
      expect(q('ai-engine-status')!.textContent).toContain('Engine: Pitwall 1.0.4');
      const forget = vi.spyOn(backend, 'pitwallForget');
      q('ai-pitwall-forget')!.click();
      await flush();
      expect(forget).toHaveBeenCalled();
      expect(q('ai-pitwall-forget')).toBeNull();
      expect(q('ai-engine-status')!.textContent).toContain('Engine: built-in');
    });

    it('offers the built-in engine when Pitwall is chosen but cannot be used, and shows the problem', async () => {
      const { q, el, flush, backend } = setup({ pitwall: 'installed' });
      await flush();
      const store = TestBed.inject(AiStatusStore);
      const s = store.status()!;
      store.setStatus({
        ...s,
        engine: { ...s.engine, choice: 'pitwall', active: null, pitwall: { ...s.engine.pitwall, problem: 'Pitwall stopped.' } },
      });
      await flush();
      expect(text(el)).toContain('Engine: none');
      expect(text(el)).toContain('Pitwall is installed but not running. Open Pitwall to use it.');
      expect(text(el)).toContain('Pitwall stopped.');
      const useBuiltIn = vi.spyOn(backend, 'useBuiltInEngine');
      q('ai-use-built-in')!.click();
      await flush();
      expect(useBuiltIn).toHaveBeenCalled();
    });

    it('shows a rejected command inline', async () => {
      const { q, flush, backend } = setup({ pitwall: 'running' });
      await flush();
      vi.spyOn(backend, 'pitwallConnect').mockRejectedValue(new AiError('engine', 'Pitwall did not answer.'));
      q('ai-pitwall-connect')!.click();
      await flush();
      expect(q('ai-engine-error')!.textContent).toBe('Pitwall did not answer.');
      expect(q('ai-engine-error')!.getAttribute('role')).toBe('alert');
    });
  });
});
