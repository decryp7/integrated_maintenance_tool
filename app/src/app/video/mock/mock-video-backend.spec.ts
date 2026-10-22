import { describe, expect, it } from 'vitest';
import { CmdError } from '../../core/cmd-error';
import { MAX_VIDEO_FRAMES, MOCK_FFMPEG_PATH, MOCK_LOCATED_PATH, MOCK_VIDEO_PATHS, MockVideoBackend, mockKey, videoOptionsFromSearch } from './mock-video-backend';

const options = { mode: { mode: 'changes' as const, threshold: 0.1 }, cap: 30 };
const open = (b: MockVideoBackend, path = MOCK_VIDEO_PATHS[0]!, include = false) => b.open({ path, options, include });

describe('MockVideoBackend runtime', () => {
  it('starts with a Homebrew ffmpeg in use and detected', async () => {
    const r = await new MockVideoBackend().getRuntime();
    expect(r.status).toMatchObject({ mode: 'custom', customPath: MOCK_FFMPEG_PATH, customVersion: '7.1.1' });
    expect(r.canDownload).toBe(false);
    expect(r.detected).toEqual([{ path: MOCK_FFMPEG_PATH, origin: 'Homebrew' }]);
    await expect(new MockVideoBackend().installRuntime()).rejects.toMatchObject({ code: 'noPlatform' });
  });

  it('installs, updates, rolls back and deletes a downloaded build', async () => {
    const b = new MockVideoBackend({ runtime: 'windows' });
    expect((await b.getRuntime()).status).toMatchObject({ mode: 'none', pinnedTag: 'autobuild-2026-09-30' });
    await expect(open(b)).rejects.toMatchObject({ code: 'noRuntime' });
    const installed = await b.installRuntime();
    expect(installed.status).toMatchObject({ mode: 'managed', active: { tag: 'autobuild-2026-09-30' } });
    await expect(b.rollbackRuntime()).rejects.toMatchObject({ code: 'notFound' });

    const old = new MockVideoBackend({ runtime: 'windows-installed' });
    expect((await old.getRuntime()).status.updateAvailable).toBe(true);
    const updated = await old.installRuntime();
    expect(updated.status.previous?.tag).toBe('autobuild-2026-08-31');
    expect((await old.rollbackRuntime()).status.active?.tag).toBe('autobuild-2026-08-31');
    const again = await old.installRuntime();
    expect((await old.deleteRuntime(again.status.previous!.tag)).status.previous).toBeNull();
    expect((await old.deleteRuntime(again.status.active!.tag)).status).toMatchObject({ active: null, mode: 'none' });
    await expect(old.deleteRuntime('nope')).rejects.toBeInstanceOf(CmdError);
  });

  it('tests a chosen ffmpeg before using it', async () => {
    const b = new MockVideoBackend({ runtime: 'windows' });
    await expect(b.setCustomRuntime('  ')).rejects.toMatchObject({ code: 'invalid' });
    await expect(b.setCustomRuntime('/tmp/broken/ffmpeg')).rejects.toMatchObject({ code: 'selfTestFailed' });
    expect((await b.setCustomRuntime('/usr/local/bin/ffmpeg')).status).toMatchObject({ mode: 'custom', customPath: '/usr/local/bin/ffmpeg' });
    expect((await b.clearCustomRuntime()).status).toMatchObject({ mode: 'none', customPath: null });
    const managed = new MockVideoBackend({ runtime: 'windows-installed' });
    await managed.setCustomRuntime('/x/ffmpeg');
    expect((await managed.clearCustomRuntime()).status.mode).toBe('managed');
  });

  it('reports a damaged installation', async () => {
    expect((await new MockVideoBackend({ runtime: 'broken' }).getRuntime()).status.problem).toContain('Install it again');
  });

  it('reads URL switches', () => {
    expect(videoOptionsFromSearch('?mockVideoRuntime=windows&mockVideoVision=off')).toEqual({ runtime: 'windows', vision: false });
    expect(videoOptionsFromSearch('?mockVideoRuntime=other')).toEqual({});
  });
});

describe('MockVideoBackend videos', () => {
  it('sniffs, opens, lists and removes', async () => {
    const b = new MockVideoBackend();
    expect(await b.sniff([MOCK_VIDEO_PATHS[0]!, '/x/app.log', '/x/not-a-video.mp4', '/x/clip.MKV'])).toEqual([MOCK_VIDEO_PATHS[0], '/x/clip.MKV']);
    const v = await open(b);
    expect(v.record.frames).toHaveLength(6);
    expect(v.media).toBe('referenced');
    expect(v.record.key).toBe(mockKey('checkout-session.mp4'));
    expect(v.record.alignment.by).toBe('container');
    // Opening again returns the same video with its work.
    await b.setNote({ key: v.record.key, index: 0, note: 'kept' });
    expect((await open(b)).record.frames[0]!.note).toBe('kept');
    const moved = await open(b, MOCK_VIDEO_PATHS[1]);
    expect(moved.media).toBe('missing');
    expect(moved.record.frames).toHaveLength(5);
    const list = await b.list();
    expect(list.videos).toHaveLength(2);
    expect(list.projectFrames).toBe(11);
    await b.remove(v.record.key);
    expect((await b.list()).projectFrames).toBe(5);
    await expect(b.remove(v.record.key)).rejects.toMatchObject({ code: 'notFound' });
    await expect(b.getFrameImage({ key: v.record.key, index: 0 })).rejects.toMatchObject({ code: 'notFound' });
  });

  it('refuses what the engine refuses', async () => {
    const b = new MockVideoBackend();
    await expect(open(b, '/x/missing-file.mp4')).rejects.toMatchObject({ code: 'notFound' });
    await expect(open(b, '/x/readme.txt')).rejects.toMatchObject({ code: 'unsupported' });
  });

  it('finds a moved video again and includes it', async () => {
    const b = new MockVideoBackend();
    const moved = await open(b, MOCK_VIDEO_PATHS[1]);
    await expect(b.include({ key: moved.record.key, include: true })).rejects.toMatchObject({ code: 'notFound' });
    expect((await open(b, MOCK_LOCATED_PATH, true)).media).toBe('included');
    expect((await b.include({ key: moved.record.key, include: false })).media).toBe('referenced');
    expect((await b.include({ key: moved.record.key, include: true })).record.included).toBe(true);
  });

  it('stops at the project cap', async () => {
    const b = new MockVideoBackend();
    const v = await open(b);
    for (let i = 0; i < 114; i++) await b.addFrame({ key: v.record.key, videoMs: 100 + i * 250 });
    await expect(b.addFrame({ key: v.record.key, videoMs: 35_000 })).rejects.toMatchObject({ code: 'limit' });
    expect((await b.list()).videos[0]!.record.frames).toHaveLength(MAX_VIDEO_FRAMES);
    // A second video does not fit in what is left of the project.
    for (const name of ['a', 'b', 'c', 'd']) {
      const w = await open(b, `/x/${name}.mp4`);
      for (let i = 0; i < 114; i++) await b.addFrame({ key: w.record.key, videoMs: 100 + i * 250 });
    }
    await expect(open(b, '/x/e.mp4')).rejects.toMatchObject({ code: 'limit' });
  });
});

describe('MockVideoBackend frames', () => {
  it('serves pictures, adds and removes frames', async () => {
    const b = new MockVideoBackend();
    const v = await open(b);
    const key = v.record.key;
    expect((await b.getFrameImage({ key, index: 1 })).dataUrl).toMatch(/^data:image\/svg\+xml;base64,/);
    await expect(b.getFrameImage({ key, index: 99 })).rejects.toMatchObject({ code: 'notFound' });
    const added = await b.addFrame({ key, videoMs: 20_000 });
    expect(added.record.frames).toHaveLength(7);
    expect(added.record.frames.find((f) => f.manual)?.videoMs).toBe(20_033);
    // The same instant twice adds nothing.
    expect((await b.addFrame({ key, videoMs: 20_000 })).record.frames).toHaveLength(7);
    await expect(b.addFrame({ key, videoMs: 99_000 })).rejects.toMatchObject({ code: 'invalid' });
    await expect(b.addFrame({ key, videoMs: -1 })).rejects.toMatchObject({ code: 'invalid' });
    expect((await b.removeFrame({ key, index: 0 })).record.frames).toHaveLength(6);
    const single = await open(b, '/x/one.mp4');
    for (let i = single.record.frames.length; i > 1; i--) await b.removeFrame({ key: single.record.key, index: 0 });
    await expect(b.removeFrame({ key: single.record.key, index: 0 })).rejects.toMatchObject({ code: 'invalid' });
  });

  it('keeps notes, describes frames and explains ranges', async () => {
    const b = new MockVideoBackend();
    const key = (await open(b)).record.key;
    expect((await b.setNote({ key, index: 1, note: 'Spinner' })).record.frames[1]!.note).toBe('Spinner');
    expect((await b.setNote({ key, index: 1, note: '  ' })).record.frames[1]!.note).toBeNull();
    await expect(b.setNote({ key, index: 1, note: 'x'.repeat(2001) })).rejects.toMatchObject({ code: 'limit' });
    const described = await b.generateDescriptions({ key, indices: [], redo: false });
    expect(described.record.frames.every((f) => !!f.description)).toBe(true);
    await b.generateDescriptions({ key, indices: [0], redo: true });
    await expect(b.generateDescriptions({ key, indices: [40], redo: false })).rejects.toMatchObject({ code: 'notFound' });
    expect((await b.explainFrames({ key, indices: [0, 1] })).text).toContain('2 frames');
    await expect(b.explainFrames({ key, indices: [] })).rejects.toMatchObject({ code: 'invalid' });
    await expect(b.explainFrames({ key, indices: [0, 1, 2, 3, 4] })).rejects.toMatchObject({ code: 'invalid' });
  });

  it('refuses the model that cannot see', async () => {
    const b = new MockVideoBackend({ vision: false });
    const key = (await open(b)).record.key;
    for (const f of [() => b.generateDescriptions({ key, indices: [], redo: false }), () => b.explainFrames({ key, indices: [0] }), () => b.alignFromClock({ key, index: 0 })]) {
      await expect(f()).rejects.toMatchObject({ code: 'noVision', hint: expect.stringContaining('Settings ▸ AI') });
    }
  });

  it('opens a frame as an image of this window', async () => {
    const b = new MockVideoBackend();
    const key = (await open(b)).record.key;
    const a = await b.frameToImage({ key, index: 2 });
    const c = await b.frameToImage({ key, index: 3 });
    expect(a.name).toBe('checkout-session.mp4 @ 9 s');
    expect(c.id).toBeGreaterThan(a.id);
    const custom = new MockVideoBackend({ registerImage: async (name) => ({ ...a, name }) });
    expect((await custom.frameToImage({ key: (await open(custom)).record.key, index: 0 })).name).toBe('checkout-session.mp4 @ 0 s');
  });
});

describe('MockVideoBackend alignment', () => {
  it('decides from the evidence it has, the user first', async () => {
    const b = new MockVideoBackend();
    const v = await open(b);
    const key = v.record.key;
    expect(v.record.alignment).toMatchObject({ by: 'container', confidence: 'medium' });

    const clock = await b.alignFromClock({ key, index: 2 });
    expect(clock.record.alignment).toMatchObject({ by: 'clock', confidence: 'medium', evidence: { kind: 'clock', frameMs: 9800, text: '10:42:04' } });
    // The clock showed 10:42:04 at 9.8 s into the video.
    expect(clock.record.alignment.startMs).toBe(Date.UTC(2026, 9, 14, 10, 41, 54, 200));

    const start = Date.UTC(2026, 9, 14, 12, 0, 0);
    expect((await b.setAlignment({ key, alignment: { kind: 'start', startMs: start } })).record.alignment).toMatchObject({ by: 'user', startMs: start, confidence: 'high', evidence: { kind: 'userStart' } });
    const shifted = await b.setAlignment({ key, alignment: { kind: 'offset', offsetMs: 2500 } });
    expect(shifted.record.alignment).toMatchObject({ by: 'user', startMs: Date.UTC(2026, 9, 14, 10, 41, 56, 700), evidence: { kind: 'userOffset', offsetMs: 2500, base: { kind: 'clock' } } });
    expect((await b.setAlignment({ key, alignment: { kind: 'auto' } })).record.alignment.by).toBe('clock');
    await expect(b.setAlignment({ key, alignment: { kind: 'start', startMs: NaN } })).rejects.toMatchObject({ code: 'invalid' });
    await expect(b.setAlignment({ key, alignment: { kind: 'offset', offsetMs: 9e9 } })).rejects.toMatchObject({ code: 'invalid' });
  });

  it('falls back to the file time and to nothing', async () => {
    const b = new MockVideoBackend();
    const v = await open(b);
    const record = (b as unknown as { videos: Map<string, { creationMs: number | null; fileModifiedMs: number | null }> }).videos.get(v.record.key)!;
    record.creationMs = null;
    expect((await b.setAlignment({ key: v.record.key, alignment: { kind: 'auto' } })).record.alignment).toMatchObject({ by: 'fileTime', confidence: 'low' });
    record.fileModifiedMs = null;
    expect((await b.setAlignment({ key: v.record.key, alignment: { kind: 'auto' } })).record.alignment).toMatchObject({ by: 'none', evidence: { kind: 'nothing' } });
  });
});
