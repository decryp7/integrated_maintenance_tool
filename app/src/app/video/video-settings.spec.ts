import { ComponentFixture, TestBed } from '@angular/core/testing';
import { describe, expect, it } from 'vitest';
import { all, buttonByText, el, settle, text } from '../../testing/helpers';
import { MockVideoBackend } from './mock/mock-video-backend';
import { setupVideo } from './testing/video-test-support';
import { modeText, VideoSettings } from './video-settings';
import { VideoStore } from './video.store';

async function mount(runtime?: ConstructorParameters<typeof MockVideoBackend>[0]) {
  const t = setupVideo({ mock: runtime });
  const fixture: ComponentFixture<VideoSettings> = TestBed.createComponent(VideoSettings);
  fixture.detectChanges();
  await settle(fixture, 6);
  return { fixture, t, store: TestBed.inject(VideoStore) };
}

const id = (f: ComponentFixture<unknown>, testId: string) => el<HTMLButtonElement>(f, `[data-testid="${testId}"]`);
const click = async (f: ComponentFixture<unknown>, testId: string) => {
  id(f, testId).click();
  await settle(f, 6);
};

describe('VideoSettings on macOS with Homebrew', () => {
  it('says there is no download, recommends Homebrew and shows the ffmpeg in use', async () => {
    const { fixture } = await mount();
    expect(id(fixture, 'video-settings-mode').textContent).toBe('The ffmpeg you chose is used.');
    expect(id(fixture, 'video-settings-mac').textContent).toContain('brew install ffmpeg');
    expect(id(fixture, 'video-settings-custom').textContent).toContain('/opt/homebrew/bin/ffmpeg (FFmpeg 7.1.1)');
    expect(fixture.nativeElement.querySelector('[data-testid="video-install"]')).toBeNull();
    const [found] = all(fixture, '[data-testid="video-detected"]');
    expect(found!.textContent).toContain('Homebrew');
    expect(found!.textContent).toContain('In use');
  });

  it('stops using the ffmpeg and uses a detected one again', async () => {
    const { fixture, store } = await mount();
    await click(fixture, 'video-stop-custom');
    expect(store.runtime()?.status.mode).toBe('none');
    expect(text(fixture)).toContain('No FFmpeg is set up yet.');
    expect(text(fixture)).toContain('None chosen.');
    expect(id(fixture, 'video-stop-custom').disabled).toBe(true);
    el<HTMLButtonElement>(fixture, '[data-testid="video-detected"] button').click();
    await settle(fixture, 6);
    expect(store.runtime()?.status.customPath).toBe('/opt/homebrew/bin/ffmpeg');
    expect(text(fixture)).toContain('In use');
  });

  it('chooses an ffmpeg and shows the self-test failure of a bad one', async () => {
    const { fixture, t, store } = await mount();
    t.picker.ffmpeg = '/usr/local/bin/ffmpeg';
    await click(fixture, 'video-choose');
    expect(store.runtime()?.status.customPath).toBe('/usr/local/bin/ffmpeg');
    t.picker.ffmpeg = '/opt/broken/ffmpeg';
    await click(fixture, 'video-choose');
    expect(el(fixture, 'debrief-video-error [role="alert"]').textContent).toContain('did not pass the self-test');
    expect(el(fixture, 'debrief-video-error').textContent).toContain('Choose another ffmpeg.');
    buttonByText(fixture, 'Dismiss').click();
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('debrief-video-error')).toBeNull();
  });
});

describe('VideoSettings with a download', () => {
  it('offers the pinned build with its size, installs it and shows the version', async () => {
    const { fixture, store } = await mount({ runtime: 'windows' });
    expect(id(fixture, 'video-settings-mode').textContent).toBe('No FFmpeg is set up yet.');
    expect(id(fixture, 'video-settings-pinned').textContent).toContain('FFmpeg 8.0 (LGPL, shared) (74 MB)');
    expect(id(fixture, 'video-install').textContent?.trim()).toBe('Download FFmpeg (74 MB)');
    expect(id(fixture, 'video-rollback').disabled).toBe(true);
    await click(fixture, 'video-install');
    expect(store.runtime()?.status.mode).toBe('managed');
    expect(id(fixture, 'video-settings-active').textContent).toContain('autobuild-2026-09-30');
    expect(id(fixture, 'video-settings-active').textContent).toContain('FFmpeg 8.0.1');
    // Installed and current: nothing to install.
    expect(id(fixture, 'video-install').disabled).toBe(true);
  });

  it('updates, rolls back and deletes the old build', async () => {
    const { fixture, store } = await mount({ runtime: 'windows-installed' });
    expect(id(fixture, 'video-settings-update').textContent).toContain('An update is available');
    expect(id(fixture, 'video-install').textContent?.trim()).toBe('Update (74 MB)');
    await click(fixture, 'video-install');
    expect(id(fixture, 'video-settings-previous').textContent).toContain('autobuild-2026-08-31');
    await click(fixture, 'video-rollback');
    expect(store.runtime()?.status.active?.tag).toBe('autobuild-2026-08-31');
    await click(fixture, 'video-install');
    await click(fixture, 'video-delete-old');
    expect(store.runtime()?.status.previous).toBeNull();
    expect(fixture.nativeElement.querySelector('[data-testid="video-settings-previous"]')).toBeNull();
  });

  it('shows a problem prominently and offers to install again', async () => {
    const { fixture } = await mount({ runtime: 'broken' });
    expect(id(fixture, 'video-settings-problem').textContent).toContain('ffmpeg.exe is missing');
    expect(id(fixture, 'video-settings-problem').getAttribute('role')).toBe('alert');
    expect(id(fixture, 'video-install').textContent?.trim()).toBe('Install again (74 MB)');
    expect(id(fixture, 'video-install').disabled).toBe(false);
  });

  it('shows a failed check with Retry', async () => {
    const t = setupVideo();
    let fail = true;
    const real = (t.backend as MockVideoBackend).getRuntime.bind(t.backend);
    (t.backend as MockVideoBackend).getRuntime = () => (fail ? Promise.reject(new Error('offline')) : real());
    const fixture = TestBed.createComponent(VideoSettings);
    fixture.detectChanges();
    await settle(fixture, 6);
    expect(text(fixture)).toContain('offline');
    expect(text(fixture)).not.toContain('Checking FFmpeg');
    fail = false;
    await click(fixture, 'video-error-retry');
    expect(id(fixture, 'video-settings-mode')).toBeTruthy();
  });

  it('says so on a computer Debrief has no build for', async () => {
    const t = setupVideo();
    const real = (t.backend as MockVideoBackend).getRuntime.bind(t.backend);
    (t.backend as MockVideoBackend).getRuntime = async () => ({ ...(await real()), platform: 'linux-riscv64' });
    const fixture = TestBed.createComponent(VideoSettings);
    fixture.detectChanges();
    await settle(fixture, 6);
    expect(id(fixture, 'video-settings-nodownload').textContent).toContain('no FFmpeg build to download');
    expect(fixture.nativeElement.querySelector('[data-testid="video-settings-mac"]')).toBeNull();
  });

  it('says what each mode means', () => {
    expect(modeText('managed')).toBe("Debrief's downloaded FFmpeg is used.");
    expect(modeText('custom')).toBe('The ffmpeg you chose is used.');
    expect(modeText('none')).toBe('No FFmpeg is set up yet.');
  });
});
