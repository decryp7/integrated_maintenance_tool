import { TestBed } from '@angular/core/testing';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { FakeContext, provideAnalysisTesting } from '../../analysis/test-setup';
import { TimeCursor } from '../../analysis/time-cursor';
import { MockAiBackend, MockAiOptions } from '../../core/mock/mock-ai-backend';
import { mockRelatedOutcome } from '../../core/mock/mock-ai-vision';
import { MockBackend } from '../../core/mock/mock-backend';
import { Platform } from '../../core/platform';
import { WorkspaceStore } from '../../core/workspace.store';
import { COMMAND_CONTEXT } from '../../extensions/command-registry';
import { CommandContext } from '../../extensions/tokens';
import { JobViewer } from '../../jobs/job-viewer';
import { JobsBackend } from '../../jobs/jobs-backend';
import { MockJobs, resetMockJobs } from '../../jobs/mock-jobs';
import { Sidebar } from '../../shell/sidebar';
import { all, provideFakeDock, provideTinyBackend, settle, tinyDataset } from '../../../testing/helpers';
import { AiBackend } from '../ai-backend';
import { AI_IMAGE_VIEW, AiLauncher } from '../ai-launcher';
import { AiStatusStore } from '../ai-status.store';
import { aiCommands, aiViews } from '../provide-ai';
import { AiImageStore } from './ai-image.store';

function setup(options: MockAiOptions = {}, pick: string | null = '/shots/dialog.png') {
  const backend = new MockAiBackend({ stepMs: 0, ...options });
  const ctx = new FakeContext();
  const platform = Object.assign(new Platform(), { pickOpenPath: vi.fn(async () => pick) });
  TestBed.configureTestingModule({
    providers: [...provideAnalysisTesting({ context: ctx }), { provide: AiBackend, useValue: backend }, { provide: Platform, useValue: platform }],
  });
  return { backend, ctx, platform, launcher: TestBed.inject(AiLauncher), store: TestBed.inject(AiImageStore) };
}

describe('vision wiring', () => {
  beforeEach(() => resetMockJobs(new MockJobs({ visibleMs: 0, attentionMs: 0, failOnce: new Set() })));
  afterEach(() => {
    TestBed.resetTestingModule();
    resetMockJobs();
  });

  it('opens an Explorer image or a picked one in the image view, and can find its related logs at once', async () => {
    const { launcher, ctx, store, platform } = setup();
    expect(await launcher.openImage('/logs/shots/error.png')).toBeNull();
    expect(platform.pickOpenPath).not.toHaveBeenCalled();
    expect(ctx.last('showView')).toEqual([AI_IMAGE_VIEW]);
    expect(store.current()?.name).toBe('error.png');
    expect(await launcher.openImage(null, { findRelated: true })).toBeNull();
    expect(platform.pickOpenPath).toHaveBeenCalledWith('Open an image', [expect.objectContaining({ extensions: expect.arrayContaining(['png', 'webp']) })]);
    expect(store.current()?.name).toBe('dialog.png');
    expect(store.currentRelated()?.value?.result.links.length).toBeGreaterThan(0);
    platform.pickOpenPath.mockResolvedValueOnce(null);
    expect(await launcher.openImage()).toBeNull();
    expect(await launcher.openImage('/logs/notes.txt')).toContain('PNG, JPEG');
  });

  it('registers the image view and its commands while AI is available', async () => {
    const { launcher } = setup();
    const status = TestBed.inject(AiStatusStore);
    const view = aiViews(status).find((v) => v.id === AI_IMAGE_VIEW)!;
    expect(view.enabled!()).toBe(false);
    await status.refresh();
    expect(view.enabled!()).toBe(true);
    expect((await view.component()).name).toContain('AiImagePanel');
    const open = vi.spyOn(launcher, 'openImage').mockResolvedValue(null);
    const ws = TestBed.inject(WorkspaceStore);
    const cmds = aiCommands(launcher, status, ws, TestBed.inject(TimeCursor));
    const byId = (id: string) => cmds.find((c) => c.id === id)!;
    const c = {} as CommandContext;
    expect(byId('ai.image.open').when!(c)).toBe(true);
    byId('ai.image.open').run(c);
    expect(open).toHaveBeenCalledWith();
    await ws.reload();
    await settle(undefined, 6);
    expect(byId('ai.image.related').when!(c)).toBe(true);
    byId('ai.image.related').run(c);
    expect(open).toHaveBeenLastCalledWith(null, { findRelated: true });
  });
});

describe('JobViewer related logs', () => {
  beforeEach(() => resetMockJobs(new MockJobs({ visibleMs: 0, attentionMs: 0, failOnce: new Set() })));
  afterEach(() => {
    TestBed.resetTestingModule();
    resetMockJobs();
  });

  it('shows a related-logs result in the image view', async () => {
    const ctx = new FakeContext();
    const outcome = mockRelatedOutcome('dialog.png');
    const jobs = { result: vi.fn(async () => ({ type: 'relatedLogs' as const, result: outcome })) };
    TestBed.configureTestingModule({
      providers: [
        ...provideAnalysisTesting({ context: ctx }),
        { provide: AiBackend, useValue: new MockAiBackend({ stepMs: 0 }) },
        { provide: JobsBackend, useValue: jobs },
        { provide: COMMAND_CONTEXT, useValue: ctx },
      ],
    });
    expect(await TestBed.inject(JobViewer).view({ id: 12, title: 'Find the logs related to an image', resultType: 'relatedLogs' })).toBeNull();
    expect(jobs.result).toHaveBeenCalledWith(12);
    expect(ctx.last('showView')).toEqual([AI_IMAGE_VIEW]);
    expect(TestBed.inject(AiImageStore).detached()).toMatchObject({ jobId: 12, outcome });
  });
});

describe('Explorer images', () => {
  afterEach(() => TestBed.resetTestingModule());

  async function render(ai: MockAiOptions = {}) {
    const dataset = () => {
      const d = tinyDataset();
      return { ...d, unrecognized: [...d.unrecognized, { path: '/logs/shots/error.png', name: 'error.png', reason: 'No importer' }] };
    };
    TestBed.configureTestingModule({
      providers: [
        provideTinyBackend(new MockBackend({ latency: false, dataset, pluginMarkerKinds: [] })),
        provideFakeDock(),
        { provide: AiBackend, useValue: new MockAiBackend({ stepMs: 0, ...ai }) },
      ],
    });
    const fixture = TestBed.createComponent(Sidebar);
    await TestBed.inject(AiStatusStore).refresh();
    await TestBed.inject(WorkspaceStore).reload();
    await settle(fixture, 8);
    return fixture;
  }

  it('offers "Open image" for an image file the importers did not recognise', async () => {
    const fixture = await render();
    const open = vi.spyOn(TestBed.inject(AiLauncher), 'openImage').mockResolvedValue(null);
    const buttons = all<HTMLButtonElement>(fixture, '[data-testid=open-image]');
    expect(buttons.map((b) => b.getAttribute('aria-label'))).toEqual(['Open image error.png']);
    buttons[0]!.click();
    expect(open).toHaveBeenCalledWith('/logs/shots/error.png');
    // Other files keep "Set up format…".
    expect((fixture.nativeElement as HTMLElement).textContent).toContain('Set up format…');
  });

  it('does not offer it while AI cannot be used', async () => {
    const fixture = await render({ mode: 'off' });
    expect(all(fixture, '[data-testid=open-image]')).toEqual([]);
  });
});
