import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';
import { settle } from '../../testing/helpers';
import { CmdError } from '../core/cmd-error';
import { MockVideoBackend, MOCK_FFMPEG_PATH, MOCK_LOCATED_PATH, MOCK_VIDEO_PATHS } from './mock/mock-video-backend';
import { setupVideo } from './testing/video-test-support';
import { VideoAi } from './video-ai';
import { VideoPicker, VideoStore } from './video.store';

const store = () => TestBed.inject(VideoStore);

function setup(mock: ConstructorParameters<typeof MockVideoBackend>[0] = {}) {
  const t = setupVideo({ mock });
  const ai = { run: vi.fn(async () => undefined) };
  TestBed.overrideProvider(VideoAi, { useValue: ai });
  return { ...t, ai, backend: t.backend as MockVideoBackend };
}

describe('VideoStore opening', () => {
  it('opens the picked videos, shows the last and selects its first frame', async () => {
    const t = setup();
    const opened = await store().pickAndOpen();
    expect(opened.map((v) => v.record.name)).toEqual(['checkout-session.mp4', 'moved-recording.mov']);
    expect(store().videos()).toHaveLength(2);
    expect(store().active()?.record.name).toBe('moved-recording.mov');
    expect(store().selectedIndex()).toBe(0);
    expect(store().frame()?.index).toBe(0);
    expect(store().projectFrames()).toBe(11);
    expect(store().opening()).toBe(false);
    expect(t.picker.videos).toHaveLength(2);
  });

  it('collects failures, keeps the rest and retries only the failed paths', async () => {
    setup();
    const s = store();
    const first = await s.openPaths(['/x/missing-file.mp4', MOCK_VIDEO_PATHS[0]!, '/x/also-missing-file.mp4']);
    expect(first).toHaveLength(1);
    expect(s.errors().open?.message).toContain('was not found');
    expect(s.errors().open?.message).toContain('(and 1 more failed)');
    expect(s.videos()).toHaveLength(1);
    const again = await s.retry('open');
    expect(again).toHaveLength(1);
    expect(s.errors().open).toBeDefined();
    s.dismiss('open');
    expect(s.errors().open).toBeUndefined();
    expect(await s.openPaths([])).toEqual([]);
  });

  it('shows a picker failure inline', async () => {
    const t = setup();
    t.picker.fail = true;
    expect(await store().pickAndOpen()).toEqual([]);
    expect(store().errors().open?.message).toBe('the dialog failed');
  });

  it('does not show a cancelled analysis as an error', async () => {
    const t = setup();
    t.backend.open = () => Promise.reject(new CmdError({ code: 'cancelled', message: 'Cancelled.', retryable: false }));
    expect(await store().openPaths(MOCK_VIDEO_PATHS)).toEqual([]);
    expect(store().errors().open).toBeUndefined();
  });

  it('claims dropped videos by extension and signature, leaving the rest', async () => {
    setup();
    const s = store();
    expect(await s.openDropped(['/x/app.log'])).toEqual({ claimed: [], opened: [] });
    const r = await s.openDropped(['/x/app.log', '/x/not-a-video.mp4', MOCK_VIDEO_PATHS[0]!]);
    expect(r.claimed).toEqual([MOCK_VIDEO_PATHS[0]]);
    expect(r.opened).toHaveLength(1);
  });

  it('finds a moved video again, with its settings, through the picker', async () => {
    const t = setup();
    const s = store();
    await s.pickAndOpen();
    const moved = s.videos().find((v) => v.media === 'missing')!;
    expect(await s.locate('nope')).toBeNull();
    t.picker.located = null;
    expect(await s.locate(moved.record.key)).toBeNull();
    t.picker.located = MOCK_LOCATED_PATH;
    const found = await s.locate(moved.record.key);
    expect(found?.media).toBe('referenced');
    expect(s.videos()).toHaveLength(2);
  });
});

describe('VideoStore refresh and selection', () => {
  it('loads the project videos and keeps the active one', async () => {
    const t = setup();
    await t.backend.open({ path: MOCK_VIDEO_PATHS[0]!, options: { mode: { mode: 'changes', threshold: 0.1 }, cap: 30 }, include: false });
    const s = store();
    await s.refresh();
    expect(s.videos()).toHaveLength(1);
    expect(s.activeKey()).toBe(s.videos()[0]!.record.key);
    expect(s.maxProjectFrames()).toBe(600);
    await s.refresh();
    expect(s.loading()).toBe(false);
  });

  it('shows a list failure with Retry', async () => {
    const t = setup();
    let fail = true;
    const real = t.backend.list.bind(t.backend);
    t.backend.list = () => (fail ? Promise.reject(new Error('disk')) : real());
    const s = store();
    await s.refresh();
    expect(s.errors().list?.message).toBe('disk');
    fail = false;
    await s.retry('list');
    expect(s.errors().list).toBeUndefined();
  });

  it('switches videos and frames within bounds', async () => {
    setup();
    const s = store();
    await s.pickAndOpen();
    const [a, b] = s.videos();
    s.setActive(a!.record.key);
    expect(s.active()?.record.key).toBe(a!.record.key);
    s.selectFrame(3);
    expect(s.frame()?.frame.videoMs).toBe(15_500);
    s.selectFrame(99);
    expect(s.selectedIndex()).toBeNull();
    s.selectFrame(-1);
    expect(s.selectedIndex()).toBeNull();
    s.selectFrame(null);
    s.setActive('unknown');
    expect(s.active()?.record.key).toBe(a!.record.key);
    s.setActive(b!.record.key);
    expect(s.selectedIndex()).toBe(0);
  });

  it('has nothing to work on without a video', async () => {
    setup();
    const s = store();
    expect(s.active()).toBeNull();
    expect(s.frame()).toBeNull();
    expect(await s.saveNote(0, 'x')).toBeNull();
    expect(await s.addFrame(1)).toBeNull();
    expect(await s.retry('frame')).toBeUndefined();
  });
});

describe('VideoStore frames', () => {
  async function opened() {
    const t = setup();
    const s = store();
    await s.openPaths([MOCK_VIDEO_PATHS[0]!]);
    return { t, s };
  }

  it('loads each picture once', async () => {
    const { t, s } = await opened();
    const spy = vi.spyOn(t.backend, 'getFrameImage');
    const key = s.active()!.record.key;
    const [a, b] = await Promise.all([s.frameImage(key, 1), s.frameImage(key, 1)]);
    expect(a).toBe(b);
    await s.frameImage(key, 1);
    expect(spy).toHaveBeenCalledTimes(1);
    await s.frameImage(key, 2);
    expect(spy).toHaveBeenCalledTimes(2);
  });

  it('asks again after a picture failed', async () => {
    const { t, s } = await opened();
    const key = s.active()!.record.key;
    const real = t.backend.getFrameImage.bind(t.backend);
    let fail = true;
    t.backend.getFrameImage = (f) => (fail ? Promise.reject(new Error('gone')) : real(f));
    await expect(s.frameImage(key, 0)).rejects.toThrow('gone');
    await settle();
    fail = false;
    expect(await s.frameImage(key, 0)).toContain('data:image');
  });

  it('saves and removes a note', async () => {
    const { s } = await opened();
    await s.saveNote(1, 'Spinner never ends');
    expect(s.active()!.record.frames[1]!.note).toBe('Spinner never ends');
    await s.saveNote(1, '   ');
    expect(s.active()!.record.frames[1]!.note).toBeNull();
    expect(s.busy()).toBeNull();
  });

  it('shows a failed save with Retry that repeats it', async () => {
    const { t, s } = await opened();
    const real = t.backend.setNote.bind(t.backend);
    let fail = true;
    t.backend.setNote = (r) => (fail ? Promise.reject(new CmdError({ code: 'limit', message: 'Too long.', retryable: true })) : real(r));
    await s.saveNote(0, 'n');
    expect(s.errors().frame).toMatchObject({ message: 'Too long.', retryable: true });
    fail = false;
    await s.retry('frame');
    expect(s.errors().frame).toBeUndefined();
    expect(s.active()!.record.frames[0]!.note).toBe('n');
  });

  it('adds a frame and selects it, removes a frame and keeps the selection valid', async () => {
    const { s } = await opened();
    await s.addFrame(20_000.4);
    expect(s.active()!.record.frames).toHaveLength(7);
    expect(s.frame()?.frame.videoMs).toBe(20_033);
    expect(s.projectFrames()).toBe(7);
    s.selectFrame(6);
    await s.removeFrame(0);
    expect(s.active()!.record.frames).toHaveLength(6);
    expect(s.selectedIndex()).toBe(5);
    await s.addFrame(-50);
    expect(s.errors().frame).toBeUndefined();
  });

  it('describes frames', async () => {
    const { s } = await opened();
    await s.describe([]);
    expect(s.active()!.record.frames.every((f) => !!f.description)).toBe(true);
    await s.describe([0], true);
    expect(s.busy()).toBeNull();
  });

  it('shows the vision failure with the way to Settings', async () => {
    setup({ vision: false });
    const s = store();
    await s.openPaths([MOCK_VIDEO_PATHS[0]!]);
    await s.describe([]);
    expect(s.errors().frame).toMatchObject({ aiSetup: true, hint: expect.stringContaining('Settings ▸ AI') });
  });

  it('explains frames and uses the AI image use cases', async () => {
    const { t, s } = await opened();
    expect(await s.explainFrames([0, 1])).toMatchObject({ text: expect.stringContaining('2 frames') });
    await s.useWithAi('explain', 2);
    await s.useWithAi('related', 3);
    expect(t.ai.run).toHaveBeenNthCalledWith(1, 'explain', { key: s.active()!.record.key, index: 2 });
    expect(t.ai.run).toHaveBeenNthCalledWith(2, 'related', { key: s.active()!.record.key, index: 3 });
  });
});

describe('VideoStore alignment and storage', () => {
  async function opened() {
    const t = setup();
    const s = store();
    await s.openPaths([MOCK_VIDEO_PATHS[0]!, MOCK_VIDEO_PATHS[1]!]);
    s.setActive(s.videos()[0]!.record.key);
    return { t, s };
  }

  it('sets, shifts and resets the alignment, and reads a clock', async () => {
    const { s } = await opened();
    const start = Date.UTC(2026, 9, 14, 12, 0, 0);
    await s.setAlignment({ kind: 'start', startMs: start });
    expect(s.active()!.record.alignment).toMatchObject({ by: 'user', startMs: start });
    await s.setAlignment({ kind: 'auto' });
    expect(s.active()!.record.alignment.by).toBe('container');
    await s.readClock(2);
    expect(s.active()!.record.alignment.by).toBe('clock');
    expect(s.busy()).toBeNull();
    await s.setAlignment({ kind: 'offset', offsetMs: 9e9 });
    expect(s.errors().align?.retryable).toBe(false);
  });

  it('includes the video and drops the copy', async () => {
    const { s } = await opened();
    await s.include(true);
    expect(s.active()!.media).toBe('included');
    await s.include(false);
    expect(s.active()!.media).toBe('referenced');
  });

  it('refuses to include a video that is not there', async () => {
    const { s } = await opened();
    s.setActive(s.videos()[1]!.record.key);
    await s.include(true);
    expect(s.errors().storage?.message).toContain('locate it first');
  });

  it('removes a video and shows the next', async () => {
    const { s } = await opened();
    const [a, b] = s.videos();
    expect(await s.remove(a!.record.key)).toBe(true);
    expect(s.videos().map((v) => v.record.key)).toEqual([b!.record.key]);
    expect(s.active()?.record.key).toBe(b!.record.key);
    expect(s.projectFrames()).toBe(5);
    expect(await s.remove(b!.record.key)).toBe(true);
    expect(s.active()).toBeNull();
    expect(s.selectedIndex()).toBeNull();
    expect(await s.remove('gone')).toBe(false);
    expect(s.errors().storage?.message).toContain('not in the project');
  });

  it('removing another video keeps the one shown', async () => {
    const { s } = await opened();
    const [a, b] = s.videos();
    s.selectFrame(2);
    await s.remove(b!.record.key);
    expect(s.activeKey()).toBe(a!.record.key);
    expect(s.selectedIndex()).toBe(2);
  });

  it('knows when the project is full', async () => {
    const { t, s } = await opened();
    expect(s.projectFull()).toBe(false);
    const real = t.backend.list.bind(t.backend);
    t.backend.list = async () => ({ ...(await real()), maxProjectFrames: 11 });
    await s.refresh();
    expect(s.projectFull()).toBe(true);
  });
});

describe('VideoStore runtime', () => {
  it('reports a runtime that cannot run', async () => {
    setup({ runtime: 'windows' });
    const s = store();
    expect(s.runtimeMissing()).toBe(false);
    await s.loadRuntime();
    expect(s.runtimeMissing()).toBe(true);
    await s.installRuntime();
    expect(s.runtimeMissing()).toBe(false);
    expect(s.runtime()?.status.mode).toBe('managed');
  });

  it('flags a damaged installation', async () => {
    setup({ runtime: 'broken' });
    const s = store();
    await s.loadRuntime();
    expect(s.runtimeMissing()).toBe(true);
  });

  it('installs, rolls back, deletes and uses an ffmpeg of its own', async () => {
    const t = setup({ runtime: 'windows-installed' });
    const s = store();
    await s.loadRuntime();
    expect(await s.installRuntime()).toBe(true);
    expect(s.runtime()?.status.previous).not.toBeNull();
    expect(await s.rollbackRuntime()).toBe(true);
    expect(await s.rollbackRuntime()).toBe(false);
    expect(s.errors().runtime?.message).toContain('no earlier');
    await s.installRuntime();
    expect(await s.deleteRuntime(s.runtime()!.status.previous!.tag)).toBe(true);
    expect(await s.useCustomRuntime('/x/broken')).toBe(false);
    expect(s.errors().runtime?.message).toContain('self-test');
    expect(await s.chooseRuntime()).toBe(true);
    expect(s.runtime()?.status.customPath).toBe(MOCK_FFMPEG_PATH);
    t.picker.ffmpeg = null;
    expect(await s.chooseRuntime()).toBe(false);
    expect(await s.clearCustomRuntime()).toBe(true);
    expect(s.runtime()?.status.customPath).toBeNull();
    expect(s.runtimeBusy()).toBeNull();
  });

  it('retries a failed runtime check', async () => {
    const t = setup();
    let fail = true;
    const real = t.backend.getRuntime.bind(t.backend);
    t.backend.getRuntime = () => (fail ? Promise.reject(new Error('no answer')) : real());
    const s = store();
    await s.loadRuntime();
    expect(s.errors().runtime?.message).toBe('no answer');
    fail = false;
    await s.retry('runtime');
    expect(s.runtime()).not.toBeNull();
    expect(s.errors().runtime).toBeUndefined();
  });

  it('does not show a cancelled install as an error', async () => {
    const t = setup({ runtime: 'windows' });
    t.backend.installRuntime = () => Promise.reject(new CmdError({ code: 'cancelled', message: 'Cancelled.', retryable: false }));
    expect(await store().installRuntime()).toBe(false);
    expect(store().errors().runtime).toBeUndefined();
  });
});

describe('VideoPicker in a browser', () => {
  it('offers the demo files', async () => {
    TestBed.configureTestingModule({});
    const p = TestBed.inject(VideoPicker);
    expect(await p.pickVideos()).toEqual(MOCK_VIDEO_PATHS);
    expect(await p.locateVideo()).toBe(MOCK_LOCATED_PATH);
    expect(await p.pickFfmpeg()).toBe(MOCK_FFMPEG_PATH);
  });
});
