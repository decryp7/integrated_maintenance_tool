import { signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { provideTinyBackend, settle } from '../../testing/helpers';
import { RecordView } from '../core/api.types';
import { ViewRequests } from '../core/view-requests';
import { SelectionStore } from '../core/workspace/selection.store';
import { DockPolicy, DockPolicyTarget } from './dock-policy';

function target() {
  const calls: string[] = [];
  const ready = signal(false);
  const dock: DockPolicyTarget = {
    ready,
    show: (id) => calls.push(`show:${id}`),
    revealInspector: () => calls.push('reveal'),
    syncConditionalViews: () => calls.push('sync'),
  };
  return { calls, ready, dock };
}

describe('DockPolicy', () => {
  function setup() {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const t = target();
    TestBed.inject(DockPolicy).start(t.dock);
    return { ...t, selection: TestBed.inject(SelectionStore), requests: TestBed.inject(ViewRequests) };
  }

  it('does nothing until the dock has a layout, then syncs conditional views with the workspace', async () => {
    const { calls, ready } = setup();
    await settle();
    expect(calls).toEqual([]);
    ready.set(true);
    await settle();
    expect(calls).toEqual(['sync']);
  });

  it('shows the events for a jump, reveals the inspector for a new selection only, and shows requested views', async () => {
    const { calls, ready, selection, requests } = setup();
    ready.set(true);
    await settle();
    calls.length = 0;
    selection.jumpTo(3);
    await settle();
    expect(calls).toEqual(['show:events']);
    calls.length = 0;
    selection.select({ id: 4 } as RecordView);
    await settle();
    selection.select({ id: 4 } as RecordView);
    await settle();
    expect(calls).toEqual(['reveal']);
    calls.length = 0;
    requests.show('events');
    await settle();
    requests.show('events');
    await settle();
    expect(calls).toEqual(['show:events', 'show:events']);
  });

  it('drops view requests made before the dock has a layout', async () => {
    const { calls, ready, requests } = setup();
    requests.show('metrics');
    await settle();
    ready.set(true);
    await settle();
    expect(calls.filter((c) => c.startsWith('show:'))).toEqual([]);
  });
});
