import { TestBed } from '@angular/core/testing';
import { describe, expect, it } from 'vitest';
import { AiBackend } from '../ai/ai-backend';
import { Logger } from '../core/logger';
import { MockAiBackend } from '../core/mock/mock-ai-backend';
import { MemoryLogger } from '../../testing/helpers';
import { MOCK_VIDEO_PATHS } from './mock/mock-video-backend';
import { createVideoBackend, provideVideoBackend, TauriVideoBackend, VideoBackend } from './video-backend';

describe('the browser build engine', () => {
  it('is the mock, and its frame images go to the AI mock image cache', async () => {
    const ai = new MockAiBackend({ stepMs: 0 });
    TestBed.configureTestingModule({
      providers: [...provideVideoBackend(), { provide: AiBackend, useValue: ai }, { provide: Logger, useClass: MemoryLogger }],
    });
    const backend = TestBed.inject(VideoBackend);
    expect(createVideoBackend).toBeTypeOf('function');
    const v = await backend.open({ path: MOCK_VIDEO_PATHS[0]!, options: { mode: { mode: 'changes', threshold: 0.1 }, cap: 30 }, include: false });
    const image = await backend.frameToImage({ key: v.record.key, index: 1 });
    expect(image.name).toBe('checkout-session.mp4 @ 4 s');
    // The same image is known to the AI image use cases.
    await expect(ai.imageClose(image.id)).resolves.toBe(true);
  });
});

describe('TauriVideoBackend', () => {
  it('calls one command per method with its arguments', async () => {
    const calls: [string, unknown][] = [];
    const invoke = (async (cmd: string, args?: unknown) => {
      calls.push([cmd, args]);
      return null;
    }) as never;
    const b = new TauriVideoBackend(invoke);
    const frame = { key: 'k', index: 2 };
    await b.getRuntime();
    await b.installRuntime();
    await b.rollbackRuntime();
    await b.deleteRuntime('t1');
    await b.setCustomRuntime('/opt/homebrew/bin/ffmpeg');
    await b.clearCustomRuntime();
    await b.sniff(['/a.mp4']);
    await b.list();
    await b.open({ path: '/a.mp4', options: { mode: { mode: 'changes', threshold: 0.1 }, cap: 30 }, include: false });
    await b.remove('k');
    await b.include({ key: 'k', include: true });
    await b.getFrameImage(frame);
    await b.addFrame({ key: 'k', videoMs: 5 });
    await b.removeFrame(frame);
    await b.setNote({ key: 'k', index: 2, note: 'n' });
    await b.generateDescriptions({ key: 'k', indices: [], redo: false });
    await b.explainFrames({ key: 'k', indices: [1] });
    await b.frameToImage(frame);
    await b.setAlignment({ key: 'k', alignment: { kind: 'auto' } });
    await b.alignFromClock(frame);
    expect(calls.map((c) => c[0])).toEqual([
      'video_runtime',
      'video_runtime_install',
      'video_runtime_rollback',
      'video_runtime_delete',
      'video_runtime_set_custom',
      'video_runtime_clear_custom',
      'video_sniff',
      'video_list',
      'video_open',
      'video_remove',
      'video_include',
      'video_frame_image',
      'video_add_frame',
      'video_remove_frame',
      'video_set_note',
      'video_describe',
      'video_explain',
      'video_frame_to_image',
      'video_set_alignment',
      'video_read_clock',
    ]);
    expect(calls[0]![1]).toBeUndefined();
    expect(calls[3]![1]).toEqual({ tag: 't1' });
    expect(calls[4]![1]).toEqual({ path: '/opt/homebrew/bin/ffmpeg' });
    expect(calls[6]![1]).toEqual({ paths: ['/a.mp4'] });
    expect(calls[9]![1]).toEqual({ key: 'k' });
    expect(calls[11]![1]).toEqual({ frame });
    expect(calls[14]![1]).toEqual({ request: { key: 'k', index: 2, note: 'n' } });
    expect(calls[19]![1]).toEqual({ frame });
  });
});
