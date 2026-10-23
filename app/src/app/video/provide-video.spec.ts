import { TestBed } from '@angular/core/testing';
import { describe, expect, it } from 'vitest';
import { settle } from '../../testing/helpers';
import { DROPPED_FILES_HANDLERS } from '../core/dropped-files';
import { flattenContributions, DEBRIEF_COMMANDS, DEBRIEF_VIEWS, DebriefCommand, DebriefView } from '../extensions/tokens';
import { MOCK_VIDEO_PATHS } from './mock/mock-video-backend';
import { provideVideo, showOpened, VideoCommands, videoViews } from './provide-video';
import { setupVideo } from './testing/video-test-support';
import { VideoBackend } from './video-backend';
import { VideoStore } from './video.store';

const shown = (ctx: { calls: { fn: string; args: unknown[] }[] }) => ctx.calls.filter((c) => c.fn === 'showView').map((c) => c.args[0]);

describe('video view and commands', () => {
  it('declares the Video view, whose component loads', async () => {
    const [view, ...rest] = videoViews();
    expect(rest).toEqual([]);
    expect(view).toMatchObject({ id: 'video', category: 'Diagnostics', icon: 'image' });
    expect(typeof (await view!.component())).toBe('function');
  });

  it('opens videos from the File menu and shows the view', async () => {
    const t = setupVideo();
    const commands = TestBed.inject(VideoCommands);
    const byId = (id: string) => commands.commands.find((c) => c.id === id)!;
    expect(byId('video.open').category).toBe('File');
    expect(byId('video.open').title).toBe('Open video…');
    await byId('video.open').run(t.context);
    expect(shown(t.context)).toEqual(['video']);
    expect(TestBed.inject(VideoStore).videos()).toHaveLength(2);
    await byId('video.show').run(t.context);
    expect(shown(t.context)).toEqual(['video', 'video']);
  });

  it('shows the view for a failed open too, so it can say why, and for nothing chosen it stays', async () => {
    const t = setupVideo();
    const commands = TestBed.inject(VideoCommands);
    t.picker.videos = ['/x/missing-file.mp4'];
    await commands.open(t.context);
    expect(shown(t.context)).toEqual(['video']);
    t.picker.videos = [];
    t.context.calls.length = 0;
    TestBed.inject(VideoStore).dismiss('open');
    await commands.open(t.context);
    expect(shown(t.context)).toEqual([]);
    showOpened(t.context, [], false);
    expect(shown(t.context)).toEqual([]);
  });

  it('provides the backend, view, commands, drop handler and the start-up refresh', async () => {
    const t = setupVideo();
    TestBed.configureTestingModule({ providers: [provideVideo(), { provide: VideoBackend, useValue: t.backend }] });
    expect(flattenContributions<DebriefView>(TestBed.inject(DEBRIEF_VIEWS)).map((v) => v.id)).toContain('video');
    expect(flattenContributions<DebriefCommand>(TestBed.inject(DEBRIEF_COMMANDS)).map((c) => c.id)).toEqual(expect.arrayContaining(['video.open', 'video.show']));
    const [drop] = TestBed.inject(DROPPED_FILES_HANDLERS);
    const claimed = await drop!([MOCK_VIDEO_PATHS[0]!, '/x/app.log']);
    expect(claimed).toEqual([MOCK_VIDEO_PATHS[0]]);
    expect(shown(t.context)).toEqual(['video']);
    // Not videos: nothing claimed, the view is not shown.
    expect(await drop!(['/x/app.log'])).toEqual([]);
    // Claimed but not opened: the view shows the error.
    t.backend.open = () => Promise.reject(new Error('damaged'));
    await drop!([MOCK_VIDEO_PATHS[1]!]);
    expect(shown(t.context)).toEqual(['video', 'video']);
    await settle(undefined, 4);
    expect(TestBed.inject(VideoStore).errors().open?.message).toBe('damaged');
  });
});
