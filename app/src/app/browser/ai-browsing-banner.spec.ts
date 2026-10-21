import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { settle } from '../../testing/helpers';
import { runMockResearch } from '../core/mock/mock-ai-research';
import { MockJobCtx } from '../jobs/mock-jobs';
import { AiBrowsingBanner } from './ai-browsing-banner';
import { BrowserBackend, ChatBrowserBackend } from './browser-backend';
import { BrowserStore } from './browser.store';
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
  void TestBed.inject(BrowserStore).load();
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

describe('AiBrowsingBanner', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('says the AI is browsing while an agent drives, and Stop gives the page back', async () => {
    const { web, backend, f, q, tick } = setup();
    await tick();
    expect(q('ai-browsing')).toBeNull();
    const stop = vi.spyOn(backend, 'stop');
    web.update('c-1', { agent: true });
    web.drive({ kind: 'agent', sessionId: 'c-1' });
    await tick();
    expect(q('ai-browsing')?.textContent).toContain('AI is browsing');
    expect(q('ai-browsing')?.textContent).toContain('agent mode is driving');
    expect(q('ai-browsing')?.getAttribute('role')).toBe('status');
    expect(q('ai-browsing')?.getAttribute('aria-live')).toBe('polite');
    expect(q('ai-browsing-stop')?.getAttribute('aria-label')).toBe('Stop the AI browsing');
    // The panel's progress replaces the general line.
    f.componentRef.setInput('progress', 'step 3 of 25');
    await tick();
    expect(q('ai-browsing')?.textContent).toContain('step 3 of 25');
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

  it('records where the panel places the page, like the shell', async () => {
    const web = resetMockWeb();
    const backend = new MockBrowserBackend(web);
    expect(await backend.place({ visible: true, rect: { x: 1, y: 2, width: 30, height: 40 }, scale: 1 })).toBe(true);
    expect(web.placement?.rect?.width).toBe(30);
    expect((window as unknown as { __imtMockBrowserView: { shown: boolean } }).__imtMockBrowserView.shown).toBe(true);
    expect(await backend.place({ visible: false, rect: null, scale: 1 })).toBe(false);
    expect(await backend.place({ visible: true, rect: { x: 0, y: 0, width: 0.2, height: 9 }, scale: 1 })).toBe(false);
    await expect(backend.place({ visible: true, rect: null, scale: 0 })).rejects.toThrow('out of range');
    const reveals: BrowserEvent[] = [];
    web.handlers.add((e) => e.type === 'reveal' && reveals.push(e));
    web.show('https://e.org/');
    web.show('https://e.org/b', false);
    expect(reveals).toEqual([
      { type: 'reveal', activate: true },
      { type: 'reveal', activate: false },
    ]);
  });
});
