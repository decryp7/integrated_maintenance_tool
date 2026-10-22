import { TestBed } from '@angular/core/testing';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { AiBackend } from '../ai/ai-backend';
import { AiImageStore } from '../ai/vision/ai-image.store';
import { FakeContext } from '../analysis/test-setup';
import { MockAiBackend } from '../core/mock/mock-ai-backend';
import { Logger } from '../core/logger';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { MockJobs, resetMockJobs } from '../jobs/mock-jobs';
import { MemoryLogger, settle } from '../../testing/helpers';
import { MockVideoBackend, MOCK_VIDEO_PATHS } from './mock/mock-video-backend';
import { VideoAi } from './video-ai';
import { VideoBackend } from './video-backend';

async function setup() {
  const ai = new MockAiBackend({ stepMs: 0 });
  const video = new MockVideoBackend({ registerImage: (name) => ai.imagePaste('iVBORw0KGgoAAAAN', name) });
  const context = new FakeContext();
  TestBed.configureTestingModule({
    providers: [
      { provide: AiBackend, useValue: ai },
      { provide: VideoBackend, useValue: video },
      { provide: COMMAND_CONTEXT, useValue: context },
      { provide: Logger, useClass: MemoryLogger },
    ],
  });
  const opened = await video.open({ path: MOCK_VIDEO_PATHS[0]!, options: { mode: { mode: 'changes', threshold: 0.1 }, cap: 30 }, include: false });
  return { context, key: opened.record.key, store: TestBed.inject(AiImageStore), videoAi: TestBed.inject(VideoAi) };
}

describe('VideoAi', () => {
  beforeEach(() => resetMockJobs(new MockJobs({ visibleMs: 0, attentionMs: 0, failOnce: new Set() })));
  afterEach(() => {
    TestBed.resetTestingModule();
    resetMockJobs();
  });

  it('shows the clean frame in the image view and explains it with the existing image use case', async () => {
    const { context, key, store, videoAi } = await setup();
    await videoAi.run('explain', { key, index: 2 });
    expect(context.last('showView')).toEqual(['ai-image']);
    expect(store.current()?.name).toBe('checkout-session.mp4 @ 9 s');
    await settle(undefined, 8);
    expect(store.currentExplain()?.value).not.toBeNull();
  });

  it('finds related logs for the frame', async () => {
    const { key, store, videoAi } = await setup();
    await videoAi.run('related', { key, index: 0 });
    await settle(undefined, 8);
    expect(store.currentRelated()?.value).not.toBeNull();
  });

  it('lets the failure of opening the frame reach the caller', async () => {
    const { videoAi } = await setup();
    await expect(videoAi.run('explain', { key: 'gone', index: 0 })).rejects.toMatchObject({ code: 'notFound' });
  });
});
