import { TestBed } from '@angular/core/testing';
import { FakeDock, provideFakeDock, provideTinyBackend, settle } from '../../testing/helpers';
import { LiveTail } from '../workspace/live-tail';
import { DismissedStore, LIVE_DISMISSED_STORE, LiveAutoOpen, localDismissedStore } from './live-auto-open';

/** A dock whose show/close change what is open. */
class OpeningDock extends FakeDock {
  override show = (id: string): undefined => {
    this.calls.push(`show:${id}`);
    this.openIds.set(new Set([...this.openIds(), id]));
    return undefined;
  };
  override close = (id: string): undefined => {
    const next = new Set(this.openIds());
    next.delete(id);
    this.openIds.set(next);
    return undefined;
  };
}

function setup(saved: string[] = []) {
  const dock = new OpeningDock();
  let stored = [...saved];
  const storage: DismissedStore = { load: () => stored, save: (ids) => void (stored = ids) };
  TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideFakeDock(dock), { provide: LIVE_DISMISSED_STORE, useValue: storage }] });
  TestBed.inject(LiveAutoOpen);
  const live = TestBed.inject(LiveTail);
  return { dock, live, stored: () => stored };
}

describe('LiveAutoOpen', () => {
  it('opens Tail status and Alerts when live tail starts', async () => {
    const { dock, live } = setup();
    await settle();
    expect(dock.calls).toEqual([]);
    live.enabled.set(true);
    await settle();
    expect(dock.calls).toEqual(['show:tail-status', 'show:live-alerts']);
  });

  it('remembers a view the user closed while live tail ran and does not open it again; opening it by hand forgets that', async () => {
    const { dock, live, stored } = setup();
    live.enabled.set(true);
    await settle();
    dock.close('live-alerts');
    await settle();
    expect(stored()).toEqual(['live-alerts']);
    live.enabled.set(false);
    await settle();
    dock.close('tail-status');
    await settle();
    // Closed after live tail stopped: not a dismissal.
    expect(stored()).toEqual(['live-alerts']);
    dock.calls.length = 0;
    live.enabled.set(true);
    await settle();
    expect(dock.calls).toEqual(['show:tail-status']);
    dock.show('live-alerts');
    await settle();
    expect(stored()).toEqual([]);
  });

  it('honours dismissals saved earlier', async () => {
    const { dock, live } = setup(['tail-status', 'live-alerts']);
    live.enabled.set(true);
    await settle();
    expect(dock.calls).toEqual([]);
  });

  it('stores dismissals in browser storage, tolerating missing, corrupt and failing storage', () => {
    localStorage.clear();
    expect(localDismissedStore.load()).toEqual([]);
    localDismissedStore.save(['tail-status']);
    expect(localDismissedStore.load()).toEqual(['tail-status']);
    localStorage.setItem('debrief.live.autoOpen.dismissed.v1', '{not json');
    expect(localDismissedStore.load()).toEqual([]);
    localStorage.setItem('debrief.live.autoOpen.dismissed.v1', '{"a":1}');
    expect(localDismissedStore.load()).toEqual([]);
    localStorage.setItem('debrief.live.autoOpen.dismissed.v1', '["x", 3]');
    expect(localDismissedStore.load()).toEqual(['x']);
    const set = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('quota');
    });
    expect(() => localDismissedStore.save(['y'])).not.toThrow();
    set.mockRestore();
    localStorage.clear();
  });
});
