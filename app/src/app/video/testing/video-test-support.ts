import { TestBed } from '@angular/core/testing';
import { FakeContext } from '../../analysis/test-setup';
import { Logger } from '../../core/logger';
import { COMMAND_CONTEXT } from '../../extensions/command-registry';
import { MemoryLogger, provideTinyBackend } from '../../../testing/helpers';
import { MockVideoBackend, MOCK_FFMPEG_PATH, MOCK_LOCATED_PATH, MOCK_VIDEO_PATHS, MockVideoOptions } from '../mock/mock-video-backend';
import { VideoBackend } from '../video-backend';
import { VideoPicker } from '../video.store';

/** A picker whose answers the test sets. */
export class FakeVideoPicker extends VideoPicker {
  videos: string[] = [...MOCK_VIDEO_PATHS];
  located: string | null = MOCK_LOCATED_PATH;
  ffmpeg: string | null = MOCK_FFMPEG_PATH;
  fail = false;
  override pickVideos(): Promise<string[]> {
    return this.fail ? Promise.reject(new Error('the dialog failed')) : Promise.resolve(this.videos);
  }
  override locateVideo(): Promise<string | null> {
    return Promise.resolve(this.located);
  }
  override pickFfmpeg(): Promise<string | null> {
    return Promise.resolve(this.ffmpeg);
  }
}

export interface VideoTesting {
  backend: VideoBackend;
  context: FakeContext;
  picker: FakeVideoPicker;
}

/** Providers for the video specs: the mock engine, a recorded command context and fixed pickers. */
export function setupVideo(extra: { backend?: VideoBackend; mock?: MockVideoOptions } = {}): VideoTesting {
  const doubles: VideoTesting = {
    backend: extra.backend ?? new MockVideoBackend(extra.mock),
    context: new FakeContext(),
    picker: new FakeVideoPicker(),
  };
  TestBed.configureTestingModule({
    providers: [
      ...provideTinyBackend(),
      { provide: Logger, useFactory: () => new MemoryLogger() },
      { provide: VideoBackend, useValue: doubles.backend },
      { provide: COMMAND_CONTEXT, useValue: doubles.context },
      { provide: VideoPicker, useValue: doubles.picker },
    ],
  });
  return doubles;
}
