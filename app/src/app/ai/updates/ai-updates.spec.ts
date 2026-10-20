import { TestBed } from '@angular/core/testing';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Logger } from '../../core/logger';
import { MockAiBackend, mockAiOptionsFromSearch } from '../../core/mock/mock-ai-backend';
import { MockAiUpdates } from '../../core/mock/mock-ai-updates';
import { resetMockJobs } from '../../jobs/mock-jobs';
import { MemoryLogger, settle } from '../../../testing/helpers';
import { AiBackend } from '../ai-backend';
import { AiUpdate, AiUpdateKindStatus, AiUpdatesStatus } from '../ai.types';
import { AiUpdatesPanel, kindTitle, lastCheckText, nextCheckText, policyLabel, updateLine } from './ai-updates-panel';
import { AiUpdatesStore } from './ai-updates.store';

const KEY = 'runtime:vulkan@b11700';

function kindStatus(p: Partial<AiUpdateKindStatus> = {}): AiUpdateKindStatus {
  return {
    kind: 'runtime',
    policy: 'ask',
    lastCheck: null,
    lastSuccess: null,
    lastError: null,
    manualSuggested: false,
    nextCheck: null,
    checking: false,
    updates: [],
    ...p,
  };
}

function update(p: Partial<AiUpdate> = {}): AiUpdate {
  return { kind: 'runtime', key: KEY, id: 'vulkan', name: 'Vulkan', from: 'b1', to: 'b2', downloadBytes: 32_000_000, skipped: false, installing: false, waiting: false, ...p };
}

describe('AI update texts', () => {
  it('names kinds, policies, checks and updates', () => {
    expect(kindTitle('runtime')).toBe('Inference runtime');
    expect(kindTitle('model')).toBe('Models');
    expect(['automatic', 'ask', 'manual'].map((p) => policyLabel(p as never))).toEqual(['Automatic', 'Ask', 'Manual']);
    expect(lastCheckText(kindStatus())).toBe('Never checked');
    expect(lastCheckText(kindStatus({ lastCheck: Date.UTC(2026, 9, 20, 12) }), 'en-GB')).toMatch(/^Checked 20 Oct 2026/);
    expect(nextCheckText(kindStatus())).toBe('Checks only when you choose Check now');
    expect(nextCheckText(kindStatus({ nextCheck: Date.UTC(2026, 9, 21, 12) }), 'en-GB')).toMatch(/^Next check 21 Oct 2026/);
    expect(updateLine(update())).toMatch(/^b1 → b2 · 3\d(\.\d)? MB$/);
  });
});

describe('MockAiUpdates', () => {
  beforeEach(() => resetMockJobs());

  it('asks once about the update it finds, installs it as a job, and forgets it', async () => {
    const m = new MockAiUpdates('available', 0, null);
    const s = await m.check('runtime');
    expect(s.kinds.map((k) => k.kind)).toEqual(['runtime', 'model']);
    expect(s.kinds[0]!.updates.map((u) => u.key)).toEqual([KEY]);
    expect(m.later(KEY).kinds[0]!.updates).toHaveLength(1);
    expect(m.skip(KEY).kinds[0]!.updates[0]!.skipped).toBe(true);
    const seen: AiUpdatesStatus[] = [];
    m.subscribe((x) => seen.push(x));
    await m.install(KEY);
    await expect(m.install(KEY)).rejects.toMatchObject({ code: 'busy' });
    await vi.waitFor(() => expect(m.status().kinds[0]!.updates).toEqual([]));
    expect(seen.length).toBeGreaterThan(1);
    expect(() => m.skip(KEY)).toThrow(/no longer offered/);
  });

  it('fails quietly behind a TLS-inspecting proxy and offers the manual download', async () => {
    const m = new MockAiUpdates('available', 0, 'tls');
    const s = await m.check();
    expect(s.kinds[0]!.lastError).toContain('Zscaler');
    expect(s.kinds[0]!.manualSuggested).toBe(true);
    expect(s.kinds[0]!.lastSuccess).toBeNull();
  });

  it('manual checks only when asked; automatic installs by itself', async () => {
    const manual = new MockAiUpdates('manual', 0, null);
    expect(manual.status().kinds[0]!.updates).toEqual([]);
    expect(manual.status().kinds[0]!.nextCheck).toBeNull();
    expect((await manual.check('runtime')).kinds[0]!.updates).toHaveLength(1);
    const auto = new MockAiUpdates('automatic', 5, null);
    await auto.check('runtime');
    expect(auto.status().kinds[0]!.updates[0]!.installing).toBe(true);
    const none = new MockAiUpdates('none', 0, null);
    expect((await none.check()).kinds[0]!.updates).toEqual([]);
  });

  it('is switched on from the address', () => {
    expect(mockAiOptionsFromSearch('?mockAiUpdates=available').updates).toBe('available');
    expect(mockAiOptionsFromSearch('?mockAiUpdates=bogus').updates).toBeUndefined();
  });
});

describe('AiUpdatesStore and AiUpdatesPanel', () => {
  afterEach(() => TestBed.resetTestingModule());

  function setup(backend: AiBackend = new MockAiBackend({ stepMs: 0, updates: 'manual' })) {
    TestBed.configureTestingModule({
      providers: [
        { provide: AiBackend, useValue: backend },
        { provide: Logger, useClass: MemoryLogger },
      ],
    });
    const fixture = TestBed.createComponent(AiUpdatesPanel);
    const el = fixture.nativeElement as HTMLElement;
    const q = (id: string) => el.querySelector<HTMLElement>(`[data-testid="${id}"]`);
    const flush = async () => {
      await settle();
      fixture.detectChanges();
      await settle();
      fixture.detectChanges();
    };
    fixture.detectChanges();
    return { fixture, el, q, flush, store: TestBed.inject(AiUpdatesStore), backend };
  }

  it('shows each kind, checks now, and updates or skips what it found', async () => {
    const { q, flush, store, el } = setup();
    await flush();
    expect(q('ai-updates-runtime')!.textContent).toContain('Never checked');
    expect(q('ai-updates-policy-runtime')!.textContent!.trim()).toBe('Manual');
    q('ai-updates-check')!.click();
    await vi.waitFor(async () => {
      await flush();
      expect(q(`ai-update-${KEY}`)).toBeTruthy();
    });
    expect(q('ai-updates-current-model')!.textContent).toContain('Up to date');
    const skip = el.querySelector<HTMLButtonElement>('[data-testid=ai-update-skip]')!;
    expect(skip.getAttribute('aria-label')).toBe('Skip this version of Vulkan (any GPU vendor) (b11700)');
    skip.click();
    await flush();
    expect(q(`ai-update-${KEY}`)!.textContent).toContain('Skipped');
    expect(store.offered()).toEqual([]);
    el.querySelector<HTMLButtonElement>('[data-testid=ai-update-apply]')!.click();
    // Update runs the install as a job; once it is done the update is no longer offered.
    await vi.waitFor(async () => {
      await flush();
      expect(q(`ai-update-${KEY}`)).toBeNull();
    });
    expect(q('ai-updates-current-runtime')!.textContent).toContain('Up to date');
  });

  it('shows a quiet failure with the manual download, and a failed request with Retry', async () => {
    const failing = {
      updatesStatus: vi.fn(async () => {
        throw { code: 'engine', message: 'not ready' };
      }),
      onUpdates: vi.fn(async () => () => undefined),
    } as unknown as AiBackend;
    const { q, flush, store } = setup(failing);
    await flush();
    expect(q('ai-updates-error')!.textContent).toContain('not ready');
    store.onStatus({
      intervalHours: 24,
      kinds: [kindStatus({ lastCheck: 1, lastError: 'Updating Vulkan failed: the secure connection could not be verified. The installed version is kept.', manualSuggested: true, updates: [update()] })],
    });
    await flush();
    expect(q('ai-updates-failed-runtime')!.textContent).toContain('installed version is kept');
    expect(q('ai-updates-manual')).toBeTruthy();
  });

  it('follows the status the engine sends', async () => {
    let push: ((s: AiUpdatesStatus) => void) | undefined;
    const backend = new MockAiBackend({ stepMs: 0 });
    vi.spyOn(backend, 'onUpdates').mockImplementation(async (h) => {
      push = h;
      return () => undefined;
    });
    const { store, flush, q } = setup(backend);
    await flush();
    push!({ intervalHours: 6, kinds: [kindStatus({ checking: true, updates: [update({ waiting: true })] })] });
    await flush();
    expect(store.checking()).toBe(true);
    expect(q('ai-updates-checking')).toBeTruthy();
    expect(q(`ai-update-${KEY}`)!.textContent).toContain('Waits for the chat to finish');
  });
});
