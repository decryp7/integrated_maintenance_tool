import { TestBed } from '@angular/core/testing';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Logger } from '../core/logger';
import { MemoryLogger, settle } from '../../testing/helpers';
import { AiBrowsingChip } from './ai-browsing-chip';
import { BrowserBackend, ChatBrowserBackend } from './browser-backend';
import { MockBrowserBackend, MockWeb, resetMockWeb } from './mock-browser-backend';

let web: MockWeb;
let backend: MockBrowserBackend;

async function render() {
  const f = TestBed.createComponent(AiBrowsingChip);
  f.detectChanges();
  await settle();
  f.detectChanges();
  return f;
}
const root = (f: { nativeElement: unknown }) => f.nativeElement as HTMLElement;
const q = (f: { nativeElement: unknown }, id: string) => root(f).querySelector<HTMLElement>(`[data-testid=${id}]`);

describe('AiBrowsingChip', () => {
  beforeEach(() => {
    web = resetMockWeb();
    backend = new MockBrowserBackend(web);
    TestBed.configureTestingModule({
      providers: [
        { provide: BrowserBackend, useValue: backend },
        { provide: ChatBrowserBackend, useValue: backend },
        { provide: Logger, useClass: MemoryLogger },
      ],
    });
  });
  afterEach(() => TestBed.resetTestingModule());

  it('is hidden while nobody drives the browser', async () => {
    const f = await render();
    expect(q(f, 'ai-browsing')).toBeNull();
  });

  it('appears when the AI drives, with the engine text and the host, as a polite status', async () => {
    const f = await render();
    web.drive({ kind: 'research' });
    web.report('reading', 'learn.example.com', 'Reading 3 of 5 results');
    f.detectChanges();
    const chip = q(f, 'ai-browsing')!;
    expect(chip.getAttribute('role')).toBe('status');
    expect(chip.getAttribute('aria-live')).toBe('polite');
    expect(q(f, 'ai-browsing-text')!.textContent).toBe('Reading 3 of 5 results');
    expect(q(f, 'ai-browsing-host')!.textContent).toBe('learn.example.com');
    const stop = q(f, 'ai-browsing-stop') as HTMLButtonElement;
    expect(stop.tagName).toBe('BUTTON');
    expect(stop.getAttribute('aria-label')).toBe("Stop the AI's browser");
  });

  it('names the activity when the engine sent no text, and a plain line before any report', async () => {
    const f = await render();
    web.drive({ kind: 'agent', sessionId: 'c-1' });
    f.detectChanges();
    expect(q(f, 'ai-browsing-text')!.textContent).toBe('The AI is browsing');
    web.report('searching', 'www.google.com');
    f.detectChanges();
    expect(q(f, 'ai-browsing-text')!.textContent).toBe('Searching');
    web.report('reading', 'e.org');
    f.detectChanges();
    expect(q(f, 'ai-browsing-text')!.textContent).toBe('Reading');
  });

  it('says it is waiting for the user, distinctly', async () => {
    const f = await render();
    web.drive({ kind: 'agent', sessionId: 'c-1' });
    web.report('waitingForYou', 'bank.example');
    f.detectChanges();
    expect(q(f, 'ai-browsing-text')!.textContent).toBe('Waiting for you in the browser window: bank.example');
    expect(q(f, 'ai-browsing')!.getAttribute('data-waiting')).toBe('true');
    expect(q(f, 'ai-browsing-host')).toBeNull();
  });

  it('Stop ends research (its job) and the browser, and the chip hides', async () => {
    const f = await render();
    web.drive({ kind: 'research' });
    f.detectChanges();
    const researchStop = vi.spyOn(backend, 'researchStop');
    const stopAll = vi.spyOn(backend, 'stopAll');
    q(f, 'ai-browsing-stop')!.click();
    await settle();
    f.detectChanges();
    expect(researchStop).toHaveBeenCalledTimes(1);
    expect(stopAll).toHaveBeenCalledTimes(1);
    expect(q(f, 'ai-browsing')).toBeNull();
  });

  it("Stop ends a chat's agent mode through the chat", async () => {
    const f = await render();
    await backend.setAgent('c-1', true);
    web.drive({ kind: 'agent', sessionId: 'c-1' });
    f.detectChanges();
    const stop = vi.spyOn(backend, 'stop');
    q(f, 'ai-browsing-stop')!.click();
    await settle();
    f.detectChanges();
    expect(stop).toHaveBeenCalledWith('c-1');
    expect(web.status('c-1').agent).toBe(false);
    expect(q(f, 'ai-browsing')).toBeNull();
  });

  it('shows a failed stop and lets Stop try again', async () => {
    const f = await render();
    web.drive({ kind: 'research' });
    f.detectChanges();
    vi.spyOn(backend, 'researchStop').mockResolvedValueOnce(0);
    vi.spyOn(backend, 'stopAll').mockRejectedValueOnce(new Error('engine busy'));
    q(f, 'ai-browsing-stop')!.click();
    await settle();
    f.detectChanges();
    expect(q(f, 'ai-browsing-error')!.textContent).toContain('engine busy');
    expect((q(f, 'ai-browsing-stop') as HTMLButtonElement).disabled).toBe(false);
    q(f, 'ai-browsing-stop')!.click();
    await settle();
    f.detectChanges();
    expect(q(f, 'ai-browsing')).toBeNull();
  });
});
