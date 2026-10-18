import { Type } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';
import { all, buttonByText, el, settle, text } from '../../../testing/helpers';
import { FrameDto, TreePage, TreeRequest } from '../../core/api.types';
import { MockProfilingBackend } from '../../core/mock/mock-profiling';
import { FakeContext } from '../../analysis/test-setup';
import { ProfileViewState } from '../profile-view-state';
import { ProfilingBackend } from '../profiling-backend';
import { openDemoCaptures, setupProfiling } from '../testing/profiling-test-support';
import { CallTreeView, InvertedTreeView } from './call-tree-view';
import { at } from '../../../testing/must';

async function mount<T>(view: Type<T> = CallTreeView as Type<T>, opts: Parameters<typeof setupProfiling>[0] = {}, open = true) {
  const doubles = setupProfiling(opts);
  if (open) await openDemoCaptures();
  const fixture: ComponentFixture<T> = TestBed.createComponent(view);
  fixture.detectChanges();
  await settle(fixture, 6);
  return { fixture, doubles, ctx: doubles.context as FakeContext, state: TestBed.inject(ProfileViewState) };
}

const rows = (f: ComponentFixture<unknown>) => all<HTMLTableRowElement>(f, 'tbody tr');
const nodeRows = (f: ComponentFixture<unknown>) => rows(f).filter((r) => r.hasAttribute('data-key'));
const press = async (f: ComponentFixture<unknown>, key: string) => {
  el(f, 'table').dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }));
  await settle(f, 4);
};
const focused = (f: ComponentFixture<unknown>) => rows(f).findIndex((r) => r.getAttribute('tabindex') === '0');

/** A backend where the top level has 3 more children than the first page, and one child expansion can fail. */
function pagedBackend(failChildOnce = false): { backend: ProfilingBackend; calls: TreeRequest[] } {
  const real = new MockProfilingBackend();
  const backend: ProfilingBackend = Object.create(real);
  const calls: TreeRequest[] = [];
  let failed = !failChildOnce;
  backend.tree = async (r: TreeRequest): Promise<TreePage> => {
    calls.push(r);
    if (r.path?.length && !failed) {
      failed = true;
      throw 'child load failed';
    }
    if (r.path?.length) return real.tree(r);
    const page = await real.tree({ ...r, offset: 0, limit: 200 });
    if (!r.offset) return { ...page, totalChildren: page.nodes.length + 2 };
    const extra: FrameDto = { ...at(page.nodes, 0).frame, id: 9000 + r.offset, name: `Extra.${r.offset}` };
    return { ...page, nodes: [{ ...at(page.nodes, 0), frame: extra, hasChildren: false }], totalChildren: page.nodes.length + 2 };
  };
  return { backend, calls };
}

describe('Call tree view', () => {
  it('shows the no-capture state', async () => {
    const { fixture } = await mount(CallTreeView, {}, false);
    expect(text(fixture)).toContain('Open a profile to see its call tree.');
  });

  it('renders a treegrid with ARIA levels, positions and expansion state', async () => {
    const { fixture } = await mount();
    const grid = el(fixture, 'table');
    expect(grid.getAttribute('role')).toBe('treegrid');
    expect(grid.getAttribute('aria-label')).toBe('Call tree (top-down)');
    const first = at(nodeRows(fixture), 0);
    expect(first.getAttribute('aria-level')).toBe('1');
    expect(first.getAttribute('aria-posinset')).toBe('1');
    expect(Number(first.getAttribute('aria-setsize'))).toBeGreaterThan(0);
    expect(first.getAttribute('aria-expanded')).toBe('false');
    expect(first.getAttribute('aria-selected')).toBe('false');
    expect(all(fixture, 'th[role=columnheader]').length).toBe(5);
    expect(all(fixture, '[data-measurement]').length).toBeGreaterThan(0);
  });

  it('expands and collapses with the twisty and selects on row click', async () => {
    const { fixture } = await mount();
    const first = at(nodeRows(fixture), 0);
    first.querySelector<HTMLElement>('.tw')!.click();
    await settle(fixture, 4);
    expect(nodeRows(fixture)[0]?.getAttribute('aria-expanded')).toBe('true');
    expect(nodeRows(fixture)[1]?.getAttribute('aria-level')).toBe('2');
    expect(nodeRows(fixture)[0]?.getAttribute('aria-selected')).toBe('false');
    at(nodeRows(fixture), 0).querySelector<HTMLElement>('.tw')!.click();
    await settle(fixture, 4);
    expect(nodeRows(fixture)[0]?.getAttribute('aria-expanded')).toBe('false');
    at(nodeRows(fixture), 0).click();
    await settle(fixture, 2);
    expect(nodeRows(fixture)[0]?.getAttribute('aria-selected')).toBe('true');
    expect(focused(fixture)).toBe(0);
  });

  it('double-click opens the butterfly of the node', async () => {
    const { fixture, ctx, state } = await mount();
    at(nodeRows(fixture), 0).dispatchEvent(new MouseEvent('dblclick', { bubbles: true }));
    expect(ctx.last('showView')).toEqual(['profiling-butterfly']);
    expect(state.selectedFrame()).not.toBeNull();
  });

  it('expands the hottest path and collapses everything', async () => {
    const { fixture } = await mount();
    const collapse = buttonByText(fixture, 'Collapse all');
    expect(collapse.disabled).toBe(true);
    buttonByText(fixture, 'Expand hottest path').click();
    await settle(fixture, 8);
    expect(nodeRows(fixture).length).toBeGreaterThan(2);
    expect(nodeRows(fixture).some((r) => r.getAttribute('aria-selected') === 'true')).toBe(true);
    expect(buttonByText(fixture, 'Collapse all').disabled).toBe(false);
    buttonByText(fixture, 'Collapse all').click();
    await settle(fixture, 4);
    expect(nodeRows(fixture).every((r) => r.getAttribute('aria-level') === '1')).toBe(true);
    expect(focused(fixture)).toBe(0);
  });

  it('reports when nothing below a leaf is hot', async () => {
    const { fixture } = await mount();
    buttonByText(fixture, 'Expand hottest path').click();
    await settle(fixture, 8);
    const leaf = nodeRows(fixture).at(-1)!;
    leaf.click();
    await settle(fixture, 2);
    buttonByText(fixture, 'Expand hottest path').click();
    await settle(fixture, 6);
    expect(el(fixture, '[role=status].muted').textContent).toContain('Nothing below this node is hot enough');
  });

  it('navigates with the keyboard', async () => {
    const { fixture, ctx } = await mount();
    expect(focused(fixture)).toBe(0);
    await press(fixture, 'ArrowRight');
    expect(nodeRows(fixture)[0]?.getAttribute('aria-expanded')).toBe('true');
    await press(fixture, 'ArrowRight');
    expect(focused(fixture)).toBe(1);
    await press(fixture, 'ArrowLeft');
    expect(focused(fixture)).toBe(0);
    await press(fixture, 'ArrowLeft');
    expect(nodeRows(fixture)[0]?.getAttribute('aria-expanded')).toBe('false');
    await press(fixture, 'ArrowLeft');
    expect(focused(fixture)).toBe(0);
    await press(fixture, 'ArrowRight');
    await press(fixture, 'ArrowDown');
    expect(focused(fixture)).toBe(1);
    expect(nodeRows(fixture)[1]?.getAttribute('aria-selected')).toBe('true');
    await press(fixture, 'ArrowUp');
    expect(focused(fixture)).toBe(0);
    await press(fixture, 'ArrowUp');
    expect(focused(fixture)).toBe(0);
    await press(fixture, 'End');
    expect(focused(fixture)).toBe(rows(fixture).length - 1);
    await press(fixture, 'Home');
    expect(focused(fixture)).toBe(0);
    await press(fixture, 'Enter');
    expect(ctx.last('showView')).toEqual(['profiling-butterfly']);
    const before = nodeRows(fixture).length;
    await press(fixture, '*');
    await settle(fixture, 6);
    expect(nodeRows(fixture).length).toBeGreaterThan(before);
    const ignored = new KeyboardEvent('keydown', { key: 'q', bubbles: true, cancelable: true });
    el(fixture, 'table').dispatchEvent(ignored);
    expect(ignored.defaultPrevented).toBe(false);
  });

  it('ArrowRight on a leaf does nothing', async () => {
    const { fixture } = await mount();
    buttonByText(fixture, 'Expand hottest path').click();
    await settle(fixture, 8);
    const leafIndex = rows(fixture).findIndex((r) => r.hasAttribute('data-key') && !r.hasAttribute('aria-expanded'));
    if (leafIndex < 0) return;
    at(rows(fixture), leafIndex).click();
    await settle(fixture, 2);
    const e = new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true });
    el(fixture, 'table').dispatchEvent(e);
    expect(e.defaultPrevented).toBe(false);
  });

  it('pages children: Load N more via click and via Enter', async () => {
    const { backend, calls } = pagedBackend();
    const { fixture } = await mount(CallTreeView, { backend });
    const more = () => rows(fixture).find((r) => r.textContent?.includes('more…'))!;
    expect(more().textContent).toContain('Load 2 more…');
    more().click();
    await settle(fixture, 6);
    expect(calls.some((c) => c.offset)).toBe(true);
    expect(text(fixture)).toContain('Extra.');
    expect(more().textContent).toContain('Load 1 more…');
    await press(fixture, 'End');
    expect(focused(fixture)).toBe(rows(fixture).indexOf(more()));
    await press(fixture, '*');
    const n = calls.length;
    await press(fixture, 'Enter');
    await settle(fixture, 6);
    expect(calls.length).toBe(n + 1);
    expect(rows(fixture).some((r) => r.textContent?.includes('more…'))).toBe(false);
  });

  it('shows a loading row, then an error row with Retry when a child page fails', async () => {
    const { backend } = pagedBackend(true);
    const { fixture } = await mount(CallTreeView, { backend });
    at(nodeRows(fixture), 0).querySelector<HTMLElement>('.tw')!.click();
    await settle(fixture, 6);
    expect(el(fixture, 'tbody [role=alert]').textContent).toContain('child load failed');
    buttonByText(fixture, 'Retry').click();
    await settle(fixture, 6);
    expect(all(fixture, 'tbody [role=alert]')).toHaveLength(0);
    expect(nodeRows(fixture)[1]?.getAttribute('aria-level')).toBe('2');
  });

  it('shows a loading status row while children load', async () => {
    const { fixture } = await mount(CallTreeView, { options: { latencyMs: 30 } });
    await new Promise((r) => setTimeout(r, 120));
    await settle(fixture, 4);
    at(nodeRows(fixture), 0).querySelector<HTMLElement>('.tw')!.click();
    await settle(fixture, 1);
    fixture.detectChanges();
    expect(rows(fixture).some((r) => r.textContent?.includes('Loading…'))).toBe(true);
    await new Promise((r) => setTimeout(r, 120));
    await settle(fixture, 4);
    expect(rows(fixture).some((r) => r.textContent?.includes('Loading…'))).toBe(false);
  });

  it('shows an error for the root with Retry', async () => {
    const { fixture } = await mount(CallTreeView, { options: { fail: { tree: 1 } } });
    expect(el(fixture, '[role=alert]').textContent).toContain('tree failed');
    buttonByText(fixture, 'Retry').click();
    await settle(fixture, 6);
    expect(nodeRows(fixture).length).toBeGreaterThan(0);
  });

  it('reveals a path from evidence and selects the node', async () => {
    const { fixture, doubles, state } = await mount();
    const pid = state.profileId()!;
    const hot = await doubles.backend.hotPath({ profileId: pid, view: state.view(), metric: 'timeNs' });
    expect(hot.path.length).toBeGreaterThan(1);
    const frame = hot.nodes.at(-1)!.frame;
    state.applyEvidence({ view: 'callTree', profileId: pid, baselineId: null, frame, path: hot.path, metric: 'timeNs', range: null, threads: null, eventKind: null });
    await settle(fixture, 10);
    const sel = nodeRows(fixture).find((r) => r.getAttribute('aria-selected') === 'true')!;
    expect(sel.getAttribute('aria-level')).toBe(String(hot.path.length));
    expect(sel.textContent).toContain(frame.name);
    expect(focused(fixture)).toBe(rows(fixture).indexOf(sel));
  });

  it('shows a notice when the revealed path is not in the tree', async () => {
    const { fixture, state } = await mount();
    state.revealPath([987654]);
    await settle(fixture, 8);
    expect(el(fixture, '[role=status].muted').textContent).toContain('That path is not in the tree');
  });

  it('applies a bottom-up reveal to the inverted tree only and expands its hot path', async () => {
    const top = await mount(CallTreeView);
    const { state } = top;
    const first = at(nodeRows(top.fixture), 0);
    const topLevel = nodeRows(top.fixture).length;
    const inv = TestBed.createComponent(InvertedTreeView);
    inv.detectChanges();
    await settle(inv, 6);
    expect(el(inv, 'table').getAttribute('aria-label')).toBe('Inverted tree (bottom-up)');
    const hot = await top.doubles.backend.hotPath({ profileId: state.profileId()!, view: state.view(), metric: 'timeNs', direction: 'bottomUp' });
    state.revealPath(hot.path.slice(0, 1), 'bottomUp');
    await settle(inv, 12);
    await settle(top.fixture, 4);
    expect(nodeRows(inv).some((r) => r.getAttribute('aria-selected') === 'true')).toBe(true);
    expect(nodeRows(inv).length).toBeGreaterThan(1);
    expect(nodeRows(top.fixture)).toHaveLength(topLevel);
    expect(first.getAttribute('aria-selected')).toBe('false');
  });

  it('re-queries with the view state (range) and resets focus', async () => {
    const { fixture, doubles, state } = await mount();
    const spy = vi.spyOn(doubles.backend, 'tree');
    state.setRange({ fromNs: 0, toNs: 5_000_000 });
    await settle(fixture, 6);
    expect(spy.mock.calls[0]?.[0].view?.range).toEqual({ fromNs: 0, toNs: 5_000_000 });
  });

  it('shows the empty state when no sample is in the view', async () => {
    const { fixture, state } = await mount();
    state.setThreads([99999]);
    await settle(fixture, 6);
    expect(text(fixture)).toContain('No samples match this view.');
  });
});
