import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { settle } from '../../testing/helpers';
import { runMockResearch } from '../core/mock/mock-ai-research';
import { MockJobCtx } from '../jobs/mock-jobs';
import { aiBrowsingProviders, AiBrowsingBanner } from './ai-browsing-banner';
import { parseAiBrowsingRoute } from './ai-browsing-route';
import { BrowserBackend, ChatBrowserBackend } from './browser-backend';
import { BrowserEvent } from './browser.types';
import { MockBrowserBackend, MockWeb, resetMockWeb } from './mock-browser-backend';

function setup(prepare?: (web: MockWeb, backend: MockBrowserBackend) => void) {
  const web = resetMockWeb();
  const backend = new MockBrowserBackend(web);
  prepare?.(web, backend);
  TestBed.configureTestingModule({
    providers: [
      { provide: BrowserBackend, useValue: backend },
      { provide: ChatBrowserBackend, useValue: backend },
    ],
  });
  const f = TestBed.createComponent(AiBrowsingBanner);
  f.detectChanges();
  const root = f.nativeElement as HTMLElement;
  const q = (id: string) => root.querySelector<HTMLElement>(`[data-testid=${id}]`);
  const tick = async () => {
    for (let i = 0; i < 5; i++) {
      await settle();
      f.detectChanges();
    }
  };
  return { web, backend, f, root, q, tick };
}

describe('parseAiBrowsingRoute', () => {
  it('routes the banner window and nothing else', () => {
    expect(parseAiBrowsingRoute('#/ai-browsing?host=main')).toEqual({ host: 'main' });
    expect(parseAiBrowsingRoute('#/ai-browsing?host=w-2')).toEqual({ host: 'w-2' });
    expect(parseAiBrowsingRoute('#/ai-browsing?host=')).toBeNull();
    expect(parseAiBrowsingRoute('#/ai-browsing?host=a%20b')).toBeNull();
    expect(parseAiBrowsingRoute('#/popout?view=x&host=main&label=p-main-1')).toBeNull();
    expect(parseAiBrowsingRoute('')).toBeNull();
  });
});

describe('AiBrowsingBanner', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('says the AI is browsing while an agent drives, and Stop gives the browser back', async () => {
    const { web, backend, q, tick } = setup();
    await tick();
    expect(q('ai-browsing')).toBeNull();
    expect(q('ai-browsing-idle')?.textContent).toContain('The browser is yours again');
    const stop = vi.spyOn(backend, 'stop');
    web.update('c-1', { agent: true });
    web.drive({ kind: 'agent', sessionId: 'c-1' });
    await tick();
    expect(q('ai-browsing')?.textContent).toContain('AI is browsing');
    expect(q('ai-browsing')?.textContent).toContain("agent mode is driving");
    expect(q('ai-browsing')?.getAttribute('role')).toBe('status');
    q('ai-browsing-stop')!.click();
    await tick();
    expect(stop).toHaveBeenCalledWith('c-1');
    expect(web.status('c-1').agent).toBe(false);
    expect(q('ai-browsing')).toBeNull();
  });

  it('starts from the driver the backend reports and stops research with its own Stop', async () => {
    const { web, backend, q, tick } = setup((w) => (w.driver = { kind: 'research' }));
    await tick();
    expect(q('ai-browsing')?.textContent).toContain('Research is reading');
    const stop = vi.spyOn(backend, 'researchStop');
    q('ai-browsing-stop')!.click();
    await tick();
    expect(stop).toHaveBeenCalled();
    expect(web.driver).toBeNull();
    expect(q('ai-browsing')).toBeNull();
  });

  it('shows a failed Stop inline and lets Stop try again', async () => {
    const { web, backend, q, tick } = setup((w) => (w.driver = { kind: 'agent', sessionId: 'c-1' }));
    await tick();
    vi.spyOn(backend, 'stop').mockRejectedValueOnce(new Error('this chat no longer exists'));
    q('ai-browsing-stop')!.click();
    await tick();
    expect(q('ai-browsing-error')?.textContent).toContain('this chat no longer exists');
    expect(q('ai-browsing-error')?.getAttribute('role')).toBe('alert');
    expect((q('ai-browsing-stop') as HTMLButtonElement).disabled).toBe(false);
    vi.spyOn(backend, 'researchStop').mockRejectedValueOnce('no jobs');
    web.drive({ kind: 'research' });
    await tick();
    expect(q('ai-browsing-error')).toBeNull();
    q('ai-browsing-stop')!.click();
    await tick();
    expect(q('ai-browsing-error')?.textContent).toContain('no jobs');
  });

  it('an event that comes before the first answer wins over it', async () => {
    let answer: (d: null) => void = () => undefined;
    const { web, q, tick } = setup((_w, b) => {
      vi.spyOn(b, 'driving').mockReturnValue(new Promise((r) => (answer = r)));
    });
    web.drive({ kind: 'research' });
    await tick();
    answer(null);
    await tick();
    expect(q('ai-browsing')?.textContent).toContain('Research');
  });

  it('stops listening when it goes and boots with logging only', async () => {
    const { web, f, tick } = setup();
    await tick();
    expect(web.handlers.size).toBe(1);
    f.destroy();
    expect(web.handlers.size).toBe(0);
    expect(aiBrowsingProviders()).toHaveLength(3);
  });
});

describe('the mock drives like the shell', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('announces a driver only when it changes, and research lets go when it ends or is cancelled', async () => {
    const web = resetMockWeb();
    const seen: BrowserEvent[] = [];
    web.handlers.add((e) => seen.push(e));
    web.drive(null);
    expect(seen).toEqual([]);
    let cancelled = false;
    const ctx = { cancelled: () => cancelled, message: () => undefined, progress: () => undefined } as unknown as MockJobCtx;
    const run = runMockResearch(ctx, 'what does 0x80070005 mean', 1, 0);
    expect(web.driver).toEqual({ kind: 'research' });
    await run;
    expect(web.driver).toBeNull();
    expect(seen.map((e) => e.type === 'driving' && e.by)).toEqual([{ kind: 'research' }, null]);
    cancelled = true;
    await expect(runMockResearch(ctx, 'x', 1, 0)).rejects.toThrow();
    expect(web.driver).toBeNull();
    const backend = new MockBrowserBackend(web);
    web.drive({ kind: 'agent', sessionId: 'c-1' });
    await backend.setAgent('c-2', false);
    expect(web.driver).toEqual({ kind: 'agent', sessionId: 'c-1' });
    await backend.setAgent('c-1', false);
    expect(await backend.driving()).toBeNull();
    web.drive({ kind: 'agent', sessionId: 'c-1' });
    await backend.researchStop();
    expect(web.driver).not.toBeNull();
  });
});
