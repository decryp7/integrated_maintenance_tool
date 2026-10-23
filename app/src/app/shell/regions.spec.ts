import { TestBed } from '@angular/core/testing';
import { FakeDock, provideFakeDock, provideTinyBackend, settle } from '../../testing/helpers';
import { WorkspaceStore } from '../core/workspace.store';
import { focusRegion, RegionNavigator, regionIndex, regionsOf } from './regions';

const all = () => true;

/** A shell like the app's: activity bar, side bar, filter bar, two dock groups and the status bar. */
function shell(): HTMLElement {
  const root = document.createElement('div');
  root.innerHTML = `
    <nav data-debrief-region="activity" tabindex="-1"><button>Explorer</button></nav>
    <aside data-debrief-region="sidebar" tabindex="-1"><button>Open files</button></aside>
    <div data-debrief-region="filters" tabindex="-1"><input aria-label="Search" /></div>
    <div class="dv-groupview" id="g1"><div class="dv-tab" role="tab" tabindex="-1">Events</div><div class="dv-tab dv-active-tab" role="tab" tabindex="0">Timeline</div></div>
    <div class="dv-groupview" id="g2"><div class="dv-tab dv-active-tab" role="tab" tabindex="0">Inspector</div></div>
    <footer data-debrief-region="status" tabindex="-1"><button>Live</button></footer>`;
  document.body.append(root);
  return root;
}

describe('regions (G1)', () => {
  let root: HTMLElement;
  let nav: RegionNavigator;
  let dock: FakeDock;
  beforeEach(() => {
    dock = new FakeDock();
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), ...provideFakeDock(dock)] });
    nav = TestBed.inject(RegionNavigator);
    root = shell();
  });
  afterEach(() => root.remove());

  it('lists the regions in F6 order and finds the one holding the focus', () => {
    const regions = regionsOf(root, all);
    expect(regions.map((r) => r.dataset['debriefRegion'] ?? r.id)).toEqual(['activity', 'sidebar', 'filters', 'g1', 'g2', 'status']);
    expect(regionIndex(regions, root.querySelector('input'))).toBe(2);
    expect(regionIndex(regions, null)).toBe(-1);
    // Off-screen regions (a closed side bar, a hidden group) are skipped.
    expect(regionsOf(root, (el) => el.id !== 'g2')).toHaveLength(5);
  });

  it('F6 / Shift+F6 cycle the regions, wrapping, and a dock group takes focus on its active tab', () => {
    (document.activeElement as HTMLElement | null)?.blur();
    nav.cycle(1, root, all);
    expect(document.activeElement?.getAttribute('data-debrief-region')).toBe('activity');
    nav.cycle(-1, root, all);
    expect(document.activeElement?.getAttribute('data-debrief-region')).toBe('status');
    nav.cycle(1, root, all);
    expect(document.activeElement?.getAttribute('data-debrief-region')).toBe('activity');
    // From inside a region (the search box) to the next one: the first dock group's active tab.
    root.querySelector<HTMLInputElement>('input')!.focus();
    nav.cycle(1, root, all);
    expect(document.activeElement?.textContent).toBe('Timeline');
    nav.cycle(1, root, all);
    expect(document.activeElement?.textContent).toBe('Inspector');
    expect(nav.cycle(1, document.createElement('div'), all)).toBe(false);
  });

  it('Ctrl+Alt+1…9 focus a dock group; other keys are left alone', () => {
    const key = (k: string, init: KeyboardEventInit = {}) =>
      new KeyboardEvent('keydown', { key: k === '1' ? '¡' : k, code: /^[1-9]$/.test(k) ? `Digit${k}` : `Key${k.toUpperCase()}`, ctrlKey: true, altKey: true, cancelable: true, ...init });
    expect(nav.group(2, root, all)).toBe(true);
    expect(document.activeElement?.textContent).toBe('Inspector');
    expect(nav.group(3, root, all)).toBe(false);
    expect(nav.handleKey(key('a'))).toBe(false);
    expect(nav.handleKey(key('1', { shiftKey: true }))).toBe(false);
    expect(nav.handleKey(new KeyboardEvent('keydown', { key: '1', code: 'Digit1' }))).toBe(false);
    expect(nav.handleKey(key('1', { altKey: false }))).toBe(false);
    expect(nav.handleKey(key('1', { metaKey: true }))).toBe(false);
    const e = key('1');
    // In jsdom nothing has a layout, so no group counts as shown: the key is still taken (not the browser's).
    expect(nav.handleKey(e)).toBe(true);
    expect(e.defaultPrevented).toBe(true);
  });

  it('focusRegion falls back to the group itself without tabs', () => {
    const g = document.createElement('div');
    g.className = 'dv-groupview';
    g.tabIndex = -1;
    root.append(g);
    focusRegion(g);
    expect(document.activeElement).toBe(g);
  });

  it('Ctrl+0 opens and focuses the side bar; Ctrl+Shift+G brings the events grid to the front', async () => {
    const store = TestBed.inject(WorkspaceStore);
    store.toggleSidebar();
    expect(store.sidebarOpen()).toBe(false);
    nav.sidebar();
    expect(store.sidebarOpen()).toBe(true);
    nav.events();
    expect(dock.calls).toContain('show:events');
    await settle();
    const ids = nav.commands.map((c) => [c.id, c.keybinding]);
    expect(ids).toEqual([
      ['view.focusNextRegion', 'F6'],
      ['view.focusPreviousRegion', 'Shift+F6'],
      ['view.focusSidebar', 'Mod+0'],
      ['view.focusEvents', 'Mod+Shift+G'],
    ]);
  });
});
