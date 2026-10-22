import { ComponentFixture, TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';
import { all, buttonByText, el, settle, text, typeInto } from '../../testing/helpers';
import { CmdError } from '../core/cmd-error';
import { SettingsDialogService } from '../settings/settings-dialog';
import { DialogService } from '../ui/dialog.service';
import { MOCK_VIDEO_PATHS, MockVideoBackend } from './mock/mock-video-backend';
import { setupVideo } from './testing/video-test-support';
import { VideoAi } from './video-ai';
import { VideoView } from './video-view';
import { VideoStore } from './video.store';

async function mount(options: { open?: string[]; mock?: ConstructorParameters<typeof MockVideoBackend>[0] } = {}) {
  const t = setupVideo({ mock: options.mock });
  const ai = { run: vi.fn(async () => undefined) };
  const settings = { open: vi.fn() };
  TestBed.overrideProvider(VideoAi, { useValue: ai });
  TestBed.overrideProvider(SettingsDialogService, { useValue: settings });
  const store = TestBed.inject(VideoStore);
  if (options.open) await store.openPaths(options.open);
  const fixture: ComponentFixture<VideoView> = TestBed.createComponent(VideoView);
  fixture.detectChanges();
  await settle(fixture, 6);
  return { fixture, t, ai, settings, store, dialogs: TestBed.inject(DialogService) };
}

const testId = (f: ComponentFixture<unknown>, id: string) => el(f, `[data-testid="${id}"]`);
const click = async (f: ComponentFixture<unknown>, id: string) => {
  testId(f, id).click();
  await settle(f, 6);
};
const answer = async (f: ComponentFixture<unknown>, d: DialogService, yes: boolean) => {
  const dialog = d.stack().at(-1)!;
  dialog.ref.close(yes ? true : undefined);
  await settle(f, 6);
};

describe('VideoView empty', () => {
  it('offers to open a video and says FFmpeg is set up in Settings', async () => {
    const { fixture, t, settings } = await mount();
    expect(testId(fixture, 'video-empty').textContent).toContain('Open a video to see');
    expect(text(fixture)).toContain('Set it up once in Settings ▸ Video');
    buttonByText(fixture, 'Open a video…').click();
    await settle(fixture, 8);
    expect(testId(fixture, 'video-pick')).toBeTruthy();
    expect(t.picker.videos).toHaveLength(2);
    expect(settings.open).not.toHaveBeenCalled();
  });

  it('opens Settings ▸ Video from the empty state', async () => {
    const { fixture, settings } = await mount();
    await click(fixture, 'video-empty-settings');
    expect(settings.open).toHaveBeenCalledWith('@video');
  });

  it('says when FFmpeg is not set up, with the way to Settings', async () => {
    const { fixture, settings } = await mount({ mock: { runtime: 'windows' } });
    expect(testId(fixture, 'video-runtime-problem').textContent).toContain('FFmpeg is not set up');
    await click(fixture, 'video-runtime-setup');
    expect(settings.open).toHaveBeenCalledWith('@video');
  });

  it('shows the problem of a damaged runtime', async () => {
    const { fixture } = await mount({ mock: { runtime: 'broken' } });
    expect(testId(fixture, 'video-runtime-problem').textContent).toContain('Install it again');
  });

  it('shows a runtime check that failed, with Retry', async () => {
    const t = setupVideo();
    let fail = true;
    const real = (t.backend as MockVideoBackend).getRuntime.bind(t.backend);
    (t.backend as MockVideoBackend).getRuntime = () => (fail ? Promise.reject(new Error('no answer')) : real());
    const fixture = TestBed.createComponent(VideoView);
    fixture.detectChanges();
    await settle(fixture, 6);
    expect(text(fixture)).toContain('no answer');
    fail = false;
    await click(fixture, 'video-error-retry');
    expect(text(fixture)).not.toContain('no answer');
  });

  it('shows an open that failed, and Dismiss', async () => {
    const { fixture, t } = await mount();
    t.picker.videos = ['/x/missing-file.mp4'];
    await click(fixture, 'video-open');
    expect(text(fixture)).toContain('missing-file.mp4 was not found');
    // The file is not there: trying again would not help, so there is no Retry.
    expect(fixture.nativeElement.querySelector('[data-testid="video-error-retry"]')).toBeNull();
    buttonByText(fixture, 'Dismiss').click();
    await settle(fixture);
    expect(text(fixture)).not.toContain('was not found');
  });
});

describe('VideoView with videos', () => {
  const both = MOCK_VIDEO_PATHS;

  it('shows the player area, the frame strip and the first frame', async () => {
    const { fixture } = await mount({ open: both.slice(0, 1) });
    expect(testId(fixture, 'video-name').textContent).toBe('checkout-session.mp4');
    expect(testId(fixture, 'video-facts').textContent).toContain('1280×720 · h264 · 00:36');
    // The mock has no playable address: the frames still work.
    expect(testId(fixture, 'video-unplayable')).toBeTruthy();
    expect(all(fixture, '[role="option"]')).toHaveLength(6);
    expect(all(fixture, '[role="option"]')[1]!.getAttribute('aria-label')).toBe('Frame 2 at 00:04');
    expect(testId(fixture, 'video-frame-name').textContent).toBe('Frame 1 at 00:00');
    expect(all(fixture, 'debrief-frame-image img').length).toBeGreaterThan(0);
  });

  it('picks between several videos', async () => {
    const { fixture, store } = await mount({ open: both });
    const select = testId(fixture, 'video-pick') as HTMLSelectElement;
    expect(select.options).toHaveLength(2);
    typeInto(select, store.videos()[0]!.record.key, 'change');
    await settle(fixture, 4);
    expect(store.active()?.record.name).toBe('checkout-session.mp4');
    expect(all(fixture, '[role="option"]')).toHaveLength(6);
  });

  it('chooses a frame by click', async () => {
    const { fixture, store } = await mount({ open: both.slice(0, 1) });
    const options = all(fixture, '[role="option"]');
    options[2]!.click();
    await settle(fixture, 4);
    expect(store.selectedIndex()).toBe(2);
    expect(testId(fixture, 'video-frame-name').textContent).toBe('Frame 3 at 00:09');
    expect(testId(fixture, 'video-strip').getAttribute('aria-label')).toBe('Frames of the video');
  });

  it('saves a note on the frame, and the strip marks it', async () => {
    const { fixture, store } = await mount({ open: both.slice(0, 1) });
    const save = testId(fixture, 'video-note-save') as HTMLButtonElement;
    expect(save.disabled).toBe(true);
    typeInto(testId(fixture, 'video-note') as HTMLTextAreaElement, 'Spinner never ends');
    await settle(fixture);
    expect(save.disabled).toBe(false);
    save.click();
    await settle(fixture, 6);
    expect(store.active()!.record.frames[0]!.note).toBe('Spinner never ends');
    expect((testId(fixture, 'video-note') as HTMLTextAreaElement).value).toBe('Spinner never ends');
    expect((testId(fixture, 'video-note-save') as HTMLButtonElement).disabled).toBe(true);
    expect(testId(fixture, 'video-frame-0').textContent).toContain('N');
    expect(testId(fixture, 'video-frame-0').getAttribute('aria-label')).toContain('has a note');
  });

  it('does not lose an unsaved note when something else changes', async () => {
    const { fixture, store } = await mount({ open: both.slice(0, 1) });
    typeInto(testId(fixture, 'video-note') as HTMLTextAreaElement, 'draft');
    await settle(fixture);
    await store.setAlignment({ kind: 'offset', offsetMs: 1000 });
    await settle(fixture, 4);
    expect((testId(fixture, 'video-note') as HTMLTextAreaElement).value).toBe('draft');
  });

  it('describes a frame and all frames, and describes again', async () => {
    const { fixture, store } = await mount({ open: both.slice(0, 1) });
    await click(fixture, 'video-frame-describe');
    expect(testId(fixture, 'video-frame-description').textContent).toContain('shop home page');
    expect(testId(fixture, 'video-frame-describe').textContent).toContain('Describe again');
    await click(fixture, 'video-describe-all');
    expect(store.active()!.record.frames.every((f) => f.description)).toBe(true);
    expect((testId(fixture, 'video-describe-all') as HTMLButtonElement).disabled).toBe(true);
  });

  it('shows a model that cannot see inline, with the way to Settings ▸ AI', async () => {
    const { fixture, settings } = await mount({ open: both.slice(0, 1), mock: { vision: false } });
    await click(fixture, 'video-frame-describe');
    const err = el(fixture, 'debrief-frame-detail [role="alert"]');
    expect(err.textContent).toContain('reads text only');
    expect(err.textContent).toContain('Settings ▸ AI');
    await click(fixture, 'video-error-ai');
    await vi.waitFor(() => expect(settings.open).toHaveBeenCalledWith('ai'));
  });

  it('explains the frame with AI and finds related logs through the image use cases', async () => {
    const { fixture, ai, store } = await mount({ open: both.slice(0, 1) });
    store.selectFrame(2);
    await settle(fixture);
    await click(fixture, 'video-frame-explain');
    await click(fixture, 'video-frame-related');
    const key = store.active()!.record.key;
    expect(ai.run).toHaveBeenNthCalledWith(1, 'explain', { key, index: 2 });
    expect(ai.run).toHaveBeenNthCalledWith(2, 'related', { key, index: 2 });
  });

  it('adds a frame at the typed time and removes the selected frame', async () => {
    const { fixture, store } = await mount({ open: both.slice(0, 1) });
    expect((testId(fixture, 'video-add-at') as HTMLInputElement).value).toBe('0');
    typeInto(testId(fixture, 'video-add-at') as HTMLInputElement, 'abc');
    await settle(fixture);
    expect((testId(fixture, 'video-add-frame') as HTMLButtonElement).disabled).toBe(true);
    typeInto(testId(fixture, 'video-add-at') as HTMLInputElement, '20');
    await settle(fixture);
    await click(fixture, 'video-add-frame');
    expect(store.active()!.record.frames).toHaveLength(7);
    expect(testId(fixture, 'video-frame-name').textContent).toBe('Frame 5 at 00:20');
    await click(fixture, 'video-frame-remove');
    expect(store.active()!.record.frames).toHaveLength(6);
  });

  it('shows the alignment with its evidence and changes it', async () => {
    const { fixture, store } = await mount({ open: both.slice(0, 1) });
    expect(testId(fixture, 'video-align-start').textContent).toContain('14 Oct 10:40:00.000');
    expect(testId(fixture, 'video-align-by').textContent).toContain("the video's own creation time");
    expect(testId(fixture, 'video-align-confidence').textContent).toBe('medium');
    expect(testId(fixture, 'video-align-evidence').textContent).toContain('The video says it was created at');
    expect(testId(fixture, 'video-align-user').textContent).toContain('Automatic');
    expect((testId(fixture, 'video-align-auto') as HTMLButtonElement).disabled).toBe(true);

    // A start the user sets.
    typeInto(testId(fixture, 'video-align-input') as HTMLInputElement, '2026-10-14T12:00:00');
    await settle(fixture);
    await click(fixture, 'video-align-set');
    expect(store.active()!.record.userAlignment.kind).toBe('start');
    expect(testId(fixture, 'video-align-by').textContent).toContain('set by you');
    expect(testId(fixture, 'video-align-confidence').textContent).toBe('high');
    expect(testId(fixture, 'video-align-user').textContent).toContain('Start set to');

    // An offset.
    typeInto(testId(fixture, 'video-align-offset') as HTMLInputElement, '-2,5');
    await settle(fixture);
    await click(fixture, 'video-align-shift');
    expect(store.active()!.record.userAlignment).toEqual({ kind: 'offset', offsetMs: -2500 });
    expect(testId(fixture, 'video-align-evidence').textContent).toContain('-2.5 s');
    expect((testId(fixture, 'video-align-offset') as HTMLInputElement).value).toBe('');

    // Back to Auto.
    await click(fixture, 'video-align-auto');
    expect(store.active()!.record.userAlignment.kind).toBe('auto');
  });

  it('reads the clock from the chosen frame', async () => {
    const { fixture, store } = await mount({ open: both.slice(0, 1) });
    store.selectFrame(2);
    await settle(fixture);
    await click(fixture, 'video-align-clock');
    expect(testId(fixture, 'video-align-by').textContent).toContain('the clock on the screen');
    expect(testId(fixture, 'video-align-evidence').textContent).toContain('The clock in the frame at 00:09 read 10:42:04');
  });

  it('shows an alignment the engine refused', async () => {
    const { fixture } = await mount({ open: both.slice(0, 1) });
    typeInto(testId(fixture, 'video-align-offset') as HTMLInputElement, '9999999');
    await settle(fixture);
    await click(fixture, 'video-align-shift');
    expect(el(fixture, 'debrief-alignment-panel [role="alert"]').textContent).toContain('more than a week');
    buttonByText(fixture, 'Dismiss').click();
    await settle(fixture);
  });

  it('shows the storage, the selection and its notes', async () => {
    const { fixture } = await mount({ open: both.slice(0, 1) });
    expect(testId(fixture, 'video-frames-count').textContent).toBe('This video keeps 6 of 120 frames; the project keeps 6 of 600.');
    expect(testId(fixture, 'video-selection').textContent).toBe('Frames where the picture changed (threshold 0.1), at most 30.');
    expect(testId(fixture, 'video-selection-note').textContent).toContain('threshold was raised from 0.1 to 0.15');
    expect(el(fixture, '[role="progressbar"]').getAttribute('aria-valuenow')).toBe('6');
  });

  it('includes the video after confirming with its size, and drops the copy without asking', async () => {
    const { fixture, store, dialogs } = await mount({ open: both.slice(0, 1) });
    const box = testId(fixture, 'video-include') as HTMLInputElement;
    expect(box.checked).toBe(false);
    box.click();
    await settle(fixture, 4);
    expect(dialogs.stack()).toHaveLength(1);
    expect(testId(fixture, 'video-include').closest('debrief-video-storage')).toBeTruthy();
    expect(dialogs.stack()[0]!.options.inputs?.['message']).toMatch(/MB\)/);
    await answer(fixture, dialogs, false);
    expect(store.active()!.record.included).toBe(false);
    expect(box.checked).toBe(false);
    box.click();
    await settle(fixture, 4);
    await answer(fixture, dialogs, true);
    expect(store.active()!.record.included).toBe(true);
    expect(box.checked).toBe(true);
    box.click();
    await settle(fixture, 6);
    expect(dialogs.stack()).toHaveLength(0);
    expect(store.active()!.record.included).toBe(false);
  });

  it('removes a video only after the question names what goes', async () => {
    const { fixture, store, dialogs } = await mount({ open: both });
    store.setActive(store.videos()[0]!.record.key);
    await settle(fixture);
    await store.saveNote(1, 'a note');
    await click(fixture, 'video-remove');
    const message = String(dialogs.stack()[0]!.options.inputs?.['message']);
    expect(message).toContain('6 frames, 1 notes and 0 descriptions');
    expect(message).toContain('original video file is not touched');
    await answer(fixture, dialogs, false);
    expect(store.videos()).toHaveLength(2);
    await click(fixture, 'video-remove');
    await answer(fixture, dialogs, true);
    expect(store.videos()).toHaveLength(1);
    expect(testId(fixture, 'video-name').textContent).toBe('moved-recording.mov');
  });

  it('explains a video that is not where it was and locates it again', async () => {
    const { fixture, store } = await mount({ open: both });
    expect(store.active()!.media).toBe('missing');
    expect(testId(fixture, 'video-missing').textContent).toContain('not where it was');
    expect(testId(fixture, 'video-missing-path').textContent).toContain('/Volumes/OldDisk/');
    // The frames and notes still work.
    expect(all(fixture, '[role="option"]')).toHaveLength(5);
    expect((testId(fixture, 'video-include') as HTMLInputElement).disabled).toBe(true);
    await click(fixture, 'video-locate');
    expect(store.active()!.media).toBe('referenced');
    expect(el(fixture, '[data-testid="video-unplayable"]')).toBeTruthy();
  });

  it('shows a located video that the engine refuses', async () => {
    const { fixture, t } = await mount({ open: both });
    (t.backend as MockVideoBackend).open = () => Promise.reject(new CmdError({ code: 'invalid', message: 'This is another video.', retryable: false }));
    await click(fixture, 'video-locate');
    expect(text(fixture)).toContain('This is another video.');
  });

  it('shows the project as full', async () => {
    const { fixture, t, store } = await mount({ open: both.slice(0, 1) });
    const real = (t.backend as MockVideoBackend).list.bind(t.backend);
    (t.backend as MockVideoBackend).list = async () => ({ ...(await real()), maxProjectFrames: 6 });
    await store.refresh();
    await settle(fixture, 4);
    expect(testId(fixture, 'video-project-full').textContent).toContain('project is full');
  });

  it('shows the analysis in progress', async () => {
    const t = setupVideo();
    TestBed.overrideProvider(VideoAi, { useValue: { run: vi.fn() } });
    let release!: () => void;
    const real = (t.backend as MockVideoBackend).open.bind(t.backend);
    (t.backend as MockVideoBackend).open = async (r) => {
      await new Promise<void>((res) => (release = res));
      return real(r);
    };
    const fixture = TestBed.createComponent(VideoView);
    fixture.detectChanges();
    const opening = TestBed.inject(VideoStore).openPaths(both.slice(0, 1));
    await settle(fixture, 3);
    expect(testId(fixture, 'video-opening').textContent).toContain('Analysing the video');
    expect((testId(fixture, 'video-open') as HTMLButtonElement).disabled).toBe(true);
    expect(fixture.nativeElement.querySelector('[data-testid="video-empty"]')).toBeNull();
    release();
    await opening;
    await settle(fixture, 4);
    expect(fixture.nativeElement.querySelector('[data-testid="video-opening"]')).toBeNull();
  });
});
