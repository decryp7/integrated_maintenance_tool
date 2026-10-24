import { TestBed } from '@angular/core/testing';
import type { DockviewApi, IDockviewPanel } from 'dockview';
import { provideDockTesting } from '../../testing/helpers';
import { DebriefView } from '../extensions/tokens';
import { DockPlacement } from './dock-placement';

const view = (minSize?: DebriefView['minSize']): DebriefView => ({ id: 'v', title: 'V', icon: 'panel', defaultPosition: 'center', ...(minSize ? { minSize } : {}), component: async () => class {} }) as DebriefView;

/** A placement whose `place` returns a fake panel, so only the constraint step of `addView` is under test. */
function setup(panel: IDockviewPanel | null) {
  TestBed.configureTestingModule({ providers: provideDockTesting() });
  const placement = TestBed.inject(DockPlacement);
  vi.spyOn(placement as unknown as { place: () => IDockviewPanel | null }, 'place').mockReturnValue(panel);
  return placement;
}

function fakePanel() {
  const setConstraints = vi.fn();
  return { panel: { group: { api: { setConstraints } } } as unknown as IDockviewPanel, setConstraints };
}

const api = {} as DockviewApi;

describe('DockPlacement.addView: DebriefView.minSize', () => {
  it('applies both dimensions of the minimum size as the group constraints', () => {
    const { panel, setConstraints } = fakePanel();
    expect(setup(panel).addView(api, view({ width: 360, height: 240 }), false)).toBe(panel);
    expect(setConstraints).toHaveBeenCalledWith({ minimumWidth: 360, minimumHeight: 240 });
  });

  it('applies only the dimension that is set', () => {
    const a = fakePanel();
    setup(a.panel).addView(api, view({ width: 300 }), false);
    expect(a.setConstraints).toHaveBeenCalledWith({ minimumWidth: 300 });
    TestBed.resetTestingModule();
    const b = fakePanel();
    setup(b.panel).addView(api, view({ height: 120 }), false);
    expect(b.setConstraints).toHaveBeenCalledWith({ minimumHeight: 120 });
  });

  it('ignores a zero dimension and sets nothing without a minimum size', () => {
    const a = fakePanel();
    setup(a.panel).addView(api, view({ width: 0, height: 0 }), false);
    expect(a.setConstraints).toHaveBeenCalledWith({});
    TestBed.resetTestingModule();
    const b = fakePanel();
    setup(b.panel).addView(api, view(), false);
    expect(b.setConstraints).not.toHaveBeenCalled();
  });

  it('returns null and constrains nothing when the view went to a side bar instead of a panel', () => {
    expect(setup(null).addView(api, view({ width: 360 }), false)).toBeNull();
  });
});

describe('DockPlacement.addView: DebriefView.fullHeight (Home, Registry)', () => {
  const full = (id = 'full'): DebriefView => ({ id, title: id, icon: 'panel', area: 'document', fullHeight: true, component: async () => class {} }) as DebriefView;

  /** A fake dock holding the timeline; `addPanel` records the position it is given and returns a panel with a sizeable group. */
  function fakeDock(width: number, groups: unknown[] = [], main?: { id: string; width: number }) {
    const setSize = vi.fn();
    const addPanel = vi.fn(() => ({ group: { api: { setSize, setConstraints: vi.fn() } } }));
    const group = main ? { id: main.id, width: main.width, api: { location: { type: 'grid' }, setConstraints: vi.fn() } } : undefined;
    const getPanel = (id: string) => (id === 'timeline' ? { group } : id === 'events' && group ? { group } : undefined);
    const dock = { width, height: 900, groups, panels: [], getPanel, addPanel } as unknown as DockviewApi;
    TestBed.configureTestingModule({ providers: provideDockTesting() });
    return { dock, addPanel, setSize, placement: TestBed.inject(DockPlacement) };
  }

  it('opens in its own column at the window edge, spanning the dock height, 55% of its width', () => {
    const { dock, addPanel, setSize, placement } = fakeDock(1400);
    placement.addView(dock, full(), false);
    expect(addPanel).toHaveBeenCalledWith(expect.objectContaining({ position: { direction: 'right' } }));
    expect(setSize).toHaveBeenCalledWith({ width: 770 });
  });

  it('caps the column on a wide window (2560 px)', () => {
    const { dock, setSize, placement } = fakeDock(2560);
    placement.addView(dock, full(), false);
    expect(setSize).toHaveBeenCalledWith({ width: 960 });
  });

  it('does not land in a document group that is only the timeline row\'s strip: it gets its own column', () => {
    const strip = { id: 'g3', height: 300, api: { location: { type: 'grid' } }, panels: [{ id: 'metrics' }] };
    const { dock, addPanel, placement } = fakeDock(1400, [strip]);
    vi.spyOn(placement as unknown as { isDocument: (id: string) => boolean }, 'isDocument').mockReturnValue(true);
    placement.addView(dock, full(), false);
    expect(addPanel).toHaveBeenCalledWith(expect.objectContaining({ position: { direction: 'right' } }));
  });

  it('tabs into a document group that already spans the dock', () => {
    const column = { id: 'g7', height: 900, api: { location: { type: 'grid' } }, panels: [{ id: 'home' }] };
    const { dock, addPanel, setSize, placement } = fakeDock(1400, [column]);
    vi.spyOn(placement as unknown as { isDocument: (id: string) => boolean }, 'isDocument').mockReturnValue(true);
    placement.addView(dock, full('registry'), false);
    expect(addPanel).toHaveBeenCalledWith(expect.objectContaining({ position: { referenceGroup: 'g7', direction: 'within' } }));
    expect(setSize).not.toHaveBeenCalled();
  });

  it('shares the events group as a tab when the timeline column cannot spare a readable column ', () => {
    const { dock, addPanel, setSize, placement } = fakeDock(900, [], { id: 'gm', width: 900 });
    placement.addView(dock, full(), false);
    expect(addPanel).toHaveBeenCalledWith(expect.objectContaining({ position: { referenceGroup: 'gm', direction: 'within' } }));
    expect(setSize).not.toHaveBeenCalled();
  });

  it('limits its column to the width the timeline column can spare', () => {
    // The timeline column is 1000 px of a 1600 px dock: 55% (880) would leave it under its 560 px, so it gets 440.
    const { dock, addPanel, setSize, placement } = fakeDock(1600, [], { id: 'gm', width: 1000 });
    placement.addView(dock, full(), false);
    expect(addPanel).toHaveBeenCalledWith(expect.objectContaining({ position: { direction: 'right' } }));
    expect(setSize).toHaveBeenCalledWith({ width: 1000 - 560 });
  });
});

describe('DockPlacement.addView: side tool windows', () => {
  it('never get narrower than the side-tool minimum, whatever their view declares', () => {
    const side = (minSize?: DebriefView['minSize']): DebriefView => ({ ...view(minSize), area: 'side' });
    const a = fakePanel();
    setup(a.panel).addView(api, side(), false);
    expect(a.setConstraints).toHaveBeenCalledWith({ minimumWidth: 320 });
    TestBed.resetTestingModule();
    const b = fakePanel();
    setup(b.panel).addView(api, side({ width: 400 }), false);
    expect(b.setConstraints).toHaveBeenCalledWith({ minimumWidth: 400 });
  });
});

describe('DockPlacement.addView: a side tool in a narrow window (makeRoomForTool)', () => {
  it('tabs a full-height column that cannot shrink into the events group and keeps the events grid in front', () => {
    // Regression (notes.e2e "a note on an event row…"): opening Notes tabbed the Registry column into the events group,
    // which made the Registry the active tab and hid the grid the user was working in.
    TestBed.configureTestingModule({ providers: provideDockTesting() });
    const placement = TestBed.inject(DockPlacement);
    vi.spyOn(placement as unknown as { isDocument: (id: string) => boolean }, 'isDocument').mockReturnValue(true);
    const order: string[] = [];
    const events = { id: 'events', api: { setActive: vi.fn(() => order.push('events active')) } };
    const main = { id: 'gm', width: 700, activePanel: events, api: { location: { type: 'grid' } } };
    const column = { id: 'gr', width: 500, height: 900, panels: [{ id: 'registry' }], api: { location: { type: 'grid' }, setSize: vi.fn(), moveTo: vi.fn(() => order.push('registry moved in')) } };
    const toolGroup = { api: { setSize: vi.fn(), setConstraints: vi.fn() } };
    const dock = {
      width: 1200,
      height: 900,
      panels: [],
      groups: [main, column],
      getPanel: (id: string) => (id === 'events' ? { group: main } : undefined),
      addPanel: vi.fn(() => ({ group: toolGroup })),
    } as unknown as DockviewApi;
    placement.addView(dock, { id: 'notes', title: 'Notes', icon: 'panel', area: 'side', component: async () => class {} } as DebriefView, false);
    expect(column.api.moveTo).toHaveBeenCalledWith({ group: main, position: 'center' });
    expect(column.api.setSize).not.toHaveBeenCalled();
    expect(order).toEqual(['registry moved in', 'events active']);
  });
});
