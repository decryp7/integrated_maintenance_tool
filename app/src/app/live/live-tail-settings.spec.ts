import { TestBed } from '@angular/core/testing';
import { all, el, provideTinyBackend, settle, tinyDataset, typeInto } from '../../testing/helpers';
import { MAX_DETECT_EVERY_MS } from '../core/api.types';
import { MockBackend } from '../core/mock/mock-backend';
import { at } from '../../testing/must';
import { WorkspaceStore } from '../core/workspace.store';
import { LiveTail } from '../workspace/live-tail';
import { LiveTailSettings, parseLimit } from './live-tail-settings';

describe('live tail settings (Settings > Live tail)', () => {
  async function setup() {
    const backend = new MockBackend({ latency: false, dataset: tinyDataset, liveMs: 100_000, memoryStoreFeatures: true });
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
    TestBed.inject(WorkspaceStore);
    await settle();
    return { backend, live: TestBed.inject(LiveTail) };
  }

  it('parses limits', () => {
    expect(parseLimit(' 50 ')).toBe(50);
    expect(parseLimit('')).toBeNull();
    expect(parseLimit('0')).toBeNull();
    expect(parseLimit('-3')).toBeNull();
  });

  it('every control goes to set_live, with units as suffixes', async () => {
    const { backend, live } = await setup();
    const setLive = vi.spyOn(backend, 'setLive');
    const fixture = TestBed.createComponent(LiveTailSettings);
    await settle(fixture);
    await live.set(true);
    await settle(fixture);
    expect(all(fixture, '.unit').map((u) => u.textContent)).toEqual(['ms', 'events', 'minutes', 'ms', 'ms']);
    const selects = all<HTMLSelectElement>(fixture, 'select');
    const watch = at(selects, 0);
    const deleted = at(selects, 1);
    const inputs = all<HTMLInputElement>(fixture, 'input');
    const poll = at(inputs, 0);
    const maxRecords = at(inputs, 1);
    const maxAge = at(inputs, 2);
    const carry = at(inputs, 3);
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
    expect(detect.closest('label')!.textContent).toContain('Run detectors every');
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
    expect(at(all<HTMLInputElement>(fixture, 'input'), 0).disabled).toBe(false);
    typeInto(watch, 'events', 'change');
    await settle(fixture);
    expect(at(all<HTMLInputElement>(fixture, 'input'), 0).disabled).toBe(true);
    await live.set(false);
  });

  it('renders an applied setting at once, so the next render cannot overwrite what the user types next', async () => {
    const { backend, live } = await setup();
    const setLive = vi.spyOn(backend, 'setLive');
    const fixture = TestBed.createComponent(LiveTailSettings);
    await settle(fixture);
    await live.set(true);
    await settle(fixture);
    const detect = el<HTMLInputElement>(fixture, '[data-testid=detect-every]');
    const poll = at(all<HTMLInputElement>(fixture, 'input'), 0);
    for (const [input, first, next, key, expected] of [
      [detect, '250', '99999999', 'detectEveryMs', MAX_DETECT_EVERY_MS],
      [poll, '500', '700', 'pollIntervalMs', 700],
    ] as const) {
      // A change is applied, and before the app renders again (a busy app renders late) the user types the next value.
      typeInto(input, first, 'change');
      input.value = next;
      await settle(fixture);
      expect(input.value).toBe(next);
      input.dispatchEvent(new Event('change', { bubbles: true }));
      await settle(fixture);
      expect(setLive).toHaveBeenLastCalledWith(true, expect.objectContaining({ [key]: expected }));
    }
    await live.set(false);
  });

});
