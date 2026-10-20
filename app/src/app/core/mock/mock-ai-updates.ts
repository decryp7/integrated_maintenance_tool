/**
 * The AI update checks of the browser build (`app/src-tauri/src/ai/updates.rs` is the reference): one runtime
 * update of the demo machine, "Check now", Update as a download job, Later and Skip this version, and the
 * "Update / Later / Skip this version" message for the Ask policy. Plausible data, never the reference.
 */
import { AiError } from '../../ai/ai-backend';
import { AiUpdate, AiUpdateKind, AiUpdateKindStatus, AiUpdatePolicy, AiUpdatesStatus } from '../../ai/ai.types';
import { MockJobCancelled, mockJobs, mockSteps } from '../../jobs/mock-jobs';
import { mockNotifications } from '../../notifications/mock-notifications';
import { MockNetFailure, mockNetError } from './mock-ai-runtimes';

/** `?mockAiUpdates=`: `available` (an update waits to be found), `none` (default), or a policy for both kinds. */
export type MockUpdatesMode = 'none' | 'available' | 'automatic' | 'manual';

const UPDATE: Omit<AiUpdate, 'skipped' | 'installing' | 'waiting'> = {
  kind: 'runtime',
  key: 'runtime:vulkan@b11700',
  id: 'vulkan',
  name: 'Vulkan (any GPU vendor)',
  from: 'b11669',
  to: 'b11700',
  downloadBytes: 32_400_000,
};

export class MockAiUpdates {
  private readonly policy: AiUpdatePolicy;
  private found: AiUpdate[] = [];
  private pending: boolean;
  private readonly skipped = new Set<string>();
  private readonly announced = new Set<string>();
  private lastCheck: number | null = null;
  private lastError: string | null = null;
  private manualSuggested = false;
  private checking = false;
  private readonly handlers = new Set<(s: AiUpdatesStatus) => void>();

  constructor(
    mode: MockUpdatesMode,
    private readonly stepMs: number,
    private readonly net: MockNetFailure | null,
  ) {
    this.policy = mode === 'automatic' || mode === 'manual' ? mode : 'ask';
    this.pending = mode !== 'none';
    // Like the scheduler's first check after start-up (here shortly after, once the UI listens).
    if (this.pending && this.policy !== 'manual') setTimeout(() => void this.check('runtime'), stepMs * 40);
  }

  private kindStatus(kind: AiUpdateKind): AiUpdateKindStatus {
    const runtime = kind === 'runtime';
    return {
      kind,
      policy: this.policy,
      lastCheck: this.lastCheck,
      lastSuccess: runtime && this.lastError ? null : this.lastCheck,
      lastError: runtime ? this.lastError : null,
      manualSuggested: runtime && this.manualSuggested,
      nextCheck: this.policy === 'manual' ? null : (this.lastCheck ?? Date.now()) + 24 * 3600_000,
      checking: runtime && this.checking,
      updates: runtime ? this.found.map((u) => ({ ...u, skipped: this.skipped.has(u.key) })) : [],
    };
  }

  status(): AiUpdatesStatus {
    return { intervalHours: 24, kinds: [this.kindStatus('runtime'), this.kindStatus('model')] };
  }

  private emit(): void {
    const s = this.status();
    for (const h of this.handlers) h(s);
  }

  subscribe(h: (s: AiUpdatesStatus) => void): () => void {
    this.handlers.add(h);
    return () => this.handlers.delete(h);
  }

  /** A check of `kind` (only the runtime has something to find in the demo). */
  private find(kind: AiUpdateKind): void {
    this.lastCheck = Date.now();
    if (kind !== 'runtime') return;
    if (this.net) {
      const e = mockNetError(this.net, 'github.com');
      this.lastError = e.message;
      this.manualSuggested = ['tls', 'proxyAuth', 'proxy', 'httpStatus'].includes(this.net);
      return;
    }
    this.lastError = null;
    this.manualSuggested = false;
    if (!this.pending) return;
    this.found = this.found.length ? this.found : [{ ...UPDATE, skipped: false, installing: false, waiting: false }];
    for (const u of this.found) {
      if (this.skipped.has(u.key)) continue;
      if (this.policy === 'automatic') void this.install(u.key);
      else if (this.policy === 'ask' && !this.announced.has(u.key)) {
        this.announced.add(u.key);
        mockNotifications().post({
          source: 'ai',
          severity: 'info',
          scope: 'app',
          title: `AI runtime update: ${u.name}`,
          body: `llama.cpp ${u.to} replaces ${u.from} (${Math.round(u.downloadBytes / 1e6)} MB download).`,
          primary: { type: 'updateAi', key: u.key },
          secondary: { type: 'laterAiUpdate', key: u.key },
          tertiary: { type: 'skipAiUpdate', key: u.key },
          dedupeKey: `ai-update:${u.kind}:${u.id}`,
        });
      }
    }
  }

  async check(kind?: AiUpdateKind | null): Promise<AiUpdatesStatus> {
    this.checking = true;
    this.emit();
    await new Promise((r) => setTimeout(r, this.stepMs * 4));
    for (const k of kind ? [kind] : (['runtime', 'model'] as const)) this.find(k);
    this.checking = false;
    this.emit();
    return this.status();
  }

  private update(key: string): AiUpdate {
    const u = this.found.find((x) => x.key === key);
    if (!u) throw new AiError('notFound', 'that update is no longer offered; check for updates again');
    return u;
  }

  /** Runs the install as a download job; it ends with the update gone from the list. */
  async install(key: string): Promise<AiUpdatesStatus> {
    const u = this.update(key);
    if (this.found.some((x) => x.installing)) throw new AiError('busy', 'Another AI update is being installed; wait for it first.');
    u.installing = true;
    this.emit();
    void mockJobs()
      .run({ kind: 'download', title: `Updating the ${u.name} inference runtime`, appWide: true, link: { type: 'model', id: u.id } }, async (ctx) => {
        // Long enough to be listed in the Jobs pane (jobs under 2 s are not shown), like a real download.
        await mockSteps(ctx, this.stepMs * 1000, 20);
        return { value: undefined, result: { type: 'model', fileName: null } };
      })
      .then(() => {
        this.found = this.found.filter((x) => x.key !== key);
        this.pending = false;
      })
      .catch((e: unknown) => {
        u.installing = false;
        if (!(e instanceof MockJobCancelled)) this.lastError = `Updating ${u.name} failed. The installed version is kept.`;
      })
      .finally(() => this.emit());
    return this.status();
  }

  skip(key: string): AiUpdatesStatus {
    this.update(key);
    this.skipped.add(key);
    this.emit();
    return this.status();
  }

  later(key: string): AiUpdatesStatus {
    this.update(key);
    this.announced.delete(key);
    this.emit();
    return this.status();
  }
}
