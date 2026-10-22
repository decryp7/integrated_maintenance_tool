import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, text, tinyDataset, typeInto } from '../../testing/helpers';
import { MAX_DETECT_EVERY_MS, TailFileState, TailStatus } from '../core/api.types';
import { MockBackend } from '../core/mock/mock-backend';
import { at } from '../../testing/must';
import { SettingsDialogService } from '../settings/settings-dialog';
import { WorkspaceStore } from '../core/workspace.store';
import { DEFAULT_LIVE_OPTIONS, LiveTail } from '../workspace/live-tail';
import { parseLimit } from './live-tail-settings';
import { STATUS_POLL_MS, TailStatusView } from './tail-status-view';
import { stateHint, stateLabel, TailStatusStore } from './tail-status.store';

const STATES: TailFileState[] = [{ kind: 'tailing' }, { kind: 'rotated' }, { kind: 'truncated' }, { kind: 'deleted' }, { kind: 'whole' }, { kind: 'error', message: 'access denied' }];

function status(): TailStatus {
  return {
    files: STATES.map((state, i) => ({
      sourceId: i,
      path: `/logs/f${i}.log`,
      name: `f${i}.log`,
      group: `/logs/f${i}.log`,
      mode: i === 1 ? 'polling' : i === 2 ? 'off' : 'events',
      offset: 2048 * i,
      size: 2048 * i + 10,
      linesPerSec: i * 60,
      lastEvent: i ? Date.now() - i * 1000 : null,
      state,
    })),
    totals: { files: 6, tailing: 1, errors: 1, bytes: 5_000_000, linesPerSec: 12.5, evicted: 42, records: 1234 },
  };
}

describe('tail status', () => {
  async function setup() {
    const backend = new MockBackend({ latency: false, dataset: tinyDataset, liveMs: 100_000, memoryStoreFeatures: true });
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend), { provide: SettingsDialogService, useValue: { open: vi.fn() } }] });
    TestBed.inject(WorkspaceStore);
    await settle();
    return { backend, live: TestBed.inject(LiveTail), store: TestBed.inject(TailStatusStore) };
  }

  afterEach(() => vi.useRealTimers());

  it('the store keeps the newest answer, the notable states, and the error of a failed refresh', async () => {
    const { backend, store } = await setup();
    vi.spyOn(backend, 'tailStatus').mockResolvedValueOnce(status());
    await store.refresh();
    expect(store.status()!.files.length).toBe(6);
    expect([...store.notable().keys()]).toEqual([1, 2, 3, 5]);
    vi.spyOn(backend, 'tailStatus').mockRejectedValueOnce('gone');
    await store.refresh();
    expect(store.error()).toBe('gone');
    expect(store.loading()).toBe(false);
    let release!: (s: TailStatus) => void;
    vi.spyOn(backend, 'tailStatus').mockImplementationOnce(() => new Promise((r) => (release = r)));
    const slow = store.refresh();
    await store.refresh();
    release({ files: [], totals: status().totals });
    await slow;
    expect(store.status()!.files.length).toBeGreaterThan(0);
  });

  it('applies per-file state changes from live reports, and fetches the status for files it does not know', async () => {
    const { backend, store } = await setup();
    const tail = vi.spyOn(backend, 'tailStatus').mockResolvedValue(status());
    // Nothing loaded yet: a change fetches the status.
    store.applyFileStates([{ sourceId: 0, state: { kind: 'deleted' } }]);
    await vi.waitFor(() => expect(store.status()).not.toBeNull());
    const calls = tail.mock.calls.length;
    store.applyFileStates([]);
    store.applyFileStates([
      { sourceId: 0, state: { kind: 'error', message: 'locked' } },
      { sourceId: 5, state: { kind: 'tailing' } },
    ]);
    expect(tail.mock.calls.length).toBe(calls);
    expect(store.notable().get(0)).toEqual({ kind: 'error', message: 'locked' });
    expect(store.notable().has(5)).toBe(false);
    expect(store.status()!.totals).toMatchObject({ tailing: 2, errors: 1 });
    // A new file: patched rows stay, and the whole status is fetched again.
    store.applyFileStates([{ sourceId: 99, state: { kind: 'tailing' } }]);
    expect(tail.mock.calls.length).toBe(calls + 1);
  });

  it('labels and explains every file state in words', () => {
    expect(STATES.map(stateLabel)).toEqual(['Tailing', 'Rotated', 'Truncated', 'Deleted', 'Read whole', 'Error']);
    expect(stateHint({ kind: 'error', message: 'locked' })).toContain('locked');
    for (const s of STATES) expect(stateHint(s).length).toBeGreaterThan(10);
  });

  it('formats sizes, ages and limits', () => {
    expect(parseLimit(' 50 ')).toBe(50);
    expect(parseLimit('')).toBeNull();
    expect(parseLimit('0')).toBeNull();
    expect(parseLimit('-3')).toBeNull();
  });

  it('the panel shows the table and totals, and a read-only summary of the options', async () => {
    const { backend, live } = await setup();
    vi.spyOn(backend, 'tailStatus').mockResolvedValue(status());
    const setLive = vi.spyOn(backend, 'setLive');
    const fixture = TestBed.createComponent(TailStatusView);
    await settle(fixture);
    expect(all(fixture, '[data-testid=tail-file]').length).toBe(6);
    const t = text(fixture);
    for (const s of ['6 files', '1 tailing', '1 with errors', '4.8 MB', '12.5 lines/s', '1,234 events', '42 evicted', 'Polling', 'Off', 'Whole file', 'Rotated', 'access denied', '240', 'just now']) expect(t).toContain(s);

    const toggle = buttonByText(fixture, 'Live tail off');
    toggle.click();
    await vi.waitFor(() => expect(live.enabled()).toBe(true));
    expect(setLive).toHaveBeenLastCalledWith(true, DEFAULT_LIVE_OPTIONS);
    await settle(fixture);
    expect(toggle.getAttribute('aria-pressed')).toBe('true');

    // The options are read-only here: a summary and a way to Settings ▸ Live tail (I2).
    expect(el(fixture, '[data-testid=tail-summary]').textContent).toContain('Watch: auto');
    expect(all(fixture, 'input, select')).toHaveLength(0);
    el<HTMLButtonElement>(fixture, '[data-testid=tail-settings]').click();
    expect(TestBed.inject(SettingsDialogService).open).toHaveBeenCalledWith('workspace');
    await live.setOptions({ ...live.options(), watch: 'polling', pollIntervalMs: 500, retention: { maxRecords: 100000, maxAgeMs: null } });
    await settle(fixture);
    expect(el(fixture, '[data-testid=tail-summary]').textContent).toBe('Watch: polling · poll every 500 ms · keep 100,000 events');
    await live.set(false);
  });

  it('polls while live tail is on, and offers Retry when the status cannot be loaded', async () => {
    const { backend, live } = await setup();
    const tail = vi.spyOn(backend, 'tailStatus').mockRejectedValueOnce('no status');
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
    const fixture = TestBed.createComponent(TailStatusView);
    await settle(fixture);
    expect(text(fixture)).toContain('The tail status could not be loaded: no status');
    buttonByText(fixture, 'Retry').click();
    await settle(fixture);
    expect(all(fixture, '[data-testid=tail-file]').length).toBe(2);
    await live.set(true);
    await settle(fixture);
    const calls = tail.mock.calls.length;
    vi.advanceTimersByTime(STATUS_POLL_MS * 2);
    await settle(fixture);
    expect(tail.mock.calls.length).toBeGreaterThanOrEqual(calls + 2);
    fixture.destroy();
    await live.set(false);
  });

  it('shows Loading first and an empty table when no files are open', async () => {
    const { backend } = await setup();
    let release!: (s: TailStatus) => void;
    vi.spyOn(backend, 'tailStatus').mockImplementationOnce(() => new Promise((r) => (release = r)));
    const fixture = TestBed.createComponent(TailStatusView);
    await settle(fixture);
    expect(text(fixture)).toContain('Loading…');
    release({ files: [], totals: { files: 0, tailing: 0, errors: 0, bytes: 0, linesPerSec: 0, evicted: 0, records: 0 } });
    await settle(fixture);
    // Files are open, but the engine lists none: it keeps no per-file state (the log store, for now).
    expect(text(fixture)).toContain('The state of each tailed file is not available yet.');
    await TestBed.inject(WorkspaceStore).clearWorkspace();
    await settle(fixture);
    expect(text(fixture)).toContain('No files are open.');
  });

  it('the default mock answers like the log store: an empty status, and live alerts cannot be muted yet', async () => {
    const backend = new MockBackend({ latency: false, dataset: tinyDataset });
    expect(await backend.tailStatus()).toEqual({ files: [], totals: { files: 0, tailing: 0, errors: 0, bytes: 0, linesPerSec: 0, evicted: 0, records: 0 } });
    const refused = { code: 'unsupported', retryable: false, message: 'live alerts are not available with the new log store yet' };
    await expect(backend.muteAlerts({ kind: 'errorSpike' }, Date.now() + 1000)).rejects.toMatchObject(refused);
    await expect(backend.unmuteAlerts({ kind: 'errorSpike' })).rejects.toMatchObject(refused);
  });
});

