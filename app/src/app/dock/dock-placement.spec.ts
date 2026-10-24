import { TestBed } from '@angular/core/testing';
import type { DockviewApi, IDockviewPanel } from 'dockview';
import { provideDockTesting } from '../../testing/helpers';
import { DebriefView } from '../extensions/tokens';
import { DockLayout } from './dock-layout';
import { DockPlacement } from './dock-placement';

const view = (patch: Partial<DebriefView> = {}): DebriefView => ({ id: 'v', title: 'V', icon: 'panel', defaultPosition: 'center', component: async () => class {}, ...patch }) as DebriefView;

/** A placement whose `place` returns a fake panel, so only the step after the add is under test. */
function setup(panel: IDockviewPanel | null) {
  TestBed.configureTestingModule({ providers: provideDockTesting() });
  const placement = TestBed.inject(DockPlacement);
  const fit = vi.spyOn(TestBed.inject(DockLayout), 'fit').mockReturnValue(null);
  vi.spyOn(placement as unknown as { place: () => IDockviewPanel | null }, 'place').mockReturnValue(panel);
  return { placement, fit };
}

const api = {} as DockviewApi;
const panel = {} as IDockviewPanel;

describe('DockPlacement.addView: the plan decides sizes, tabs and constraints (ADR 0056)', () => {
  it('re-plans after every add, with the new panel marked fresh so its own size is not taken as a wish', () => {
    const { placement, fit } = setup(panel);
    expect(placement.addView(api, view(), false, 'v#2')).toBe(panel);
    expect(fit).toHaveBeenCalledWith(api, { sizes: true, fresh: ['v#2'] });
  });

  it('does not re-plan when the view went to a side bar instead of a panel', () => {
    const { placement, fit } = setup(null);
    expect(placement.addView(api, view(), false)).toBeNull();
    expect(fit).not.toHaveBeenCalled();
  });
});

describe('DockPlacement.addView: fullHeight documents (Home, Registry)', () => {
  const full = (id = 'full'): DebriefView => view({ id, area: 'document', fullHeight: true });

  /** A fake dock holding the timeline; `addPanel` records the position it is given. */
  function fakeDock(groups: unknown[] = []) {
    const addPanel = vi.fn(() => ({ group: { api: { setSize: vi.fn(), setConstraints: vi.fn() } } }));
    const timeline = { group: { id: 'gt', width: 1000, api: { location: { type: 'grid' } } } };
    const dock = { width: 1400, height: 900, groups, panels: [], getPanel: (id: string) => (id === 'timeline' ? timeline : undefined), addPanel } as unknown as DockviewApi;
    TestBed.configureTestingModule({ providers: provideDockTesting() });
    const fit = vi.spyOn(TestBed.inject(DockLayout), 'fit').mockReturnValue(null);
    return { dock, addPanel, fit, placement: TestBed.inject(DockPlacement) };
  }

  it('opens in its own column at the window edge and leaves its width to the plan', () => {
    const { dock, addPanel, fit, placement } = fakeDock();
    placement.addView(dock, full(), false);
    expect(addPanel).toHaveBeenCalledWith(expect.objectContaining({ position: { direction: 'right' } }));
    expect(fit).toHaveBeenCalledWith(dock, { sizes: true, fresh: ['full'] });
  });

  it('does not land in a document group that is only the timeline row\'s strip: it gets its own column', () => {
    const strip = { id: 'g3', height: 300, api: { location: { type: 'grid' } }, panels: [{ id: 'metrics' }] };
    const { dock, addPanel, placement } = fakeDock([strip]);
    vi.spyOn(placement as unknown as { isDocument: (id: string) => boolean }, 'isDocument').mockReturnValue(true);
    placement.addView(dock, full(), false);
    expect(addPanel).toHaveBeenCalledWith(expect.objectContaining({ position: { direction: 'right' } }));
  });

  it('tabs into a document group that already spans the dock', () => {
    const column = { id: 'g7', height: 900, api: { location: { type: 'grid' } }, panels: [{ id: 'home' }] };
    const { dock, addPanel, placement } = fakeDock([column]);
    vi.spyOn(placement as unknown as { isDocument: (id: string) => boolean }, 'isDocument').mockReturnValue(true);
    placement.addView(dock, full('registry'), false);
    expect(addPanel).toHaveBeenCalledWith(expect.objectContaining({ position: { referenceGroup: 'g7', direction: 'within' } }));
  });

  it('is theirs: another document opens beside the timeline, not as a tab of the Home column', () => {
    const column = { id: 'g7', height: 900, api: { location: { type: 'grid' } }, panels: [{ id: 'home' }] };
    const { dock, addPanel, placement } = fakeDock([column]);
    vi.spyOn(placement, 'documentWell').mockReturnValue(column as never);
    const ctx = (placement as unknown as { ctx: { views: { find: (id: string) => DebriefView | undefined } } }).ctx;
    vi.spyOn(ctx.views, 'find').mockImplementation((id: string) => (id === 'home' ? full('home') : undefined));
    placement.addView(dock, view({ id: 'metrics', area: 'document' }), false);
    expect(addPanel).toHaveBeenCalledWith(expect.objectContaining({ position: { referenceGroup: 'gt', direction: 'right' } }));
  });
});
