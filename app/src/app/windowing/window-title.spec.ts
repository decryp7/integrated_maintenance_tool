import { TestBed } from '@angular/core/testing';
import { ProjectService } from '../projects/project.service';
import { provideToolsTesting } from '../tools/mock/test-support';
import { testLogger } from '../../testing/helpers';
import { memoryHost, MockWindowingBackend } from './mock-windowing-backend';
import { DEFAULT_WINDOW_TITLE, TITLE_DEBOUNCE_MS, windowTitleFor, WindowTitleSync } from './window-title';
import { WindowingBackend } from './windowing-backend';

describe('windowTitleFor', () => {
  it('names the project, marks unsaved changes, and falls back to the app name', () => {
    expect(windowTitleFor(null, 'Untitled')).toBe(DEFAULT_WINDOW_TITLE);
    expect(windowTitleFor('/p/a.imtproj', 'A')).toBe('A — IMT');
    expect(windowTitleFor('/p/a.imtproj', 'A', true)).toBe('● A — IMT');
    expect(windowTitleFor(null, 'Untitled', true)).toBe('● Untitled — IMT');
  });
});

describe('WindowTitleSync', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  function setup() {
    const backend = new MockWindowingBackend(memoryHost());
    TestBed.configureTestingModule({ providers: [...provideToolsTesting(), { provide: WindowingBackend, useValue: backend }] });
    const set = vi.spyOn(backend, 'setWindowTitle');
    return { backend, set, sync: TestBed.inject(WindowTitleSync), projects: TestBed.inject(ProjectService) };
  }

  it('sends the title debounced and only when it changes', async () => {
    const { set, sync, projects } = setup();
    sync.start();
    sync.start();
    TestBed.tick();
    expect(set).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(TITLE_DEBOUNCE_MS);
    expect(set.mock.calls).toEqual([[DEFAULT_WINDOW_TITLE]]);

    projects.path.set('/p/a.imtproj');
    projects.name.set('A');
    TestBed.tick();
    projects.name.set('Alpha');
    TestBed.tick();
    await vi.advanceTimersByTimeAsync(TITLE_DEBOUNCE_MS);
    expect(set.mock.calls.slice(1)).toEqual([['Alpha — IMT']]);

    // Back to the title that was sent: nothing new is sent.
    projects.name.set('B');
    TestBed.tick();
    projects.name.set('Alpha');
    TestBed.tick();
    await vi.advanceTimersByTimeAsync(TITLE_DEBOUNCE_MS);
    expect(set).toHaveBeenCalledTimes(2);
    TestBed.resetTestingModule();
  });

  it('logs a failure and retries the same title later', async () => {
    const { set, sync, projects } = setup();
    set.mockRejectedValueOnce(new Error('no window'));
    sync.start();
    TestBed.tick();
    await vi.advanceTimersByTimeAsync(TITLE_DEBOUNCE_MS);
    expect(testLogger().entries.some((e) => e.message === 'Could not set the window title')).toBe(true);
    projects.name.set('Other');
    TestBed.tick();
    await vi.advanceTimersByTimeAsync(TITLE_DEBOUNCE_MS);
    expect(set).toHaveBeenCalledTimes(2);
    TestBed.resetTestingModule();
  });
});
