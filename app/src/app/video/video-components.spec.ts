import { Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { describe, expect, it } from 'vitest';
import { el, settle } from '../../testing/helpers';
import { VideoView } from '../core/generated/video';
import { FrameImage } from './frame-image';
import { MOCK_VIDEO_PATHS, MockVideoBackend } from './mock/mock-video-backend';
import { setupVideo } from './testing/video-test-support';
import { VideoBackend } from './video-backend';
import { VideoPlayer } from './video-player';
import { VideoStore } from './video.store';

@Component({
  imports: [VideoPlayer],
  template: `<imt-video-player [video]="video()" [seekMs]="seek()" (time)="times.push($event)" (locate)="located = located + 1" />`,
})
class PlayerHost {
  readonly video = signal<VideoView>(undefined as never);
  readonly seek = signal<number | null>(null);
  readonly times: number[] = [];
  located = 0;
}

async function playerHost(patch: Partial<VideoView>, configure = true) {
  const t = configure ? setupVideo() : { backend: TestBed.inject(VideoBackend) };
  const base = await (t.backend as MockVideoBackend).open({ path: MOCK_VIDEO_PATHS[0]!, options: { mode: { mode: 'changes', threshold: 0.1 }, cap: 30 }, include: false });
  const fixture = TestBed.createComponent(PlayerHost);
  fixture.componentInstance.video.set({ ...base, ...patch });
  fixture.detectChanges();
  await settle(fixture);
  return fixture;
}

describe('VideoPlayer', () => {
  it('plays the shell media address in a native player and reports its position', async () => {
    const fixture = await playerHost({ mediaUrl: 'media://localhost/abc' });
    const video = el<HTMLVideoElement>(fixture, 'video');
    expect(video.hasAttribute('controls')).toBe(true);
    expect(video.getAttribute('src')).toBe('media://localhost/abc');
    expect(video.getAttribute('aria-label')).toBe('Video checkout-session.mp4');
    Object.defineProperty(video, 'currentTime', { value: 4.2, writable: true });
    video.dispatchEvent(new Event('timeupdate'));
    expect(fixture.componentInstance.times).toEqual([4200]);
    fixture.componentInstance.seek.set(9800);
    fixture.detectChanges();
    await settle(fixture);
    expect(video.currentTime).toBe(9.8);
  });

  it('never plays an address that is not the shell media address', async () => {
    setupVideo();
    for (const mediaUrl of ['https://example.com/a.mp4', 'file:///etc/passwd', 'javascript:alert(1)', 'media://evil/abc', null]) {
      const fixture = await playerHost({ mediaUrl }, false);
      expect(fixture.nativeElement.querySelector('video')).toBeNull();
      expect(el(fixture, '[data-testid="video-unplayable"]')).toBeTruthy();
    }
  });

  it('explains a missing video and offers to locate it', async () => {
    const fixture = await playerHost({ media: 'missing', mediaUrl: 'media://localhost/abc' });
    expect(fixture.nativeElement.querySelector('video')).toBeNull();
    expect(el(fixture, '[data-testid="video-missing"]').textContent).toContain('frames, notes and descriptions still work');
    el<HTMLButtonElement>(fixture, '[data-testid="video-locate"]').click();
    expect(fixture.componentInstance.located).toBe(1);
  });

  it('copes with a missing video that has no known path', async () => {
    const t = setupVideo();
    const base = await (t.backend as MockVideoBackend).open({ path: MOCK_VIDEO_PATHS[0]!, options: { mode: { mode: 'changes', threshold: 0.1 }, cap: 30 }, include: false });
    const fixture = TestBed.createComponent(PlayerHost);
    fixture.componentInstance.video.set({ ...base, media: 'missing', record: { ...base.record, path: null } });
    fixture.detectChanges();
    expect(fixture.nativeElement.querySelector('[data-testid="video-missing-path"]')).toBeNull();
  });
});

@Component({ imports: [FrameImage], template: `<imt-frame-image [videoKey]="key" [index]="index()" [image]="'a'" alt="Frame" />` })
class ImageHost {
  key = '';
  readonly index = signal(0);
}

describe('FrameImage', () => {
  it('shows the picture from the engine and follows the frame', async () => {
    const t = setupVideo();
    const v = await (t.backend as MockVideoBackend).open({ path: MOCK_VIDEO_PATHS[0]!, options: { mode: { mode: 'changes', threshold: 0.1 }, cap: 30 }, include: false });
    await TestBed.inject(VideoStore).refresh();
    const fixture = TestBed.createComponent(ImageHost);
    fixture.componentInstance.key = v.record.key;
    fixture.detectChanges();
    await settle(fixture);
    const first = el<HTMLImageElement>(fixture, 'img').src;
    expect(first).toContain('data:image/svg+xml');
    expect(el(fixture, 'img').getAttribute('alt')).toBe('Frame');
    fixture.componentInstance.index.set(1);
    fixture.detectChanges();
    await settle(fixture);
    expect(el<HTMLImageElement>(fixture, 'img').src).not.toBe(first);
  });

  it('marks a picture that could not be loaded', async () => {
    setupVideo();
    const fixture = TestBed.createComponent(ImageHost);
    fixture.componentInstance.key = 'gone';
    fixture.detectChanges();
    await settle(fixture);
    expect(el(fixture, '.ph').textContent).toBe('!');
  });

  it('waits to be scrolled into view when the browser can tell', async () => {
    const seen: Element[] = [];
    let notify!: (e: { isIntersecting: boolean }[]) => void;
    class Observer {
      constructor(cb: (e: { isIntersecting: boolean }[]) => void) {
        notify = cb;
      }
      observe(e: Element) {
        seen.push(e);
      }
      disconnect() {}
    }
    (globalThis as unknown as { IntersectionObserver: unknown }).IntersectionObserver = Observer;
    try {
      const t = setupVideo();
      const v = await (t.backend as MockVideoBackend).open({ path: MOCK_VIDEO_PATHS[0]!, options: { mode: { mode: 'changes', threshold: 0.1 }, cap: 30 }, include: false });
      await TestBed.inject(VideoStore).refresh();
      const fixture = TestBed.createComponent(ImageHost);
      fixture.componentInstance.key = v.record.key;
      fixture.detectChanges();
      await settle(fixture);
      expect(seen).toHaveLength(1);
      expect(fixture.nativeElement.querySelector('img')).toBeNull();
      notify([{ isIntersecting: false }]);
      await settle(fixture);
      expect(fixture.nativeElement.querySelector('img')).toBeNull();
      notify([{ isIntersecting: true }]);
      await settle(fixture);
      expect(el(fixture, 'img')).toBeTruthy();
    } finally {
      delete (globalThis as unknown as Record<string, unknown>)['IntersectionObserver'];
    }
  });
});
