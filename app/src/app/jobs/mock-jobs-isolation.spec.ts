import { tinyBackend } from '../../testing/helpers';
import { mockJobs, resetMockJobs } from './mock-jobs';

// Spec files share one window and one module graph (the Angular builder runs Vitest with `isolate: false`), so the mock
// jobs service is shared by every spec a worker runs. An import job one spec left running (its fake timers were thrown
// away when it ended) used to hold the one import slot forever: the next spec's profile save queued behind it and its
// "re-import did not finish" wait timed out, depending on which spec files a worker happened to run together.
// test-setup.ts gives every test a fresh mock jobs service; the first two tests run in order and prove it.
describe('mock jobs between tests', () => {
  afterEach(() => vi.useRealTimers());

  it('a test may end with an import job that never finishes', () => {
    mockJobs().start({ kind: 'import', title: 'Left running' }, () => new Promise(() => undefined));
    expect(mockJobs().nextId()).toBe(2);
  });

  it("the next test's re-import is not queued behind it", async () => {
    vi.useFakeTimers();
    const backend = tinyBackend();
    const save = backend.saveProfile('name = "Next"\n');
    await vi.advanceTimersByTimeAsync(0);
    const id = await save;
    let state = 'waiting';
    void mockJobs()
      .wait(id)
      .then((o) => (state = o.state));
    // The re-import takes 250 ms of (fake) time; a minute is plenty when nothing blocks the import slot.
    await vi.advanceTimersByTimeAsync(60_000);
    expect(state).toBe('done');
  });

  it('a replaced instance stops ticking and forgets who listened', async () => {
    vi.useFakeTimers();
    const old = mockJobs();
    const heard = vi.fn();
    await old.onChange(heard);
    old.start({ kind: 'import', title: 'Left running' }, () => new Promise(() => undefined));
    heard.mockClear();
    resetMockJobs();
    await vi.advanceTimersByTimeAsync(5000);
    expect(heard).not.toHaveBeenCalled();
    expect(mockJobs()).not.toBe(old);
  });
});
