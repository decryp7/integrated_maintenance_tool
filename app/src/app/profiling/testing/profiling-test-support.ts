import { Provider } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { DebriefBackend } from '../../core/backend';
import { Logger } from '../../core/logger';
import { MockProfilingBackend, MockProfilingOptions } from '../../core/mock/mock-profiling';
import { MOCK_CAPTURE_PATHS } from '../../core/mock/mock-profiling-data';
import { Platform } from '../../core/platform';
import { COMMAND_CONTEXT } from '../../extensions/command-registry';
import { FakeContext } from '../../analysis/test-setup';
import { MemoryLogger, tinyBackend } from '../../../testing/helpers';
import { CapturePicker, CapturesStore } from '../captures.store';
import { ProfilingBackend } from '../profiling-backend';

/** Test doubles for the profiling specs: mock engine, recorded command context, fixed picker, no drops. */
export class FakePicker extends CapturePicker {
  paths: string[] = [...MOCK_CAPTURE_PATHS];
  override pick(): Promise<string[]> {
    return Promise.resolve(this.paths);
  }
}

export class FakePlatform {
  dropped: ((paths: string[]) => void) | null = null;
  hover: ((over: boolean) => void) | null = null;
  onFilesDropped(handler: (paths: string[]) => void, hover: (over: boolean) => void): Promise<() => void> {
    this.dropped = handler;
    this.hover = hover;
    return Promise.resolve(() => undefined);
  }
}

export interface ProfilingTesting {
  backend: ProfilingBackend;
  context: FakeContext;
  picker: FakePicker;
  platform: FakePlatform;
}

/**
 * Providers for profiling specs. `backend` defaults to the in-memory mock (no latency); pass a
 * spy or a failing backend to test error paths.
 */
export function provideProfilingTesting(extra: { backend?: ProfilingBackend; options?: MockProfilingOptions } = {}): { providers: Provider[]; doubles: ProfilingTesting } {
  const doubles: ProfilingTesting = {
    backend: extra.backend ?? new MockProfilingBackend(extra.options),
    context: new FakeContext(),
    picker: new FakePicker(),
    platform: new FakePlatform(),
  };
  return {
    doubles,
    providers: [
      { provide: DebriefBackend, useFactory: () => tinyBackend() },
      { provide: Logger, useFactory: () => new MemoryLogger() },
      { provide: ProfilingBackend, useValue: doubles.backend },
      { provide: COMMAND_CONTEXT, useValue: doubles.context },
      { provide: CapturePicker, useValue: doubles.picker },
      { provide: Platform, useValue: doubles.platform },
    ],
  };
}

/** Configures TestBed with the profiling doubles and returns them. */
export function setupProfiling(extra: { backend?: ProfilingBackend; options?: MockProfilingOptions } = {}): ProfilingTesting {
  const { providers, doubles } = provideProfilingTesting(extra);
  TestBed.configureTestingModule({ providers });
  return doubles;
}

/** Opens the demo captures (orders, regression, three heaps) through the store; the first profile becomes active. */
export async function openDemoCaptures(paths: string[] = MOCK_CAPTURE_PATHS): Promise<void> {
  await TestBed.inject(CapturesStore).openPaths(paths);
}
