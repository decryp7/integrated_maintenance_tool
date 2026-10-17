import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, text, tinyDataset, typeInto } from '../../testing/helpers';
import { MAX_DETECT_EVERY_MS, TailFileState, TailStatus } from '../core/api.types';
import { MockBackend } from '../core/mock/mock-backend';
import { WorkspaceStore } from '../core/workspace.store';
import { DEFAULT_LIVE_OPTIONS, LiveTail } from '../workspace/live-tail';
import { formatAgo, formatBytes, parseLimit, STATUS_POLL_MS, TailStatusView } from './tail-status-view';
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
    const backend = new MockBackend({ latency: false, dataset: tinyDataset, liveMs: 100_000 });
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
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
    expect(formatBytes(512)).toBe('512 B');
    expect(formatBytes(2048)).toBe('2.0 KB');
    expect(formatBytes(50 * 1024 * 1024)).toBe('50 MB');
    expect(formatBytes(3 * 1024 ** 5)).toBe('3072 TB');
    const now = 1_000_000;
    expect(formatAgo(null, now)).toBe('—');
    expect(formatAgo(now - 500, now)).toBe('just now');
    expect(formatAgo(now - 12_000, now)).toBe('12 s ago');
    expect(formatAgo(now - 180_000, now)).toBe('3 min ago');
    expect(formatAgo(now - 7_200_000, now)).toBe('2 h ago');
    expect(parseLimit(' 50 ')).toBe(50);
    expect(parseLimit('')).toBeNull();
    expect(parseLimit('0')).toBeNull();
    expect(parseLimit('-3')).toBeNull();
  });

  it('the panel shows the table and totals, and every control goes to set_live', async () => {
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

    const [watch, deleted] = all<HTMLSelectElement>(fixture, 'select');
    const [poll, maxRecords, maxAge, carry] = all<HTMLInputElement>(fixture, 'input');
    typeInto(watch, 'polling', 'change');
    await settle(fixture);
    expect(setLive).toHaveBeenLastCalledWith(true, expect.objectContaining({ watch: 'polling' }));
    typeInto(poll, '50', 'change');
    await settle(fixture);
    expect(setLive).toHaveBeenLastCalledWith(true, expect.objectContaining({ pollIntervalMs: 100 }));
    typeInto(poll, '', 'change');
    await settle(fixture);
    expect(setLive).toHaveBeenLastCalledWith(true, expect.objectContaining({ pollIntervalMs: null }));
    typeInto(maxRecords, '100000', 'change');
    await settle(fixture);
    expect(setLive).toHaveBeenLastCalledWith(true, expect.objectContaining({ retention: { maxRecords: 100000, maxAgeMs: null } }));
    typeInto(maxAge, '30', 'change');
    await settle(fixture);
    expect(setLive).toHaveBeenLastCalledWith(true, expect.objectContaining({ retention: { maxRecords: 100000, maxAgeMs: 1_800_000 } }));
    expect(el<HTMLInputElement>(fixture, 'input[placeholder="no limit"]:nth-of-type(1)')).toBeTruthy();
    typeInto(maxAge, '', 'change');
    await settle(fixture);
    expect(live.options().retention?.maxAgeMs).toBeNull();
    typeInto(deleted, 'drop', 'change');
    await settle(fixture);
    expect(setLive).toHaveBeenLastCalledWith(true, expect.objectContaining({ deletedFiles: 'drop' }));
    typeInto(carry, '500', 'change');
    await settle(fixture);
    expect(setLive).toHaveBeenLastCalledWith(true, expect.objectContaining({ carryFlushMs: 500 }));
    typeInto(carry, 'abc', 'change');
    await settle(fixture);
    expect(live.options().carryFlushMs).toBe(2000);
    // Detector interval: labelled, applied, clamped to the engine's range; invalid text shows the value again.
    const detect = el<HTMLInputElement>(fixture, '[data-testid=detect-every]');
    expect(detect.closest('label')!.textContent).toContain('Run detectors every (ms)');
    expect(detect.max).toBe(String(MAX_DETECT_EVERY_MS));
    expect(detect.value).toBe('1000');
    typeInto(detect, '250', 'change');
    await settle(fixture);
    expect(setLive).toHaveBeenLastCalledWith(true, expect.objectContaining({ detectEveryMs: 250 }));
    typeInto(detect, '99999999', 'change');
    await settle(fixture);
    expect(setLive).toHaveBeenLastCalledWith(true, expect.objectContaining({ detectEveryMs: MAX_DETECT_EVERY_MS }));
    expect(detect.value).toBe(String(MAX_DETECT_EVERY_MS));
    const before = setLive.mock.calls.length;
    typeInto(detect, '-5', 'change');
    await settle(fixture);
    expect(setLive.mock.calls.length).toBe(before);
    expect(detect.value).toBe(String(MAX_DETECT_EVERY_MS));
    expect(all<HTMLInputElement>(fixture, 'input')[0].disabled).toBe(false);
    typeInto(watch, 'events', 'change');
    await settle(fixture);
    expect(all<HTMLInputElement>(fixture, 'input')[0].disabled).toBe(true);
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
    expect(text(fixture)).toContain('No files are open.');
  });
});

