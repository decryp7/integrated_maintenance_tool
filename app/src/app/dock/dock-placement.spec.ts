import { TestBed } from '@angular/core/testing';
import type { DockviewApi, IDockviewPanel } from 'dockview';
import { provideDockTesting } from '../../testing/helpers';
import { ImtView } from '../extensions/tokens';
import { DockPlacement } from './dock-placement';

const view = (minSize?: ImtView['minSize']): ImtView => ({ id: 'v', title: 'V', icon: 'panel', defaultPosition: 'center', ...(minSize ? { minSize } : {}), component: async () => class {} }) as ImtView;

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

describe('DockPlacement.addView: ImtView.minSize', () => {
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
